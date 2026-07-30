# Syncthing 冷启动优化设计

## 1. 目标

消除 Syncthing 文件安全集成对 Textora 首帧可见时间的显著回退，同时保留以下能力：

- 外部修改、删除和原子替换能够触发既有 `FileSafetyWorker` 校验。
- Syncthing 配置、Keychain 和 REST 探测继续在后台执行。
- 首帧前不递归遍历打开文档的父目录，不读取目录内全部文件内容。
- 后台服务启动失败不能阻止编辑器窗口显示和正常编辑。

本轮不以替换 `reqwest`、缩减安装包或增加 Cargo feature 为目标。二进制体积优化与首帧关键路径解耦，后续单独评估。

## 2. 已确认根因

当前 `LibraryFileMonitor` 使用 `notify::PollWatcher`，配置为 50 ms 周期和 `compare_contents(true)`。监控根目录时，`PollWatcher` 会递归遍历目录并读取每个普通文件的完整内容计算哈希；后续每个周期重复扫描。

`App::refresh_file_monitor_roots()` 通过同步 `recv()` 等待根目录替换完成，而 workspace 恢复流程在首帧前调用它。因此首帧耗时随已打开文档父目录的文件数量和总字节数增长。Syncthing controller 的磁盘、Keychain 和 HTTP 工作已经位于后台线程，不是主要的同步阻塞点，但其线程创建仍发生在事件循环启动前。

## 3. 方案比较

### 方案 A：原生事件后端 + 首帧后启动（采用）

使用 `notify::RecommendedWatcher`。在 macOS 上它映射到 FSEvents，只投递发生变化的路径；具体内容一致性继续交给现有 revision/BLAKE3 文件安全校验。首帧提交后通过 `AppEvent` 请求初始化文件监控和 Syncthing controller，主线程不在首次 `present()` 前启动这些服务。

优点：同时消除首帧递归扫描和运行期 50 ms 全量读取，保持现有安全校验边界。缺点：后台服务在首帧后存在一个很短的未就绪窗口，需要就绪后的 revision reconciliation 补偿。

### 方案 B：保留轮询但降低频率

把轮询周期提高到 1–2 秒并关闭内容比较。

优点：改动最小。缺点：仍需递归扫描元数据，目录越大成本越高；也会引入最高一个轮询周期的事件延迟，只适合作为原生事件后端不可用时的降级策略。

### 方案 C：仅把现有监控移到首帧后

保留 50 ms 全量内容轮询，只延迟初始化。

优点：表面上的首帧指标会改善。缺点：启动后仍持续大量读取，窗口显示后可能立即卡顿，并消耗 CPU、磁盘和电池，因此不采用。

## 4. 架构与数据流

### 4.1 文件监控后端

`LibraryFileMonitor` 保持现有公共职责：接收根目录集合、汇总 `notify::Event` 路径、去抖后唤醒应用。内部 watcher 类型改为 `RecommendedWatcher`，不再设置轮询周期或内容比较。

事件数据流保持为：

```text
FSEvents / RecommendedWatcher
  -> LibraryFileMonitor 路径批次
  -> AppEvent::FileSafetyResultsReady
  -> FileSafetyWorker revision/BLAKE3 校验
  -> 文档刷新、冲突副本或安全通知
```

原生 watcher 只负责发现候选路径，不承担内容真实性判断。这样不会把 UI 或 app 状态泄漏到监控后端，也不改变现有文件安全语义。

### 4.2 首帧后后台服务事件

新增语义化 `AppEvent::StartBackgroundServices`。第一次 GPU `present()` 完成并恢复窗口透明度后，renderer 只向 event loop 投递一次该事件，不直接创建线程或访问文件系统。

应用收到事件后执行幂等的后台服务初始化：

1. 创建 `LibraryFileMonitor` 并提交当前监控根目录。
2. 创建 `SyncController`；配置文件、Keychain 和 Syncthing REST 探测仍由 controller worker 执行。
3. 服务已存在时直接返回，避免重复创建线程。

`FileSafetyWorker` 保持在 `set_event_loop_proxy()` 中提前创建。它不递归扫描目录，而且保存与外部变更校验可能在后台服务事件到达前被其他 app 路径使用。

### 4.3 就绪窗口与一致性

renderer 发送事件之后、monitor 完成根目录注册之前，外部文件可能发生变化。监控启动完成后，应用立即对当前打开的磁盘文档提交一次既有 revision 检查。该检查只读取受管文档，不扫描整个目录，用来弥补初始化窗口。

若 watcher 或 controller 创建失败：

- 记录不包含密钥的错误。
- 编辑、打开和保存继续可用。
- 对应服务保持 `None`，后续显式操作可以按现有错误路径报告不可用。

## 5. 任务拆分与文件边界

### 子任务一：事件驱动文件监控

- 修改 `crates/app/src/library_file_monitor.rs`。
- 先增加会拒绝 `PollWatcher`、50 ms 轮询和内容比较配置的回归测试，确认测试在当前实现上失败。
- 改用 `RecommendedWatcher`，运行监控定向测试和 app 编译。

### 子任务二：后台服务启动事件协议

- 修改 `crates/app/src/app_event.rs`、`crates/app/src/app.rs`、`crates/app/src/app_lifecycle.rs`。
- 定义 `StartBackgroundServices`，增加幂等初始化入口并在用户事件处理中调用。
- 先增加缺失事件/入口的失败测试，再完成最小实现。

### 子任务三：移出首帧关键路径

- 修改 `crates/app/src/app.rs`、`crates/app/src/app_renderer.rs`。
- 先增加源码顺序回归测试：后台服务事件必须位于第一次 `present()` 之后，`set_event_loop_proxy()` 不得创建 monitor 或 controller。
- renderer 只发送一次事件；移除事件循环启动前的 monitor/controller 创建。

每个子任务修改不超过 3 个文件，并在进入下一子任务前通过编译。

## 6. 验证标准

- `LibraryFileMonitor` 生产代码不包含 `PollWatcher`、`with_poll_interval` 或 `with_compare_contents`。
- workspace 恢复期间不创建 watcher，不同步等待递归目录注册。
- 第一次 `present()` 之前不创建 `LibraryFileMonitor` 或 `SyncController`。
- 文件创建、修改、删除和原子替换相关现有测试继续通过。
- Syncthing 未配置、已配置、鉴权失败和离线状态仍由后台结果驱动，不阻塞窗口显示。
- 定向测试、`cargo check -p textora-app` 和 `cargo fmt --check` 通过。
- 本次跨越监控、事件和启动生命周期，最终运行 `./scripts/verify.sh` 全面验证。

## 7. 非目标与后续项

- 不实现自定义 HTTP 协议客户端。
- 不改变 Syncthing API 版本范围或安全密钥存储。
- 不把同步功能改成默认关闭的 Cargo feature。
- 不在本轮重新设计全部文件监控根目录策略；普通文档与资料库根目录的进一步细分作为后续独立优化。
- 首帧性能用现有 `[startup] first_frame_visible total` 日志进行 A/B 观测；若仍有回退，再增加分阶段 timing，而不是预先引入新的性能框架。
