# notora 启动性能分析与优化记录

> 日期：2026-08-02
> 平台：macOS 26.3.1 / Darwin arm64
> 工具链：Rust 1.93.0，release profile

## 目标与边界

本轮优化关注从 `NotoraApp` 开始构造到首个成功 `present` 的关键路径。会话恢复、工作区打开、catalog 扫描和文档恢复原本已经位于首帧之后，本轮保持该边界不变。

## 测量方法

启动插桩由 `NOTORA_TRACE_STARTUP=1` 开启，默认不产生输出：

```bash
NOTORA_TRACE_STARTUP=1 ./target/release/notora
```

warm GUI 数据使用相同 release 二进制连续采样 10 次，每次观察到 `first_frame_visible` 后结束进程。before/after 使用相同代码布局和计时边界；before 仅关闭 GPU 与字体后台准备，after 开启两项优化。

字体缓存使用独立 Criterion 基准，不读取、移动或删除用户配置：

```bash
cargo bench -p textora-shaping --bench font_cache_bench -- --noplot
```

## 根因

首帧之前的产品状态构造仅约 0.4ms，不是瓶颈。主要可优化串行阶段是：

1. winit 事件循环创建和进入 `resumed`；
2. 字体数据库 cache miss/hit；
3. 窗口创建、GPU adapter/device 请求、surface 配置和文本 GPU 资源创建；
4. 首帧 shell 绘制与 present。

其中 GPU adapter/device 与字体数据库都不依赖 native window，却原本在 `resumed` 内串行等待。

独立字体基准结果：

| 路径 | 耗时 |
|---|---:|
| 字体缓存 miss（单次诊断） | 33.42ms |
| 字体缓存 hit（Criterion 中位估计） | 2.81ms |

## 实施方案

### GPU 提前准备

- `EditorRuntime` 显式启动 surface-independent GPU 准备线程；
- 后台完成 `Instance`、adapter、device 和 queue 请求；
- window 创建后再创建 surface、验证 adapter 兼容性并配置 surface；
- worker 失败、panic、adapter 不支持 surface 或 surface format 不可用时，回退到原同步 surface-aware 初始化路径。

### 字体提前准备

- `NotoraApp::try_new` 启动字体数据库准备线程；
- 用 `FontSystemPreparation::{Deferred, InProgress}` 表示互斥状态；
- `resumed` 获取准备结果并构造共享 `FontSystem`；
- worker 无法启动或异常退出时，回退到原同步缓存加载路径；
- 测试使用的 `NotoraApp::with_paths` 不启动后台准备，保持无窗口测试轻量且可控。

### 启动顺序

正式入口先构造 `NotoraApp`，启动 GPU 与字体准备，再创建 winit 事件循环。这样两项准备可以覆盖事件循环创建和进入 `resumed` 的等待时间。

## 结果

10 次 warm GUI 采样中位数：

| 阶段 | before | after | 变化 |
|---|---:|---:|---:|
| `font_system_ready` 等待 | 3.31ms | 0.01ms | -99.7% |
| `window_gpu_text_ready` | 24.13ms | 18.14ms | -24.8% |
| 首帧累计耗时 | 80.27ms | 71.20ms | **-11.3%** |

首帧累计耗时降低 9.07ms。before 范围为 78.23–104.04ms，after 范围为 68.93–90.03ms。字体 cache miss 为 33.42ms，小于当前事件循环阶段，因此后台字体准备预计可把首次字体扫描的大部分耗时移出关键路径；这是由独立基准和阶段时序推导，未通过移动用户字体缓存进行破坏性 GUI 实测。

## 后续方向

当前 warm 路径的剩余主要成本是 winit/native window 固定初始化，以及约 18ms 的 window/surface/text 资源和约 6ms 的首帧绘制。若继续追求低于 60ms 的首帧，需要把“窗口背景帧”和“完整文本资源帧”拆成显式启动状态机；这会改变可感知启动行为，应作为独立架构阶段设计和验证。

## 2026-08-20 第二阶段：从首帧扩展到可用性

### 新基线

在当前 release 二进制上重新采样。构建后第一次启动的
`first_frame_visible` 为 172.13ms；随后三个独立进程样本为 74.16ms、77.93ms 和
76.72ms，中位数 76.72ms。随后样本的中位阶段分布为：

| 阶段 | 耗时 |
|---|---:|
| 应用构造 | 0.29ms |
| winit/macOS 激活并覆盖后台字体、GPU 准备 | 51.71ms |
| window、surface 与文本 GPU 资源 | 19.61ms |
| 首帧布局、上传与 present | 5.01ms |

字体 cache hit 为 2.75ms，cache miss 为 25.03ms；两者都已被现有并行路径覆盖，
不再作为第二阶段优化重点。

现有 `first_frame_visible` 只证明空壳帧已经提交。首帧返回后，
`restore_session_after_first_frame` 会在 UI 线程同步等待文件监视器注册、catalog
打开/恢复和索引 worker 启动。因此第二阶段把验收目标扩展为：

1. `first_frame_visible` 不回退；
2. 首帧后 UI 线程不等待工作区 I/O 或后台线程握手；
3. 输出 `workspace_session_ready`、`session_restore_finished` 和
   `last_document_rendered` 等可用性里程碑；
4. 大资料库首次入库使用批量事务，避免逐笔记 autocommit 和标签 N+1 查询。

### 分阶段实施

#### 阶段 A：可用性插桩

- 保留 `StartupTrace` 的单调时钟所有权；
- 在 session 恢复开始、工作区 session 就绪、恢复请求全部派发和最后文档首次绘制时
  记录一次性里程碑；
- 默认关闭日志，不改变生产启动行为。

#### 阶段 B：异步工作区 bootstrap

- `WorkspaceController` 新增异步 prepare/install 协议；prepare worker 独占
  `WorkspaceSession` 构造所需的 watcher、catalog 与 indexer 初始化；
- 完成消息携带 generation，UI 线程只安装仍属于当前恢复请求的结果；
- session 恢复期间展示加载状态，取消或新请求使旧完成结果失效；
- 工作区启动失败不得替换现有活动 session，也不得阻塞事件循环。

每个实现子阶段最多修改三个生产文件。先增加能证明 UI 入口立即返回、过期完成结果被
丢弃的测试，再迁移正式恢复路径。

#### 阶段 C：批量 catalog reconciliation

- 将扫描产生的 note upsert、presence reconciliation 和搜索索引提交收敛到显式批次；
- 一个扫描批次只开启并提交一次外层 SQLite transaction；
- 一次性读取相关笔记标签，禁止每个变更笔记单独查询；
- 新增“空 catalog 首次导入 10k 笔记”基准，现有
  `workspace_scan_10k` 继续覆盖稳定态。

当前稳定态 10k 全量扫描为 57.44ms。现有 benchmark 的 fixture 准备（创建 10k 文件并
首次建库）约 27 秒；该值包含文件创建，不能直接作为首次扫描耗时，但足以证明需要独立
首次导入基准和批量事务改造。

#### 阶段 D：基于数据的剩余优化

- 细分 `window_gpu_text_ready` 为 window、prepared GPU join、surface、pipeline、atlas
  和 shaper；只有确认冷样本主要消耗后，才评估 pipeline cache 或延迟 atlas；
- catalog 正常打开路径评估 `PRAGMA integrity_check` 的大库成本，再决定是否使用
  `quick_check`、非正常退出标记或后台完整检查；
- 不以降低 MSAA、移除恢复校验或牺牲元数据安全换取未经验证的启动数字。

### 最终验证

1. 定向单元和集成测试；
2. `cargo fmt --check` 与相关 crate 编译；
3. release GUI 至少五次 A/B，分别报告首帧与工作区可用中位数；
4. 10k 稳定态扫描及首次导入基准；
5. 跨模块阶段完成后执行 `./scripts/verify.sh`。

### 第二阶段实施结果

会话工作区恢复现由后台 worker 完成 watcher、catalog 和 indexer 的准备，主线程只安装
仍匹配当前 generation 的 session。五次 release GUI 样本如下：

| 里程碑 | 五次样本中位数 |
|---|---:|
| `first_frame_visible` | 88.22ms |
| `workspace_session_ready` | 91.99ms |
| `session_restore_finished` | 92.01ms |
| `restored_document_rendered` | 124.64ms |
| 首帧后的 workspace bootstrap 阶段 | 3.63ms |

本轮首帧中位数高于实施前不同时间采集的三次样本 76.72ms，但代码中的异步工作区恢复在
首帧提交之后才启动；本轮 `window_gpu_text_ready` 阶段中位数 20.44ms，也与此前
19.61ms 接近。增量主要出现在 winit/macOS 激活等待，不能归因为本轮改动，也不能把两组
非受控样本当作严格 A/B。新增里程碑使后续可以独立跟踪首帧、工作区可用和恢复文档可见。

首次扫描现在一次读取活动笔记标签，并在一个 SQLite transaction 中提交 note upsert、
缺失确认和 FTS 更新。空 catalog 首次导入 10k 笔记的新基准中位估计为 5.49s；稳定态
10k 全量扫描为 56.15ms，相比此前 57.44ms 无回退。首次导入原先只有约 27s 的复合
fixture 数据，混入了文件创建和其他 benchmark fixture 准备，不能作为严格 before；因此
保留新的独立基准作为后续回归基线，不虚构百分比收益。

10k catalog 的完整 `Catalog::open` 为 40.41ms，其中 `PRAGMA integrity_check` 为
36.47ms，`quick_check` 为 28.10ms。较弱检查只节省约 8.4ms，而 catalog 打开已经移出
首帧/UI 线程，因此保留完整检查。`window_gpu_text_ready` 也没有显示新的显著热点，本阶段
不引入 pipeline cache、延迟 atlas 或降低校验强度等风险更高的改动。
