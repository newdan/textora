# NotoraRuntime 职责收敛与组件化重构方案

> 实施状态：已完成（2026-08-09）

## 实施结果

方案的 0–10 阶段已全部落地。最终 `NotoraRuntime` 直接持有的可变运行时对象以命名组件为主：

- `ActionRuntime`：唯一拥有 `NotoraState`、`EventPump` 与 search debounce；
- `DocumentRuntime`：唯一拥有 editor、document registry、LRU、autosave 和全部文档级 pending workflow；
- `PersistenceRuntime`：唯一拥有 settings、session、catalog backup deadline 与 persistence worker；
- `WindowRuntime`：唯一拥有窗口焦点、尺寸、pointer、event-loop proxy 与 redraw 合并状态；
- `FrameRuntime`：唯一拥有 shell、UI settings/theme、字体准备、startup trace 与 GPU frame 提交逻辑；
- `NotoraProduct`、`WorkspaceController` 继续作为顶层独立组件。

Action effect 通过 `EffectExecution` 返回 follow-up actions，文档工作流通过 `DocumentOutcome` 和
`DocumentCommand` 串行交还顶层编排。`ProductEventCoordinator` 返回保持 channel FIFO 的
`ProductCompletions`，`DeadlineCoordinator` 仅消费 `DeadlineSnapshot`；两者均不再引用
`NotoraRuntime`。`ShellEffectTarget` 也改为一次调用期的 `RuntimeShellEffectTarget`，不再由整个
Runtime 实现。

实施中保留了两项有意的边界差异：

- scale factor 继续由 `EditorRuntime` 保存，因为它是 editor 渲染资源的内在状态；
  `WindowRuntime` 只拥有平台窗口快照并触发相应 redraw；
- product completion 不复制成第二套业务事件 enum，而是以有序 `Vec<NotoraProductEvent>`
  交给 Runtime 做穷尽路由，避免协议重复和 completion 重排。

最终新增结构测试，禁止 Runtime 重新吸收 pending collection、GPU primitive、完整 coordinator
访问权或 effect service。`./scripts/verify.sh` 已通过。

## 背景

上一阶段已经完成 Notora 事件循环与组合根重构：`NotoraApp` 只负责构造和
`winit::ApplicationHandler` 委托，Action/Effect 执行具备 FIFO 非重入语义，产品后台完成、
deadline 和通用 `ShellEffect` 也已有独立协调入口。

实施前剩余问题集中在 `NotoraRuntime`。它的外部定位已经是“应用运行时门面”，但内部仍同时
持有并实现以下职责：

- 产品状态归约和 effect 执行；
- 窗口生命周期、输入路由、焦点、指针和 redraw；
- editor runtime、文档注册、预览和 LRU；
- 自动保存、手动保存、Save As、冲突恢复和标题落盘；
- workspace、catalog、外部文件和后台完成事件；
- 设置、session、catalog backup 和退出快照；
- shell render model、GPU frame 和字体准备。

`runtime.rs` 当前约 5600 行（包含测试），生产代码区域拥有大量彼此独立的 pending 状态，
并直接实现 `NotoraEffectService` 与 `ShellEffectTarget`。这说明组合根问题已经解决，但运行时
内部仍然是实现中心。

## 目标

将 `NotoraRuntime` 收敛为真正的运行时编排器，只保留四类职责：

1. 组合并持有顶层运行时组件；
2. 接收平台事件、产品 wake 和 deadline tick；
3. 保证 Action、Effect、Completion、Render 的执行顺序；
4. 统一管理启动、首帧、正常关闭和异常退出边界。

具体业务工作流、pending 状态、渲染细节和持久化细节必须由有明确所有权的组件负责。

本次重构不以“减少文件行数”为唯一目标。完成标准是状态所有权唯一、组件接口窄、执行顺序
显式，并且 `NotoraRuntime` 不再实现具体产品工作流。

## 非目标

- 不替换 `winit`，不引入 Tokio 或新的异步运行时。
- 不引入 Qt 式 QObject、signal/slot 或全局事件总线。
- 不修改 `NotoraState` 的产品语义和现有 Action/Effect 业务含义。
- 不允许 `ui` 依赖 `NotoraState`、`DocumentView` 或 app 层结构。
- 不把 typed product payload 塞入 `ShellEvent` 或 `ShellEffect`。
- 不为了绕过 Rust 借用检查而引入 `Rc<RefCell<_>>`、全局可变状态或 `unsafe`。
- 不在本次重构中重新设计编辑器插件协议、workspace catalog 或文件格式。

## 核心设计原则

### 1. 所有权先于方法拆分

仅把 `impl NotoraRuntime` 移到多个文件不能算完成。每一组状态必须迁入负责其生命周期的
组件，并由该组件维护内部不变量。

### 2. 输入输出类型化

组件之间使用语义化 command、completion、snapshot 和 outcome，不互相读取整块 runtime。
协调器不得继续接收 `&mut NotoraRuntime`。

### 3. 保持单线程状态所有权

`NotoraState`、文档 runtime、pending workflow 和窗口状态仍只在 UI 线程修改。后台线程仅
返回 typed completion，由主事件循环应用。

### 4. 顺序是一等协议

必须保留以下顺序：

```text
Platform/Product Event
  -> enqueue Action or Completion
  -> reduce Action
  -> execute all Effects in declaration order
  -> enqueue follow-up Actions
  -> apply merged ShellEffect
  -> request/coalesce redraw
  -> render on RedrawRequested
```

effect 产生的 follow-up action 只能入队，不能在 effect service 内同步递归执行 reducer。

### 5. Runtime 允许编排，不允许实现工作流

Runtime 可以根据一个小型 typed outcome 决定下一位接收者；不应包含“如何保存文档”、
“如何恢复冲突”或“如何构造 GPU frame”等具体流程。

## 目标结构

```text
NotoraApp                              组合根 / winit adapter
└── NotoraRuntime                     顶层运行时编排器
    ├── ActionRuntime                 State + EventPump + reducer 顺序
    ├── DocumentRuntime               打开文档、editor、保存与文档工作流
    ├── PersistenceRuntime            settings/session/backup 持久化
    ├── WindowRuntime                 窗口状态、输入坐标、wake/redraw
    ├── FrameRuntime                  shell model、字体准备和帧渲染
    ├── NotoraProduct                 typed 后台 inbox 与产品服务
    └── WorkspaceController           workspace 生命周期与 worker
```

现有 `ProductEventCoordinator`、`DeadlineCoordinator` 和 `ShellEffectExecutor` 保留，但改为
依赖窄接口或纯数据输入，不再拥有访问整个 `NotoraRuntime` 的特权。

## 组件职责

### ActionRuntime

拥有：

- `NotoraState`；
- `EventPump<NotoraAction>`；
- 与产品 Action 直接关联的 debounce 状态，例如 `SearchController`。

负责：

- Action 入队和 drain 生命周期；
- 调用 reducer；
- 保证当前 Action 的全部 Effect 完成后才处理下一 Action；
- 收集 effect follow-up actions 和合并后的 `ShellEffect`；
- 暴露只读 `state()` 和窄的 snapshot 查询。

不负责：

- 文件 I/O、workspace command、文档保存或窗口操作；
- 直接持有 `EditorRuntime`、`NotoraProduct` 或 `PersistenceWorker`。

建议引入明确的执行结果：

```rust
struct EffectExecution {
    shell_effect: ShellEffect,
    follow_up_actions: Vec<NotoraAction>,
}
```

`NotoraEffectService` 的同步失败、创建完成等结果通过 `follow_up_actions` 返回，不再调用
`NotoraRuntime::dispatch_action`。这样非重入规则由类型和所有权共同保证，而不是依赖调用者
自觉。

### DocumentRuntime

拥有：

- `EditorRuntime`；
- `DocumentRegistry`；
- `RuntimeLru`；
- `AutoSaveScheduler`；
- save failure 和外部文档加载状态；
- conflict retry、trash move、note move、title update、title seed 等文档级 pending 状态。

负责：

- 文档准备、安装、预览提升和 runtime 淘汰；
- editor notification 转换；
- 自动保存、手动保存、Save As 和保存完成；
- 文档冲突处理和 pending workflow 的 revision 校验；
- 文档路径变化后 registry/editor 的一致性；
- 退出时生成 dirty snapshot 计划。

对外提供类型化入口，例如：

```rust
enum DocumentCommand {
    Prepare(DocumentLoadRequest),
    SaveManually(ManualSaveRequest),
    ResolveConflict(SaveConflictRequest),
    PromotePreview,
}

struct DocumentOutcome {
    actions: Vec<NotoraAction>,
    workspace_commands: Vec<WorkspaceCommand>,
    shell_effect: ShellEffect,
}
```

命令与结果应按真实互斥状态建模；不得用多个 bool 组合 pending 状态。

### PersistenceRuntime

拥有：

- `ProductSettings` 与 `SettingsPersistenceState`；
- `PersistenceWorker`；
- pending session 及 session persistence deadline；
- catalog backup deadline 和相关持久化状态。

负责：

- 设置更新、顺序写入和失败重试；
- session snapshot 的 debounce、提交和完成处理；
- catalog backup 的调度、flush 和关闭语义；
- 暴露自身 `next_deadline()` 和 `process_due_work(now)`。

`PersistenceRuntime` 不直接读取整个产品或文档状态。Runtime 应先构造纯数据
`ProductSessionSnapshot`、`WindowGeometry` 等输入，再交给它持久化。

### WindowRuntime

拥有：

- `EventLoopProxy<ShellEvent>`；
- window focused、物理尺寸、指针位置；
- redraw pending 状态；
- 与窗口生命周期直接相关的轻量状态。

负责：

- 窗口焦点、尺寸、scale factor 和 pointer snapshot；
- wake、redraw 请求及 redraw 合并；
- 将平台窗口能力暴露为窄端口。

它不判断产品 overlay、卡片焦点或 editor focus；这些判断由 ActionRuntime 提供的状态快照与
输入路由策略完成。

### FrameRuntime

拥有：

- `NotoraShell`；
- UI settings/theme 的渲染快照；
- `StartupTrace` 和字体准备状态；
- frame 构建所需的渲染资源。

负责：

- 构造静态 shell render model；
- 同步 editor render model；
- GPU frame 提交；
- 首帧记录、字体准备和 reshape invalidation。

输入必须是纯数据或窄的 editor render port。不得让 `ui` 或 FrameRuntime 获取
`NotoraState` 的可变引用。

### NotoraRuntime

最终只应直接实现以下语义：

```text
new / resume
on_shell_event
on_window_event
dispatch_action
tick
render
shutdown
```

它可以持有不可变路径配置和顶层组件，但不应再直接持有文档级 pending map，也不应直接实现
文件保存、冲突恢复、session 序列化或 GPU primitive 构造。

## 协议调整

### Effect 执行协议

当前 `NotoraEffectService for NotoraRuntime` 允许 service 方法再次调用 `dispatch_action`。
目标协议为：

1. `ActionRuntime` reduce 当前 Action，得到有序 Effect 列表；
2. `RuntimeEffectServices` 仅借用所需组件，不持有整个 Runtime；
3. 每个 Effect 返回 `EffectExecution`；
4. follow-up actions 追加到 EventPump 尾部；
5. 当前 Action 的所有 Effect 完成后才取下一 Action；
6. 合并后的 `ShellEffect` 交给 `ShellEffectExecutor`。

`RuntimeEffectServices` 是一次 drain 周期的短生命周期借用集合，不得演变成长期 service
locator，也不得暴露未使用组件。

### Product completion 协议

`ProductEventCoordinator` 不再直接修改 Runtime。建议返回保持 inbox 顺序的
`Vec<ProductCompletion>`：

```rust
enum ProductCompletion {
    Dispatch(NotoraAction),
    Document(DocumentCompletion),
    Persistence(PersistenceCompletion),
    Workspace(WorkspaceCompletion),
    Shell(ShellEffect),
}
```

该枚举只描述 product inbox 的完成语义，不作为全应用通用 command bus。Runtime 按原顺序
把每项交给唯一所有者，组件再返回 Action 或 ShellEffect。

generation、workspace identity 和 selection identity 的陈旧结果过滤必须发生在 completion
产生或应用前，且有独立测试覆盖 A-B-A、workspace switch 和 tab close 场景。

### Deadline 协议

每个拥有 deadline 的组件提供：

```rust
fn next_deadline(&self) -> Option<Instant>;
fn process_due_work(&mut self, now: Instant) -> ComponentOutcome;
```

`DeadlineCoordinator` 只负责取最早时间和按固定顺序 tick 组件，不读取组件内部字段。
`about_to_wait` 继续只设置 `Wait` 或 `WaitUntil`，不得 busy polling。

### ShellEffect 协议

`ShellEffectExecutor` 继续以 `ShellEffect::steps()` 作为唯一执行顺序来源。目标端口拆分为
Window、Frame 和 Persistence 三类能力，避免 `ShellEffectTarget for NotoraRuntime` 再次成为
全能接口。

## 状态所有权迁移表

| 当前状态 | 目标所有者 |
|---|---|
| `state`, `event_pump`, `search_controller` | `ActionRuntime` |
| `editor_runtime`, `document_registry`, `runtime_lru`, `autosave` | `DocumentRuntime` |
| 所有 document/save/title/move pending map | `DocumentRuntime` |
| `product_settings`, `settings_persistence`, `persistence_worker` | `PersistenceRuntime` |
| pending session、session/catalog deadline | `PersistenceRuntime` |
| `window_focused`, window size、pointer、redraw、event proxy | `WindowRuntime` |
| `shell`, `settings`, `theme`, font preparation、startup trace | `FrameRuntime` |
| `product`, `workspace_controller` | 继续作为顶层独立组件 |
| `paths` | `NotoraRuntime` 的不可变组合配置，按引用传入组件 |

如果某个字段迁移后仍被三个以上组件直接修改，说明边界设计失败，应先补充 typed snapshot 或
command，而不是增加 `pub(crate)` 字段。

## 分阶段实施

每阶段最多修改三个文件，并在提交前确保编译通过。发生行为回归时先补充复现测试，不叠加
防御性补丁。

### 阶段 0：冻结行为与结构约束

涉及文件：

- `crates/notora-app/tests/smoke.rs`
- `crates/notora-app/src/runtime.rs`
- 本计划文档

工作内容：

- 增加结构测试，保证 `NotoraApp` 继续是薄组合根；
- 增加事件顺序、关闭顺序、首帧 session restore 的 characterization tests；
- 记录当前所有 pending 状态及其唯一写入路径；
- 禁止重构期间改变 Action/Effect 外部语义。

实施基线（2026-08-09）：

- Action 队列顺序由 `event_pump` 单元测试冻结：effect 追加的 action 必须等待当前 action 的
  全部 effect 完成，跨 action 保持 FIFO；
- shell effect 的固定执行顺序由 `shell_effect_executor` 单元测试冻结；
- 首帧先 render、再恢复 session，以及退出时 save drain → catalog flush → session/settings
  enqueue → persistence/product/editor shutdown 的顺序由 `tests/smoke.rs` 冻结；
- 当前 pending 状态的唯一写入域记录如下，迁移时每组状态必须整体移动，不允许双写：

| pending 状态 | 当前唯一写入域 | 目标所有者 |
|---|---|---|
| `pending_session`, `pending_session_persist_at`, `pending_catalog_backup_at` | session restore、deadline 与 shutdown | `PersistenceRuntime` |
| `settings_persistence` | settings effect 与 persistence completion | `PersistenceRuntime` |
| `autosave`, `save_failure_messages` | editor notification、save completion 与 shutdown drain | `DocumentRuntime` |
| `pending_external_save_as`, `pending_external_documents` | external open/save workflow | `DocumentRuntime` |
| `pending_conflict_retries` | conflict resolution workflow | `DocumentRuntime` |
| `pending_trash_moves`, `pending_note_moves` | save-before-move workflow | `DocumentRuntime` |
| `pending_title_updates`, `pending_title_seeds` | title save/initialization workflow | `DocumentRuntime` |
| `pending_metadata_generations`, `pending_metadata_mutations` | metadata request/completion workflow | `DocumentRuntime` |
| `catalog_reconciliation_pending` | watcher completion/reconciliation workflow | `DocumentRuntime` |
| `needs_redraw` | shell effect、window event 与 render | `WindowRuntime` |

### 阶段 1：提取 ActionRuntime

涉及文件：

- 新增 `crates/notora-app/src/runtime/action_runtime.rs`
- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/event_pump.rs`

工作内容：

- 迁移 `NotoraState`、EventPump 和 search debounce 所有权；
- 建立 `enqueue`、`drain`、`state` 和 typed snapshot API；
- 保持所有 reducer 调用只有一个生产入口；
- 先保留兼容 adapter，再在阶段 2 删除同步嵌套 dispatch。

完成判据：

- `NotoraRuntime` 不再直接修改 `NotoraState`；
- reducer/effect FIFO 测试通过；
- 产品焦点同步通过 ActionRuntime 的明确 outcome 完成。

### 阶段 2：消除 Effect Service 对 Runtime 的反向调用

涉及文件：

- `crates/notora-app/src/effect_executor.rs`
- `crates/notora-app/src/runtime/action_runtime.rs`
- `crates/notora-app/src/runtime.rs`

工作内容：

- 引入 `EffectExecution`；
- 将 effect service 内的 `dispatch_action` 改为 follow-up actions；
- 用短生命周期 `RuntimeEffectServices` 组合所需能力；
- 删除 `NotoraEffectService for NotoraRuntime`。

完成判据：

- Effect service 无法直接调用 Runtime；
- effect 产生的 Action 在当前 Action 全部 Effect 之后执行；
- 同步失败、创建外部文档、移动和 metadata mutation 的测试保持通过。

### 阶段 3：提取 PersistenceRuntime

涉及文件：

- 新增 `crates/notora-app/src/runtime/persistence_runtime.rs`
- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/app/deadline_coordinator.rs`

工作内容：

- 迁移 settings/session/catalog backup 状态和 persistence worker；
- 让组件暴露自身 deadline 和 typed completion；
- session capture 改为纯数据 snapshot 输入；
- 明确 shutdown flush 顺序和错误传播。

完成判据：

- Runtime 不直接执行设置或 session 文件写入；
- 同一设置快照不会重复提交；
- session debounce、持久化失败重试和 catalog backup 测试通过。

### 阶段 4：提取 DocumentRuntime 状态容器

涉及文件：

- 新增 `crates/notora-app/src/runtime/document_runtime.rs`
- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/app/product_event_coordinator.rs`

工作内容：

- 迁移 EditorRuntime、DocumentRegistry、RuntimeLru、AutoSaveScheduler；
- 迁移全部文档级 pending state；
- 先建立 command/completion API，不同时重写保存算法；
- product completion 只通过 DocumentRuntime 的 typed 入口完成文档变更。

完成判据：

- `NotoraRuntime` 不再含文档级 HashMap/Vec pending 字段；
- tab identity、selection generation、preview promotion 和 LRU 不变量保持不变；
- coordinator 不直接访问文档内部字段。

### 阶段 5：迁移文档工作流

涉及文件：

- `crates/notora-app/src/runtime/document_runtime.rs`
- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/effect_executor.rs`

工作内容：

- 迁移 prepare/install/promote/evict；
- 迁移 autosave、manual save、Save As 和 completion；
- 迁移 conflict、trash move、note move、title update/seed；
- 由 `DocumentOutcome` 返回 Action、WorkspaceCommand 和 ShellEffect。

该阶段工作量较大，应按上述四组顺序形成独立提交；每个提交只改这三个文件，并分别编译。

完成判据：

- Runtime 中不存在具体保存和冲突恢复算法；
- 文档 revision 校验仍位于发起 I/O 前和应用 completion 前；
- save policy、recovery、trash 和 external open 集成测试全部通过。

### 阶段 6：提取 FrameRuntime

涉及文件：

- 新增 `crates/notora-app/src/runtime/frame_runtime.rs`
- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/events.rs`

工作内容：

- 迁移 shell render model、frame submission、字体准备和首帧 trace；
- 通过纯数据 `FrameInput` 获取产品布局与文档渲染信息；
- 保持 `RedrawRequested` 为唯一帧提交入口；
- reshape 由 FrameRuntime 消费。

完成判据：

- Runtime 不再包含 wgpu primitive 构造或顶点上传细节；
- resize/high-DPI/首帧恢复和 render smoke tests 通过；
- `ui` 依赖边界保持不变。

### 阶段 7：提取 WindowRuntime

涉及文件：

- 新增 `crates/notora-app/src/runtime/window_runtime.rs`
- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/events.rs`

工作内容：

- 迁移焦点、尺寸、scale、pointer、proxy 和 redraw 状态；
- 将 winit event 转换与产品输入路由分开；
- `events.rs` 保持平台 adapter，Runtime 只接收语义化输入；
- window title、cursor、IME area 和 redraw 通过 WindowRuntime 端口执行。

完成判据：

- Runtime 不直接保存平台坐标和窗口状态字段；
- overlay、焦点和 editor input gate 测试保持通过；
- redraw 仍然合并，空闲时使用 `Wait` / `WaitUntil`。

### 阶段 8：收窄协调器接口

涉及文件：

- `crates/notora-app/src/app/product_event_coordinator.rs`
- `crates/notora-app/src/app/deadline_coordinator.rs`
- `crates/notora-app/src/runtime.rs`

工作内容：

- 删除两个协调器对 `&mut NotoraRuntime` 的依赖；
- ProductEventCoordinator 返回有序 typed completion；
- DeadlineCoordinator 只依赖各组件的 deadline port；
- Runtime 仅保留小型、穷尽的顶层路由。

完成判据：

- coordinator 源码中不再出现 `NotoraRuntime`；
- 不引入通用 Any payload、字符串事件名或全局 command bus；
- 后台事件陈旧结果过滤测试完整通过。

### 阶段 9：Runtime 门面收口

涉及文件：

- `crates/notora-app/src/runtime.rs`
- `crates/notora-app/src/app.rs`
- `crates/notora-app/tests/smoke.rs`

工作内容：

- 删除迁移期 adapter、废弃字段和重复方法；
- 保留构造、事件编排、tick、render 和 shutdown；
- 增加结构测试防止 Runtime 再次吸收组件内部状态；

完成判据：

- `NotoraApp` 仍是薄组合根；
- `NotoraRuntime` 字段以组件为主，不含文档级 pending collection；
- Runtime 不实现 `NotoraEffectService`，不含具体持久化和 GPU frame 算法；
- 每种 mutable state 都有唯一、可命名的所有者。

### 阶段 10：文档归档

涉及文件：

- 本计划文档

工作内容：

- 更新最终结构、实际迁移差异和实施结果；
- 记录未进入本轮的后续优化项；
- 勾选最终验收清单。

## 测试与验证策略

### 每个小阶段

```bash
cargo fmt --all -- --check
cargo check -p notora-app
cargo test -p notora-app event_pump
cargo test -p notora-app effect_executor
cargo test -p notora-app product
```

根据阶段追加对应测试：

```bash
cargo test -p notora-app --test save_policy
cargo test -p notora-app --test recovery_flow
cargo test -p notora-app --test trash_flow
cargo test -p notora-app --test smoke
```

### 每个提交前

- `cargo fmt --all`；
- `cargo check -p notora-app`；
- 本阶段相关测试全部通过；
- 删除 unused imports、过渡 adapter 和死代码；
- 检查没有新增 `.unwrap()`、魔法值或多个 bool 表示互斥状态。

### 最终验证

```bash
./scripts/verify.sh
```

## 必须保持的不变量

- `NotoraState` 只在 UI 线程归约。
- 一个 Action 的 Effect 顺序与 reducer 返回顺序一致。
- effect follow-up action 不允许同步重入。
- Product completion 保持 channel FIFO 顺序。
- workspace generation、search generation、selection generation 不得弱化。
- 外部文件永不被 dirty snapshot 或 trash workflow 意外删除。
- 保存完成只能清理与其 revision 匹配的 dirty 状态。
- resize、scale factor 和 theme change 必须在下一帧前完成 reshape invalidation。
- redraw 可合并但不能丢失；无 deadline 时不得轮询。
- shutdown 顺序必须先停止新任务，再 flush persistence，最后 join worker。

## 主要风险与应对

### Rust 多重可变借用导致重新聚合

风险：为了同时访问多个组件，把所有字段重新包装进一个巨大的 context 或 service locator。

应对：让组件返回 typed outcome，由 Runtime 串行应用；只在一次调用栈内构造短生命周期能力
集合，不保存跨事件借用。

### completion 重排

风险：按领域批量分组 completion 后改变原 channel 顺序。

应对：使用有序 `Vec<ProductCompletion>`，逐项应用；增加同一批次内先保存后移动、先加载后切换
选择等顺序测试。

### pending 状态被复制

风险：迁移期 Runtime 与新组件各保留一份状态，形成双写。

应对：每次只迁移一组字段；字段进入新组件后立即删除旧字段，通过编译错误找出所有调用点。

### 只移动文件、不形成边界

风险：新模块继续通过 `&mut NotoraRuntime` 访问所有内部字段。

应对：阶段完成标准以所有权和函数签名为准；最终结构测试禁止 coordinator 和子组件依赖完整
Runtime。

### 关闭流程回归

风险：worker join、settings flush、session snapshot 和 dirty snapshot 顺序发生变化。

应对：在第一阶段冻结 shutdown characterization tests，最后才删除旧 adapter。

## 最终验收清单

- [x] `NotoraApp` 仍只负责组合和平台生命周期委托。
- [x] `NotoraRuntime` 只负责顶层编排与生命周期。
- [x] `ActionRuntime` 是 `NotoraState` 和 EventPump 的唯一所有者。
- [x] `DocumentRuntime` 是 editor/document/save pending 状态的唯一所有者。
- [x] `PersistenceRuntime` 是 settings/session/backup 状态的唯一所有者。
- [x] `WindowRuntime` 是平台窗口瞬时状态的唯一所有者。
- [x] `FrameRuntime` 不获取 app 层可变状态。
- [x] `NotoraEffectService for NotoraRuntime` 已删除。
- [x] coordinator 不再接收 `&mut NotoraRuntime`。
- [x] 不存在同步递归 reducer/effect 执行。
- [x] 不存在通用字符串事件或无类型 payload。
- [x] 所有架构边界、Clippy、单元、集成和文档测试通过。
- [x] `./scripts/verify.sh` 通过。

## 后续优化项

- `runtime.rs` 的大型测试模块可单独迁入测试文件，降低生产编排代码的阅读噪声；这不影响状态
  所有权或运行时边界，因此未纳入本轮行为重构。
- `DocumentRuntime` 后续可按 external/save/conflict 子域继续拆分内部私有模块，但不应改变其
  对外 typed command/outcome 协议，也不应把 pending 状态移回 Runtime。

## 决策总结

这次优化不把 Notora 改造成 Windows 消息分发器或 Qt signal/slot 系统。现有的单 UI 线程、
typed Action/Effect、后台 channel wake 和 deadline 驱动机制继续保留。重构重点是给每组状态
确定唯一所有者，让 `NotoraRuntime` 从“所有功能的实现位置”变成“组件之间顺序明确的运行时
编排器”。
