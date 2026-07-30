# App 核心架构深度优化计划

在完成基础模块拆分后，我们需要进一步通过引入更纯粹的状态管理机制和上下文封装，提升代码的可测性、降低文件间的隐式耦合。针对您的三点需求以及对 `app.rs` 本身的进一步瘦身，制定如下实施计划。

## User Review Required

> [!IMPORTANT]
> 引入 Redux 风格的 `Action` 派发机制将改变目前事件直接修改状态的习惯。该机制会使得 `events.rs` 中的方法变得更纯粹，返回需要执行的 `Action` 列表，而不再直接操作 `&mut App`。请审核这套模式是否符合您对后续架构的预期。

## Proposed Changes

### 1. 更彻底的 `events.rs` 抽离 (Action 派发机制)
* **目标**：彻底切断 `events.rs` 中对 `App` 结构体的隐式大面积可变借用 (`&mut App`)，让事件处理变成“计算出 Action”的纯数据转换流。
* **实施方案**：
  - **[NEW] 定义 `AppAction`**: 在新建的 `actions.rs` (或 `events.rs` 中) 定义 `enum AppAction`，囊括状态变更意图，例如 `AppAction::RequestRedraw`, `AppAction::UpdateDocumentEdited(bool)`, `AppAction::OpenDialog`, `AppAction::CloseTab(usize)` 等。
  - **[MODIFY] 重构事件签名**: 修改 `events.rs` 中的事件处理器，去除或弱化对 `&mut App` 的依赖，使其返回 `Vec<AppAction>`。
  - **[MODIFY] 统一派发枢纽**: 在 `app.rs` 中的 `App` 结构体实现 `fn dispatch(&mut self, action: AppAction)` 方法，作为一个集中的 Reducer 来处理所有纯状态变更，便于追踪调试。

### 2. `Workspace` 与 `App` 的界限划分
* **目标**：明确划分业务领域边界，将多标签页的生命周期逻辑（新建、关闭、切换、脏检查等）完全从 `App` 及 `events` 下沉收敛至 `Workspace`。
* **实施方案**：
  - **[MODIFY] `workspace.rs`**: 丰富并完善 `Workspace` 的核心方法。例如，将 `close_tab` 的前置状态检查、焦点切换回滚等内聚到 `Workspace` 内部。
  - **[NEW] 状态反馈枚举**: 当 `Workspace` 执行操作时，可能需要 UI 层的协助（例如弹窗提示未保存）。因此定义 `enum WorkspaceEffect`，让 `Workspace` 方法返回该 Effect 列表，再由 `App` 捕获并转换为弹窗等 UI 动作。
  - **[MODIFY] `events.rs`**: 使事件层只负责将 UI 操作（如鼠标点击 Tab 关闭按钮）翻译为对 `Workspace` API 的调用意图。

### 3. 渲染管道抽象化 (`RenderContext` 引入)
* **目标**：消除 `layout.rs`、`gutter.rs` 和 `decorations.rs` 现存的过长散装参数列表，同时避免直接传递整个 `App` 引用带来的生命周期冲突。
* **实施方案**：
  - **[NEW] 定义 `RenderContext`**: 在 `render_pipeline.rs` 内部定义 `struct RenderContext<'a>` 或类似的 Context 结构体，封装渲染所需的共享上下文组合（如 `&'a Settings`, `&'a Theme`, `&'a mut RenderCache`, `&'a GpuState` 等）。
  - **[MODIFY] `layout.rs`**: 调整接口，统一接受 `&mut RenderContext` 与具体的组件数据。
  - **[MODIFY] `gutter.rs`**: 调整接口，使用 `&mut RenderContext`。
  - **[MODIFY] `decorations.rs`**: 同上，使这三个渲染子模块的签名保持一致，且对 `App` 主体解耦。

### 4. `app.rs` 本身的进一步瘦身 (App Core Thinning)
* **目标**：将 `App` 结构体进一步瘦身到单纯的“总线组件”角色，剥离散落在内的游离状态和代理方法，力争将其行数压缩至 1000 行以内。
* **实施方案**：
  - **[MODIFY] 光标与视口状态分离**: 将 `App` 中的 `cursor_pixel_x`, `sticky_x`, `first_line`, `last_line`, `last_cursor_offset` 等十几个零散字段提取至单独的 `CursorState` 结构体，甚至下沉到 `DocumentView` 内部（如果它是以文档为粒度的话）。
  - **[MODIFY] 代理方法剥离**: 将 `move_cursor_visual`, `ensure_cursor_visible`, `page_up` 等庞大的滚动和计算逻辑彻底移出 `app.rs`，放到 `cursor_motion.rs` 和 `viewport.rs` 之中。
  - **[MODIFY] Winit 事件入口化简**: 将 `ApplicationHandler` 实现中的海量匹配分支 (`match event`) 直接转发给 `events.rs` 的顶层入口，`app.rs` 只负责 `self.dispatch(event_action)`。

## Verification Plan

### 自动化检查
- 执行 `cargo check --workspace` 确保引入 `RenderContext`、分离 `Action` 和 `CursorState` 没有导致 Rust 生命周期的借用冲突。

### 手动验证功能点
- 模拟点击 Tab 栏的关闭按钮，确保：(1) 事件层触发正确 Action (2) Workspace 执行关闭操作 (3) UI 正常响应重绘。
- 在有未保存修改的情况下关闭文件，检查是否能正常流转出 `WorkspaceEffect::RequireSaveConfirmation`。
- 随意输入字符，观察渲染管线接收 `RenderContext` 后，语法高亮、光标闪烁以及行号区域渲染是否与之前保持一致，不存在闪烁或偏移。
- 测试光标的跨行移动（上下左右、PageUp/Down），确保 `CursorState` 的抽离没有导致坐标计算错误。
