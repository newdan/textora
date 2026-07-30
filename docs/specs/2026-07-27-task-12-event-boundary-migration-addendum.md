# Task 12 事件边界迁移补充设计

## 背景

原实施计划要求在 Task 12A 立即将 `AppEvent` 替换为仅包含四个无载荷变体的
`ShellEvent`，但当前生产代码仍通过 `AppEvent` 传递三类产品数据：

- recent files 路径列表；
- sync 后台完成通知；
- macOS open-document 路径列表。

其中 sync 与 recent files 原计划到 Task 12C 才迁移，macOS open-document
则没有对应迁移步骤。因此，原顺序无法同时满足以下约束：

- 每个阶段编译通过；
- 每个实现子任务最多修改 3 个文件；
- `appkit-shell` 不解析产品 payload；
- `ShellEvent` 保持设计规定的四个变体。

本补充设计只修正迁移顺序和缺失的 macOS open-document 数据流，不改变最终架构。

## 最终边界

`appkit-shell` 只公开：

```rust
pub enum ShellEvent {
    StartBackgroundServices,
    ReshapeResultsReady,
    FileSafetyResultsReady,
    ProductWake,
}
```

`ShellEffect` 继续使用现有布尔并集语义，表达 reshape、窗口 chrome、标题、
设置持久化、workspace 持久化和 redraw。它不包含 sync、recent files、
文件路径或其他产品数据。

`TextoraProduct` 持有产品服务和产品自有 inbox。后台生产者遵循固定协议：

1. 把 typed payload 写入 `crates/app` 内部的 channel；
2. 通过 `ProductWakeHandle` 发送一个 `ShellEvent::ProductWake`；
3. 不把 payload 放入 winit user event。

`ProductWakeHandle` 只包装 `EventLoopProxy<ShellEvent>`，只公开 `wake()`。

## 产品事件分类

产品数据不跨入 `appkit-shell`，但按消费方分成两类。

### 产品内部结果

sync 完成和 recent files 加载结果由 `TextoraProduct` 自身消费：

- sync 完成时 drain `SyncController` 并请求 redraw；
- recent files 完成时更新 textora 的 `NativeMenu`。

这些结果由 `ProductHost::drain_product_events()` 消费并汇总为 `ShellEffect`。

### 产品发起的 shell 命令

macOS open-document 是 textora 平台集成产生的产品命令，但最终需要调用通用的
打开文件能力。路径保存在 `TextoraProduct` 的独立 open-document inbox 中。

本地 `App` 作为产品组合层，在收到 `ProductWake` 后：

1. 临时借用 `TextoraProduct`，取出待打开路径；
2. 调用当前 App/shell 的 typed open-file API；
3. 合并打开文件产生的 `ShellEffect`；
4. 再调用 `ProductHost::drain_product_events()` 处理产品内部结果。

因此，`appkit-shell` 只观察到 `ProductWake`，不会看到 `PathBuf`；同时不需要
`Any`、字符串 action 名、全局回调表或把产品命令塞进 `ShellEffect`。

Objective-C application delegate 仍需要进程级注册状态。该状态只保存 typed
channel sender 和 `ProductWakeHandle`，不保存回调函数或 App/shell 引用。

## 可编译迁移顺序

迁移按以下顺序拆分；每个子任务最多修改 3 个文件，且提交前相关 crate 必须编译。

### 12A-1：建立 shell 事件与 effect

- 在 `appkit-shell` 定义最终 `ShellEvent` 和 `ShellEffect`；
- 移动 effect union-law 与固定执行顺序测试；
- `app_effect.rs` 临时 re-export `ShellEffect`；
- 暂不替换仍承载产品 payload 的本地 `AppEvent`。

此时最终 shell 类型已经存在，但旧 event loop 继续编译。

### 12B：建立产品端口和产品容器

- 定义 `ProductWakeHandle`、`WakeError` 和 `ProductHost`；
- 创建 `TextoraProduct`，持有产品内部结果 inbox、open-document inbox 以及迁移中的
  产品服务；
- 用 fake host 证明 shell 侧只观察 `ProductWake`；
- 此阶段不改变现有后台生产者。

### 12C-1：迁移 sync 完成通知

- sync 后台结果先进入产品 controller/channel；
- 唤醒信号改为无 payload 的产品 wake；
- 回归测试证明一次完成只发送一次 wake，drain 后请求 redraw。

### 12C-2：迁移 recent files

- recent loader 把路径写入产品 inbox；
- native menu 在产品层 drain 时更新；
- winit event 不再携带 recent paths。

### 12C-3：迁移 macOS open-document

- application delegate 把路径写入 open-document inbox；
- 只发送产品 wake；
- 本地 App reducer 取出路径并调用 typed open-file API；
- 测试覆盖多个路径、无效路径继续处理和一次 wake。

### 12C-4：切换最终事件类型

- 删除 `AppEvent` 中全部产品变体；
- 将 `app_event.rs` 收敛为 `ShellEvent` 的临时 re-export；
- 后台服务统一使用 `ProductWakeHandle`；
- 验证 `ApplicationHandler` 和 event loop 只接收 `ShellEvent`。

完成 12C-4 后，`AppEvent` 只作为迁移期名称存在，不再拥有独立 enum。

## 错误处理

- channel receiver 已关闭：生产者记录稳定、无敏感数据的错误并结束本次投递；
- event loop 已关闭：`ProductWakeHandle::wake()` 返回 `WakeError`，调用方不得 panic；
- recent files 单个路径无效：沿用现有过滤行为；
- macOS URL 无路径或不是 file URL：忽略该项并继续处理剩余 URL；
- 打开单个文件失败：记录该路径对应错误并继续处理后续路径；
- sync drain 错误：沿用 controller 的 typed 状态映射，不把密钥或产品 payload
  写入日志或 shell event。

## 测试与完成门槛

每个行为变化必须先有失败测试，再做最小实现。Task 12 至少覆盖：

- `ShellEffect` 的单位元、幂等、交换、结合及固定执行顺序；
- `ShellEvent` 不含产品 payload；
- fake `ProductHost` 只能触发 `ProductWake`；
- sync 完成写入产品状态、只 wake 一次、drain 后 redraw；
- recent files payload 只存在于产品 inbox；
- macOS open-document payload 只存在于产品 inbox，并保留逐项继续处理语义；
- `AppEvent` 最终只是 `ShellEvent` re-export；
- `cargo test -p textora-appkit-shell event`；
- `cargo test -p textora-appkit-shell product_host`；
- `cargo test -p textora-app --lib app_lifecycle`；
- `cargo test -p textora-app --lib sync_controller`；
- `cargo check -p textora-app`；
- `cargo fmt --all -- --check`。

## 非目标

- 不提前实现 Task 16 的最终 `ShellRuntime`；
- 不把 widget 组合或 textora action reducer 移入 `appkit-shell`；
- 不设计第二个产品、泛型产品 action、动态插件产品协议或全局回调表；
- 不改变现有设置、workspace、history、pinned paths 或 dirty snapshot 格式。
