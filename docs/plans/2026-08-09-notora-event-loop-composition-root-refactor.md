# Notora 事件循环与组合根重构计划

## 目标

优化 Notora 的事件机制，使事件处理具备确定的非重入顺序、完整的通用
`ShellEffect` 执行语义和可独立测试的后台完成协调边界；最终让 `NotoraApp`
只负责构造依赖、接入 `winit::ApplicationHandler` 与有序关闭，不再承载产品事件翻译、
定时任务调度和具体 effect 实现。

## 当前问题

1. `dispatch_action` 在执行 effect 时允许 effect service 同步再次调用
   `dispatch_action`，外层 effect 与内层 action 的执行顺序依赖调用栈。
2. `ShellEffect` 声明了 reshape、窗口 chrome、标题、设置、workspace 和 redraw，
   Notora 仅消费 redraw；协议与实际执行不一致。
3. `drain_product_events` 同时负责排空 inbox、过滤陈旧结果、更新 runtime、维护 pending
   状态并派发产品 action。
4. `about_to_wait` 直接知道自动保存、搜索、会话持久化、catalog backup 和两套光标闪烁。
5. `NotoraApp` 是所有上述职责的实现中心，而不是依赖组合根。

## 目标结构

```text
NotoraApp                         组合根 / winit adapter
└── NotoraRuntime                产品与编辑器运行时门面
    ├── EventPump                非重入 Action -> Effect 泵
    ├── ProductEventCoordinator  后台完成翻译与陈旧结果门控
    ├── DeadlineCoordinator      deadline 聚合和到期任务调度
    ├── ShellEffectExecutor      通用 shell effect 固定顺序执行
    └── NotoraProduct            后台服务容器与 typed inbox
```

`NotoraApp` 可以持有这些对象并实现薄委托，但不得重新包含它们的具体分支逻辑。

## 不变量

- `ui` 不依赖 `NotoraState`、`DocumentView` 或 app 层状态。
- 产品业务 action/effect 保持类型化，不向 `ShellEffect` 塞产品 payload。
- 后台 payload 先进入 Notora 自有 channel，`winit` 只接收无 payload wake。
- 所有 `NotoraState` 变更发生在 UI 线程。
- 同一 dispatch 周期内，新 action 只入队；不允许递归执行 reducer/effect。
- `ShellEffect::steps()` 是通用 effect 的唯一执行顺序来源。
- 工作区、搜索和文档 selection generation 的陈旧结果门控不得弱化。
- `about_to_wait` 使用 `Wait` / `WaitUntil`，不得引入 busy polling。

## 分阶段实施

### 阶段 A：非重入事件泵

文件不超过 3 个：

- 新增 `crates/notora-app/src/event_pump.rs`
- 修改 `crates/notora-app/src/lib.rs`
- 修改 `crates/notora-app/src/app.rs`

先写测试证明 effect 执行期间产生的新 action 排到剩余旧 effect 之后，再接入
`NotoraApp::dispatch_action`。事件泵显式维护 `VecDeque<NotoraAction>` 和 draining 状态。

### 阶段 B：ShellEffect 完整执行

文件不超过 3 个：

- 新增 `crates/notora-app/src/shell_effect_executor.rs`
- 修改 `crates/notora-app/src/lib.rs`
- 修改 `crates/notora-app/src/app.rs`

按 `ShellEffect::steps()` 执行 reshape、window chrome、title、settings、workspace 与 redraw。
产品 effect 已经完成的 I/O 不再返回误导性的 shell persistence effect。

### 阶段 C：后台完成协调器

文件不超过 3 个：

- 新增 `crates/notora-app/src/product_event_coordinator.rs`
- 修改 `crates/notora-app/src/lib.rs`
- 修改 `crates/notora-app/src/app.rs`

把 inbox drain 后的大型 match 从 `NotoraApp` 移入协调器；协调器返回类型化 completion
commands，由 runtime 门面执行。该阶段不得把 runtime 或 app 状态暴露给 `NotoraProduct`。

### 阶段 D：deadline 协调器

文件不超过 3 个：

- 新增 `crates/notora-app/src/deadline_coordinator.rs`
- 修改 `crates/notora-app/src/lib.rs`
- 修改 `crates/notora-app/src/app.rs`

统一聚合 deadline，并返回到期任务枚举；`events.rs` 只调用一个 tick 入口并设置
`ControlFlow`。

### 阶段 E：组合根收口

文件不超过 3 个：

- 新增或拆分 `crates/notora-app/src/runtime.rs`
- 修改 `crates/notora-app/src/app.rs`
- 修改 `crates/notora-app/src/events.rs`

将 effect service、产品完成命令执行和 editor notification 协调迁入 `NotoraRuntime`。
`NotoraApp` 最终只保留构造、公开产品 API 的薄委托、生命周期和组合对象所有权。

## 验证

每阶段：

```bash
cargo fmt --all -- --check
cargo check -p notora-app
cargo test -p notora-app event_pump
cargo test -p notora-app effect_executor
cargo test -p notora-app product
```

最终：

```bash
./scripts/verify.sh
```

完成判据不是文件移动，而是：事件泵无同步重入、所有通用 effect 有明确消费者、后台完成
与 deadline 分支不再位于 `NotoraApp`、`ApplicationHandler` 仅做适配，并且全量验证通过。

## 实施结果

- `NotoraApp` 已收敛为构造、公开 API 和 `ApplicationHandler` 薄委托。
- `EventPump` 统一保证 action/effect 的 FIFO 非重入执行顺序。
- 产品完成事件、deadline 和通用 shell effect 分别由独立协调器处理。
- 架构边界检查、格式检查、全工作区 Clippy、单元测试、集成测试与文档测试已通过。
