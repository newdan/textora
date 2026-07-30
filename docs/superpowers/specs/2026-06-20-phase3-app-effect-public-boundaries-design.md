# Phase 3 AppEffect / Dispatch / Public Boundaries Design

## 目标

完成 Phase 3 重构没有闭环的两条应用层边界：

1. AppAction 进入 dispatch 后，领域 handler 只修改领域状态并返回 AppEffect；顶层 dispatch 是该调用链唯一的 effect 应用点。
2. crates/app 的 crate root 只公开主程序、集成测试和 benchmark 真正需要的契约，不再把内部实现模块整体暴露为公共 API。

本设计在 Settings/DPI 行为稳定和逻辑 Settings / 物理 UiMetrics 两个子项目完成后实施。它不改变用户可见行为，也不承担 Phase 4 UI 边界或质量门禁整改。

## 现状与缺口

Phase 3 已完成：

- Workspace 的 views、active_index、pinned_indices 已私有化，并提供 active/view accessors。
- commands、editor、mouse、search、tabs 已移动到 dispatch 子模块。
- AppEffect 已包含 redraw、reshape、update_title、persist_workspace，并支持 merge。
- App::apply_effect 已存在。

尚未闭环：

- app_dispatch.rs 仍有大量 needs_redraw、request_redraw、invalidate_reshape 和 persistence 直调。
- execute_commands 和多个 AppAction 分支各自调用 apply_effect，无法保证一次动作链只应用一次。
- dispatch/tabs.rs 内部仍有自 apply 的路径。
- 设置切换仍在 handler 内重复 load/modify/save，并与 redraw/reshape 混杂。
- app_scroll、sidebar、popup、viewport 等 action helper 仍直接修改全局 effect 状态。
- lib.rs 仍以 pub mod 暴露约 40 个内部模块，而外部真实消费者只需要少数入口。

## 范围决策

采用“dispatch 范围内严格收口，生命周期保持自治”。

### 必须经过 AppEffect 的操作

在 AppAction 路由及其下游 handler 中，以下 follow-up effect 不得直接执行：

- 设置 needs_redraw 或调用 Window::request_redraw。
- invalidate_reshape。
- update_window_title。
- persist_workspace_state。
- 持久化普通编辑器 Settings。
- 根据 ViewMode 同步平台窗口 chrome。

### 允许直接执行的领域操作

AppEffect 不是通用消息总线。下列需要同步结果或用户交互的领域操作保留在 handler：

- 打开/保存文档。
- 文件选择和关闭确认对话框。
- 剪贴板读写。
- popup/sidebar/workspace/document 的内存状态修改。
- AppAction::SetCursor 在顶层 dispatch 边界直接调用窗口 cursor API。

handler 执行这些操作后，仍以 AppEffect 报告 redraw、reshape、title 或 persistence 等后续需求。

### 明确豁免

以下路径不强制改为 AppEffect：

- winit ApplicationHandler 生命周期。
- redraw_requested / about_to_wait 的帧调度。
- resize、DPI、焦点、光标闪烁和后台 reshape 回包。
- renderer 内部 cache 与 GPU 提交。

这些路径本身就是运行时 effect 边界。强行经 AppEffect 会把帧调度与用户动作语义混为一体。

## 方案比较

### 方案 A：可合并 flag effect，dispatch 范围严格收口

保留无分配、Copy 的 AppEffect，以布尔字段表达可合并的 follow-up effect。领域 handler 返回 effect，顶层 dispatch 合并后应用一次。

优点：

- 与现有实现方向一致，迁移成本可控。
- merge 交换、结合、幂等，适合批量命令。
- 不把同步 I/O 和对话框包装成复杂 command bus。
- 可以通过静态扫描守住边界。

缺点：

- 无法表达有序、带 payload 的任意平台命令。
- action-specific 同步操作仍需明确豁免。

### 方案 B：有序 EffectCommand 列表

handler 返回 Vec<EffectCommand>，逐条执行 Redraw、Persist、SetCursor、WindowChrome 等命令。

优点是表达力强、顺序完全显式；缺点是引入分配、payload 合并和冲突决策，明显超过当前问题所需。

### 方案 C：只清理 app_dispatch.rs

只替换当前文件里的直接 effect，不约束下游 helper 和 dispatch 子模块。

改动最少，但边界仍会通过 helper 泄漏，后续重构很容易重新出现嵌套 apply 和重复 redraw。

### 结论

采用方案 A。方案 B 仅在未来出现跨线程、有序平台命令编排需求时重新评估。

## AppEffect 契约

最终结构：

~~~rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppEffect {
    pub(crate) redraw: bool,
    pub(crate) reshape: bool,
    pub(crate) update_title: bool,
    pub(crate) persist_workspace: bool,
    pub(crate) persist_settings: bool,
    pub(crate) sync_window_chrome: bool,
}
~~~

语义：

- NONE：无 follow-up effect。
- REDRAW：请求下一帧。
- RESHAPE：失效异步 reshape generation，并隐含 redraw。
- UPDATE_TITLE：按当前 active view 更新标题，并隐含 redraw。
- PERSIST_WORKSPACE：保存 workspace/tab/sidebar 状态，不隐含 redraw。
- PERSIST_SETTINGS：保存当前逻辑 Settings，不隐含 redraw。
- SYNC_WINDOW_CHROME：按当前 ViewMode 同步平台 titlebar，并隐含 redraw。

merge 对全部 bool 做 OR。它必须满足：

- identity：x.merge(NONE) == x。
- idempotence：x.merge(x) == x。
- commutativity：x.merge(y) == y.merge(x)。
- associativity：不同分组得到相同结果。

AppEffect 不携带错误、文件路径、cursor icon、对话框请求或任意闭包。

## 唯一应用点与执行顺序

顶层入口：

~~~rust
pub(crate) fn dispatch(
    &mut self,
    action: AppAction,
    event_loop: &ActiveEventLoop,
) {
    let effect = self.reduce_action(action, event_loop);
    self.apply_effect(effect);
    self.update_ime_cursor_area();
}
~~~

reduce_action 及其调用的所有领域 handler 只能返回 AppEffect，不得调用 apply_effect。

execute_commands 改为返回合并结果：

~~~rust
pub(crate) fn execute_commands(
    &mut self,
    commands: Vec<AppCommand>,
    event_loop: &ActiveEventLoop,
) -> AppEffect
~~~

App::apply_effect 的固定顺序：

1. reshape：invalidate_reshape。
2. window chrome：按当前 ViewMode 同步平台 titlebar。
3. title：update_window_title。
4. settings persistence：persist_editor_settings。
5. workspace persistence：persist_workspace_state。
6. redraw：设置 needs_redraw，并在窗口存在时 request_redraw。

顺序理由：

- 先完成 cache/window/title 状态同步，下一帧看到一致状态。
- persistence 读取最终内存状态。
- redraw 最后发出，避免窗口收到请求时状态仍未收敛。

即使 persistence 失败，后续 persistence 与 redraw 仍继续执行。

## Handler 分层

### app_dispatch.rs

职责缩减为：

- 接收 AppAction。
- 将 action 映射到对应领域 handler。
- 对少数顶层平台 action 执行明确的同步操作，例如 SetCursor。
- 调用一次 apply_effect。
- 最后同步 IME cursor area。

不再包含大段 popup/sidebar/viewport/settings 业务实现。

### 现有 dispatch 模块

- dispatch/commands.rs：AppCommand 路由和批量 effect merge。
- dispatch/editor.rs：EditCommand 与文档编辑。
- dispatch/mouse.rs：editor mouse input/cursor moved。
- dispatch/search.rs：SearchBarAction。
- dispatch/tabs.rs：tab/workspace/file-open/close 流程。

所有公开给 crate 内部的 dispatch handler 均返回 AppEffect。dispatch/tabs.rs 不得自行 apply。

### 新增 dispatch/chrome.rs

承接：

- popup 打开、清除和 overflow menu。
- tab hover/水平滚动 UI。
- sidebar pin/resize/settings menu。
- view mode、theme mode、line number、word wrap、status bar 设置。

状态修改后返回 REDRAW、RESHAPE、PERSIST_SETTINGS、PERSIST_WORKSPACE、SYNC_WINDOW_CHROME 的组合。

### 新增 dispatch/viewport.rs

承接：

- HandleScroll。
- scrollbar drag / UpdateScrollTop。
- ScrollViewportBy。
- JumpToHeading。

这些 handler 只修改 viewport/preview/scrollbar 状态并返回 effect。需要 reshape 的 editor scroll 返回 RESHAPE；仅 preview/TOC 滚动返回 REDRAW。

### 下游 helper

从 dispatch 调用的 helper 不得通过 self.needs_redraw 或 apply_effect 暗中产生全局 effect。

两种合法返回形式：

- 行为天然属于全局 follow-up：返回 AppEffect。
- 纯领域 helper：返回 bool / WorkspaceEffect / 领域结果，由上层映射为 AppEffect。

不允许返回 AppEffect 后又在 helper 内执行同一 effect。

## Settings persistence

新增单一 persist_editor_settings()：

~~~rust
pub(crate) fn persist_editor_settings(&self) -> std::io::Result<()>
~~~

它加载现有 PersistedSettings，使用当前逻辑 Settings 覆盖以下字段后保存：

- view_mode。
- theme_mode。
- show_line_numbers。
- word_wrap。
- show_status_bar。
- font_family。
- font_size。
- line_height_ratio。
- tab_width。

窗口位置/尺寸和 sidebar width 不由该方法覆盖，继续由各自已有的几何/workspace persistence 路径管理。

设置 handler 的顺序：

1. 修改内存 Settings。
2. 关闭相关 popup/menu。
3. 返回 PERSIST_SETTINGS 以及必要的 REDRAW/RESHAPE/SYNC_WINDOW_CHROME。

保存失败不回滚内存设置。apply_effect 记录带上下文的错误后继续执行其他 effect，确保 UI 仍反映用户刚刚选择的状态。

## WorkspaceEffect 映射

WorkspaceEffect 只描述 workspace 领域结果；在 dispatch/tabs.rs 统一映射：

- ActiveTabChanged → RESHAPE + UPDATE_TITLE + PERSIST_WORKSPACE。
- LayoutChanged → REDRAW + PERSIST_WORKSPACE。
- None → NONE。

match 必须显式列出这三个现有 variant；未来新增 variant 时由编译错误强制补充映射，不得用 wildcard 静默降为 NONE。

## App 公共 API

crate root 的稳定公开面：

~~~rust
pub use app::App;
pub use app_event::AppEvent;
pub use cli::{CliArgs, parse_args};
pub use gpu::{GpuError, headless_init};
~~~

App 至少保留集成测试和 binary 使用的公共方法：

- App::new。
- App::handle_resize。
- winit::application::ApplicationHandler<AppEvent> 实现。

main.rs 改用 root re-export，不再访问 cli 子模块。

### 开发期外部支持

integration render test 和 Cargo benchmark 都作为外部 crate 编译。提供单一隐藏入口：

~~~rust
#[doc(hidden)]
pub mod dev_support {
    pub use crate::measure_adapter::MeasureFromShaper;
    pub use crate::document_view::DocumentView;
    pub use crate::snap_tree::{DisplayLineEntry, SnapTree};
}
~~~

render smoke test、bench 文件只通过 dev_support 引用这些类型。dev_support 不承诺稳定性，不能被生产代码使用。

### 内部模块

其余模块在 lib.rs 中声明为 mod 或 pub(crate) mod。内部类型可以保持 pub 以满足模块间访问，但不再因父模块 pub 而泄漏到 crate 外部。

最终 lib.rs 中允许的 pub mod 只有带 doc(hidden) 的 dev_support；稳定入口全部使用 pub use。

## 错误处理

- settings persistence：返回 std::io::Result，由 apply_effect 记录错误。
- workspace persistence：保持现有 best-effort 语义，内部对 workspace snapshot 与 pinned paths 分别记录错误。
- apply_effect 无论 settings persistence 是否失败都继续调用 workspace persistence 和 redraw。
- 文档 open/save：handler 保留现有同步错误处理，并根据成功结果返回 effect。
- 对话框取消：返回 NONE，不视为错误。
- 无窗口环境：窗口 chrome、title、cursor 和 request_redraw 安全跳过。
- 无 active document/view：对应 handler 返回 NONE。
- 未识别的领域 effect：穷尽 match，编译器驱动新增语义处理。

本项目不引入全局 error bus，不更改当前用户通知 UI。

## 测试设计

### AppEffect 单元测试

- 每个常量字段正确。
- RESHAPE、UPDATE_TITLE、SYNC_WINDOW_CHROME 是否隐含 redraw 符合契约。
- merge 的 identity、idempotence、commutativity、associativity。
- 所有新增字段都参与 merge。

### Handler 测试

- RequestRedraw 返回 REDRAW。
- editor/preview scroll 分别返回 RESHAPE/REDRAW。
- tab active change 返回 reshape/title/workspace persistence。
- view mode 返回 settings persistence/window chrome/redraw。
- word wrap 返回 settings persistence/reshape。
- sidebar resize end 返回 workspace persistence。
- 对话框取消和无 active view 返回 NONE。
- 调用 handler 后、apply_effect 前，needs_redraw 和 reshape generation 不被偷偷修改。

### Effect 应用测试

- 一组 merged effect 只应用一次。
- persistence 读取 mutation 后的最终状态。
- settings persistence 失败时仍执行 workspace persistence 与 redraw。
- 无窗口 App 应用 redraw/title/chrome 不 panic。

### 公共 API 测试

- main binary 只使用 root re-export。
- integration smoke tests 继续构造 App、调用 handle_resize/headless_init。
- render smoke test 使用 dev_support::MeasureFromShaper。
- benchmark 使用 dev_support。
- cargo check -p edit-plus-app --all-targets 通过。

### 静态边界验收

~~~bash
rg -n "apply_effect\(" crates/app/src/dispatch crates/app/src/app_dispatch.rs
rg -n "needs_redraw\s*=|request_redraw\(|invalidate_reshape\(|update_window_title\(|persist_workspace_state\(|settings_io::save" \
  crates/app/src/dispatch crates/app/src/app_dispatch.rs
rg -n "^pub mod " crates/app/src/lib.rs
~~~

预期：

- 第一条只命中 app_dispatch.rs 顶层唯一调用。
- 第二条无输出。
- 第三条只命中 dev_support。

## 迁移策略

1. 扩展 AppEffect 和 apply_effect，先补齐 merge/顺序测试。
2. 将 settings persistence 抽成单一 Result API。
3. 让 execute_commands、workspace effect 和 tabs handler 只返回 effect，删除嵌套 apply。
4. 提取 chrome action 并迁移 settings/sidebar/popup。
5. 提取 viewport action并迁移 scroll/scrollbar/heading。
6. 将 app_dispatch.rs 收缩为 route + single apply。
7. 收缩 lib.rs，并逐一迁移 main、integration tests、bench。
8. 加入静态扫描与 all-targets 验收。

每个实施任务最多修改 3 个文件；涉及更多文件时按 action domain 或 external consumer 分批。

## 完成定义

- Workspace 字段继续保持私有，外部无穿透访问。
- dispatch handler 全部返回 AppEffect 或纯领域结果。
- dispatch 调用链中只有 app_dispatch.rs 顶层调用一次 apply_effect。
- dispatch 范围内无直接 redraw、reshape、title 或 persistence。
- settings handler 不再重复 load/modify/save。
- AppEffect 的全部字段拥有明确 merge 和执行顺序。
- app_dispatch.rs 只负责路由、顶层同步 action、single apply 和 IME post-hook。
- lib.rs 不再公开内部模块；外部真实消费者均经 root re-export 或 dev_support。
- app lib tests、integration tests、all-targets check 与静态扫描通过。

## 边界情况

- 一个命令批次同时请求 reshape、title、两类 persistence 和 redraw。
- 同一 effect 被多个 handler 重复请求。
- settings persistence 与 workspace persistence 同时失败。
- dispatch 过程中 active tab 被关闭，后续 effect 从最终 active state 读取数据。
- 无窗口、无 GPU、无 active view 的测试环境。
- platform titlebar 仅在 macOS 有实际行为，其他平台安全 no-op。
- popup 已关闭或 sidebar rect 尚未布局时打开菜单。
- dev_support 类型变化时，生产公共 API 不受影响。

## 不在本设计范围

- 生命周期和 renderer 全量 AppEffect 化。
- 逻辑 Settings / 物理 UiMetrics 迁移。
- UI widget 输入、ThemeRegistry 和 ui crate public API。
- warning、clippy allow、测试 fixture 泄漏和 CI gate。
- 异步 persistence 或用户可见错误通知系统。
