# Textora / Notora 共享事件机制重构方案

## 目标

Textora 与 Notora 使用同一套无业务依赖的事件运行时原语，同时保留各自类型化的
Action、后台完成事件和解释器：

- UI 线程 Action 统一通过 FIFO、非重入的 `EventPump<Action>` 执行；
- 后台线程统一通过 `ProductEventSender<Event>` 发送类型化完成事件；
- sender 必须保证“事件成功入队后再发送无 payload 的 `ShellEvent::ProductWake`”；
- UI 线程统一通过 `ProductEventInbox<Event>` 批量 drain，再由产品自己的解释器处理；
- `appkit-shell` 不依赖 Textora、Notora、workspace 或文档业务类型。

共享的是运行机制，不共享业务大枚举。禁止引入 `CommonAppEvent` 一类同时包含两个产品
payload 的类型。

## 现状问题

1. Notora 在 `product.rs` 内自行组合 `mpsc`、共享 wake handle 和 send error；Textora 在
   `textora_product.rs` 中再次实现 channel，并在同步、最近文件、macOS open-document
   路径分别手写 wake 顺序。
2. Notora 有独立 `EventPump`，Textora 的 `AppAction` 仍直接 reduce/apply，两个产品对同步
   重入的约束不同。
3. `NotoraProductEvent` 是扁平大枚举，多数变体重复携带 workspace id 与 generation；
   `ProductEventCoordinator` 因而同时承担 workspace、文档和持久化三类完成事件。
4. Textora 的 open-document 请求使用第二条 channel，生命周期入口需要分别 drain 两个
   inbox，无法获得统一 FIFO 顺序。

## 目标结构

```text
appkit-shell
├── EventPump<Action>
│   └── UI 线程 FIFO / draining 状态
└── ProductEventInbox<Event> + ProductEventSender<Event>
    ├── mpsc typed payload
    ├── ProductWakeHandle 注册
    └── enqueue-before-wake 协议

Textora
├── EventPump<AppAction>
├── ProductEventInbox<TextoraProductEvent>
└── TextoraProductEventInterpreter

Notora
├── EventPump<NotoraAction>
├── ProductEventInbox<NotoraProductEvent>
└── ProductEventCoordinator
    ├── WorkspaceCompletionInterpreter
    ├── DocumentCompletionInterpreter
    └── PersistenceCompletionInterpreter
```

## 公共协议

### EventPump

- `enqueue` 只追加 Action；
- `start_draining` 从 `Idle` 转为 `Draining`，重入调用返回 `AlreadyDraining`；
- 当前 Action 的 reduce/effect 全部完成后才能取下一 Action；
- `finish_draining` 要求队列为空并恢复 `Idle`；
- 不认识 reducer、effect、窗口或产品状态。

### ProductEventInbox

- `product_event_channel()` 创建一个 sender/inbox 对；
- wake handle 可以晚于 channel 创建注册，便于启动阶段先分发 sender；
- `send(event)` 先向 channel 提交 payload，成功后才调用 wake；
- 尚未注册 wake 时允许入队，启动后的首次 drain 仍可消费；
- `drain()` 保持 channel FIFO，返回本轮所有已到达事件；
- receiver 已关闭与 event loop 已关闭使用不同错误类型，避免调用方错误重试造成重复事件。

## 产品事件分域

Notora 的后台事件拆为：

- `WorkspaceCompletionEnvelope { scope, completion }`：工作区查询、索引、笔记命令、元数据、
  回收站、工作区文档加载；
- `DocumentCompletion`：外部文件打开、外部文档加载、Save As、冲突 reload/retry；
- `PersistenceCompletion`：产品设置和 session 持久化。

`WorkspaceEventScope { workspace_id, generation }` 只出现一次，`NotoraProduct` 在 inbox drain 时
统一丢弃非活动 scope。搜索 generation 和 selection generation 仍由各自领域解释器校验，
不得合并为一个含义模糊的 generation。

Textora 的产品事件保持较小，但 open-document、recent-files 和 sync-results 统一进入同一
`TextoraProductEvent`，由产品解释器顺序处理。

## 分阶段迁移

### 阶段 A：公共事件原语

- 在 `appkit-shell` 新增事件运行时模块；
- 迁移 Notora `EventPump`；
- 用单元测试证明 FIFO、非重入、延迟 wake 注册和 enqueue-before-wake。

### 阶段 B：Notora 产品 inbox

- 用共享 sender/inbox 替换自有 `mpsc` 和 wake 存储；
- 保留 workspace 过期事件门控；
- 删除 Notora 自有 channel/send error 实现。

### 阶段 C：Textora 产品 inbox 与 Action pump

- recent-files、sync-results、open-document 合并到同一 inbox；
- macOS bridge 只持 typed sender，不再持第二份 wake handle；
- `App::dispatch` 接入共享 `EventPump<AppAction>`。

### 阶段 D：Notora 完成事件分域

- 引入 workspace envelope 和三类 completion；
- 将协调器拆为三个解释器及三个窄 target protocol；
- 删除扁平事件字段匹配和跨领域 target 权限。

### 阶段 E：边界验证

- 两个产品源码不得直接出现产品事件用的 `mpsc::Sender/Receiver`；
- 两个产品必须引用 `appkit_shell::EventPump` 和 `ProductEventInbox`；
- `appkit-shell` 不得出现 `Notora`、`TextoraProductEvent`、workspace 或文档业务类型；
- 运行 `./scripts/verify.sh`。

## 完成判据

只有同时满足以下条件才算完成：

1. Textora 与 Notora 实际运行路径都使用公共 Action pump 和产品 inbox；
2. 两端不存在重复的 enqueue/wake/channel 基础设施；
3. Notora 后台完成事件按 workspace、document、persistence 分域；
4. 过期 workspace/search/selection 结果门控及事件 FIFO 顺序有回归测试；
5. 全工作区格式、Clippy、测试和架构验证通过。
