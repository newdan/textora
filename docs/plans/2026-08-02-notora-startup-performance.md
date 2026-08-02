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
