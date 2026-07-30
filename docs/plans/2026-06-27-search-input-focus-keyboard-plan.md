# Search Input Focus and Keyboard Routing Modification Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复搜索输入框焦点、光标、全选/复制/粘贴、Delete 与方向键事件路由，保证输入框获得焦点后，除非用户明确点击正文区，否则键盘焦点与可见光标始终属于输入框。

**Architecture:** 以 `UiShell.keyboard_focus` 作为键盘事件的单一事实来源，搜索输入框可见性只表示面板显示，不再隐式代表输入焦点。`app` 层负责 winit 事件到 `WidgetAction` 的路由，`ui::widgets::SearchBarWidget` 与 `TextBox` 只处理纯输入状态和纯数据回调，继续保持 `ui` 不依赖 `app` 状态结构。

**Tech Stack:** Rust, winit 0.30, `crates/app` event dispatch, `crates/ui` widgets, `cargo test`, `./scripts/verify.sh`。

## Global Constraints

- 全程保持 `ui` 与 `app` 跨层解耦：`ui` 只接收纯数据 snapshot 和 widget 事件，不访问 `DocumentView` / `Workspace` / app 状态。
- 遇 Bug 先补失败测试，再修复根因；不要叠加“如果失败再 fallback 到正文”的防御性补丁。
- 涉及超过 3 个文件时按任务拆分提交；每次提交前必须 `cargo fmt` 与编译通过。
- 重大修改最终运行 `./scripts/verify.sh`。
- 命名必须自解释，避免 `data/info/temp/res/flag` 等宽泛名称。

---

## Current Evidence

已排查的关键路径：

- `crates/app/src/app_lifecycle.rs`
  - `WindowEvent::KeyboardInput` 只有在 `ui_shell.keyboard_focus == SEARCH_BAR` 时才转发给 `ui_shell.forward_key()`。
  - 若搜索框有焦点但 `winit_key_to_keycode()` 返回 `None`，当前逻辑仍会 `return`，导致按键被吞掉但输入框没有响应。
  - `is_search_bar_whitelist()` 会把部分 Cmd 快捷键放回正文/全局命令路径。
- `crates/app/src/events.rs`
  - 点击搜索栏时会设置 `keyboard_focus = Some(SEARCH_BAR)`。
  - 非 widget 消费的左键按下会清空搜索焦点，并继续向正文发送 `EditorMouseInput`。
- `crates/ui/src/widgets/search_bar.rs`
  - `SearchBarWidget::on_event()` 已经能把 `KeyDown`、IME、鼠标点击转给当前输入框。
  - 当 `TextBox::on_key()` 消费事件但没有产生搜索动作时，会返回 `WidgetAction::Consumed`，这点是正确的。
- `crates/ui/src/widgets/text_box.rs`
  - 已处理普通输入、Backspace、Left/Right、Home/End、Enter、Escape、Cmd+A/C/X/V。
  - 尚未处理 `Delete`。
  - `cmd` 当前被用于单词级左右移动；macOS 常见语义应区分 `Option+Left/Right` 单词移动与 `Cmd+Left/Right` 行首/行尾。
- `crates/app/src/dispatch/editor.rs`
  - 仍有旧路径：只要 `search_state.panel_visible`，`InsertChar` / `Backspace` / `InsertNewline` 会直接改搜索 query。
  - 这把“面板可见”和“输入焦点”混在一起，是焦点错位的高风险来源。
- `crates/app/src/app_renderer.rs`
  - 会读取 `SearchState.cursor_byte_pos` 并调用 `ui_shell.update_search_cursor_x()`，但 `SearchBarWidget::set_input()` 没有使用 `SearchBarSnapshot.cursor_x` 来设置 TextBox 光标。
  - 当前 TextBox 内部 cursor 才是实际绘制光标的位置，app 层的 `cursor_byte_pos` 基本是失效状态。

## Required Behavior

- 输入框获得焦点后，普通字符、IME commit、Backspace、Delete、Left/Right、Home/End、Option+Arrow、Cmd+Arrow、Cmd+A/C/X/V 都优先由输入框处理。
- 搜索 query 被 Backspace、Delete、Cut、Select-All-then-type、Select-All-then-Backspace 清空后，搜索输入框仍保持焦点并显示输入框光标。
- 只有点击正文编辑区域的 `MouseDown` 才把焦点切回正文。
- 点击搜索栏按钮、搜索栏空白处、标签栏、侧边栏、滚动条、TOC、弹窗等 chrome 控件，不应让正文获得输入焦点。
- 输入框未处理但应被输入框拥有的键也必须被消费，不能穿透到正文移动或编辑正文光标。
- 全局级快捷键保留：保存、关闭标签、切换/打开查找、撤销重做策略按任务 3 明确处理。

## Root-Cause Hypothesis

主要问题不是 `TextBox` 单点绘制，而是焦点与键盘路由没有形成闭环：

1. `keyboard_focus` 是路由入口，但其生命周期没有被显式建模为“Editor vs SearchBar”。
2. `winit_key_to_keycode()` 对 Cmd 字符快捷键依赖 `event.text`，在 `event.text` 是控制字符或为空时可能得不到 `KeyCode::Char`，导致 Cmd+A/C/V/X 在搜索焦点下被吞掉。
3. `TextBox` 缺少 `Delete` 与更完整的 macOS 光标移动语义。
4. `dispatch/editor.rs` 中按 `panel_visible` 修改搜索 query 的旧路径仍然存在，会继续制造“面板可见但焦点不在输入框”的状态分叉。
5. 搜索框 app 层 `cursor_byte_pos` 与 UI 层 TextBox cursor 是双光标模型，实际只有后者生效，容易让后续修复误改错误状态。

---

## Task 1: 补齐焦点与键盘路由失败测试

**Files:**

- Modify/Test: `crates/app/src/app_lifecycle.rs`
- Modify/Test: `crates/app/src/events.rs`
- Modify/Test: `crates/ui/src/widgets/search_bar.rs`
- Modify/Test: `crates/ui/src/widgets/text_box.rs`

**Interfaces:**

- Consumes: `winit_key_to_keycode()`, `is_search_bar_whitelist()`, `UiShell::forward_key()`, `SearchBarWidget::on_event()`, `TextBox::on_key()`
- Produces: 能稳定复现输入框焦点、清空、快捷键、Delete 与方向键行为的测试集合

- [ ] 在 `text_box.rs` 增加失败测试：
  - `delete_removes_char_after_cursor`
  - `delete_removes_selection`
  - `alt_left_right_moves_by_word`
  - `cmd_left_right_moves_to_edges`
  - `up_down_are_consumed_without_mutating_text`
- [ ] 在 `search_bar.rs` 增加失败测试：
  - `cmd_a_selects_find_box_text`
  - `cmd_c_copies_find_box_selection`
  - `cmd_v_pastes_into_focused_find_box`
  - `delete_after_select_all_keeps_find_focus`
  - `replace_box_receives_clipboard_shortcuts_when_focused`
- [ ] 在 `app_lifecycle.rs` 增加失败测试：
  - `keycode_cmd_char_falls_back_to_logical_character_when_text_is_control`
  - `keycode_cmd_char_falls_back_to_logical_character_when_text_is_none`
  - `search_focus_does_not_whitelist_cmd_a_c_x_v`
  - `search_focus_routes_delete_to_widget`
- [ ] 在 `events.rs` 增加失败测试：
  - `left_click_editor_clears_search_keyboard_focus`
  - `left_click_non_editor_chrome_does_not_clear_search_keyboard_focus`
  - `clearing_search_query_does_not_clear_keyboard_focus`
- [ ] 运行聚焦测试，确认新增用例先失败：
  - `cargo test -p edit-plus-ui --lib text_box search_bar`
  - `cargo test -p edit-plus-app --lib app_lifecycle events`

## Task 2: 显式建模键盘焦点生命周期

**Files:**

- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Test: `crates/app/src/events.rs`

**Interfaces:**

- Consumes: 当前 `UiShell.keyboard_focus: Option<WidgetId>`
- Produces: `UiShell::focus_search_bar()`, `UiShell::focus_editor()`, `UiShell::keyboard_focus_target()`，或等价的强类型焦点 API

- [ ] 引入强类型焦点模型，建议：

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyboardFocusTarget {
    Editor,
    Widget(ui::core::widget::WidgetId),
}
```

- [ ] 将 `None` 表示正文焦点的隐式语义替换为 `KeyboardFocusTarget::Editor`。
- [ ] 搜索面板打开时调用 `focus_search_bar()`；搜索面板关闭时调用 `focus_editor()`。
- [ ] 点击搜索栏输入框、搜索栏按钮、切换 replace/find 时保持或设置 `focus_search_bar()`。
- [ ] 只有 `MouseDown` 命中 `ui_shell.editor_rect()` 时才调用 `focus_editor()`。
- [ ] 点击 chrome 或 overlay 后，如果事件被 widget 消费，不改变正文焦点。
- [ ] `update_ime_cursor_area()` 改为依据焦点选择 IME 区域：
  - `Widget(SEARCH_BAR)`：使用 `search_ime_cursor_rect()`。
  - `Editor` 且 WYSIWYG：使用 WYSIWYG 光标。
  - `Editor` 且普通正文：使用正文光标。
- [ ] 保持 `crates/ui` 纯数据边界，不把 app 焦点 enum 暴露给 ui widget。

## Task 3: 修正搜索焦点下的键盘快捷键归属

**Files:**

- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/ui/src/widgets/text_box.rs`
- Modify: `crates/ui/src/widgets/search_bar.rs`
- Test: `crates/app/src/app_lifecycle.rs`
- Test: `crates/ui/src/widgets/text_box.rs`
- Test: `crates/ui/src/widgets/search_bar.rs`

**Interfaces:**

- Consumes: `winit_key_to_keycode()`, `is_search_bar_whitelist()`, `TextBox::on_key()`
- Produces: 搜索焦点下输入框优先处理的完整键盘矩阵

- [ ] 修改 `winit_key_to_keycode()`：当 `event.text` 为空或是非 Tab 控制字符，且 `logical_key` 是 `Key::Character` 时，回退到 logical character 的首个字符。
- [ ] 调整 `is_search_bar_whitelist()`：
  - 不允许 `Cmd+A/C/X/V` 进入全局/正文路径。
  - 保留 `Cmd+F` / `Cmd+Shift+F` 用于查找与替换面板控制。
  - 保留 `Cmd+S` / `Cmd+Shift+S` / `Cmd+W` 等应用级命令。
  - 对 `Cmd+Z` / `Cmd+Shift+Z` 做明确决策：当前 TextBox 无 undo 栈，先保持全局 undo/redo，但测试锁定不影响输入框焦点。
- [ ] 搜索焦点下，如果 keycode 可转换，必须调用 `forward_key()`。
- [ ] 搜索焦点下，如果 keycode 不可转换，也必须消费事件并请求 redraw，不允许穿透到正文。
- [ ] `TextBox::on_key()` 增加：
  - `Delete` 删除光标后的一个 UTF-8 字符或当前 selection。
  - `Alt+Left/Right` 单词级移动。
  - `Cmd+Left/Right` 行首/行尾。
  - `Up/Down/PageUp/PageDown` 在单行输入框中消费但不改变文本，防止正文光标移动。
- [ ] `SearchBarWidget::on_event()` 在目标输入框没有业务 action 时继续返回 `Consumed`，保持“不穿透正文”的契约。

## Task 4: 移除正文 dispatch 中的搜索输入旧路径

**Files:**

- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/dispatch/commands.rs`
- Test: `crates/app/src/dispatch/editor.rs`

**Interfaces:**

- Consumes: `EditCommand`, `SearchState.panel_visible`, `UiShell` 焦点 API
- Produces: 搜索 query 只能由 SearchBar widget action 或显式查找命令改变

- [ ] 删除或改造 `dispatch_edit_command()` 中仅凭 `search_state.panel_visible` 处理 `InsertChar` / `Backspace` / `InsertNewline` 的分支。
- [ ] `Find` / `FindReplace` / `FindNext` / `FindPrev` 仍由编辑器命令处理，但只负责打开面板、切换模式、跳转匹配，不直接冒充输入框编辑。
- [ ] `ToggleFind` 打开已可见搜索面板时只设置搜索焦点，不清空 query，不移动正文光标。
- [ ] `Escape` 的策略：
  - 搜索框有焦点：交给 `SearchBarWidget`，执行 `DismissOrClear`。
  - 正文有焦点且搜索面板可见：可保留当前关闭/清空搜索面板行为。
- [ ] 补测试确认：搜索面板可见但焦点在正文时，普通字符编辑正文，不进入 query。
- [ ] 补测试确认：搜索框有焦点时，普通字符不进入正文。

## Task 5: 收敛搜索输入光标状态

**Files:**

- Modify: `crates/app/src/search_state.rs`
- Modify: `crates/app/src/app_search.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/ui/src/widgets/search_bar.rs`
- Test: `crates/app/src/search_state.rs`
- Test: `crates/ui/src/widgets/search_bar.rs`

**Interfaces:**

- Consumes: `SearchState.cursor_byte_pos`, `SearchBarSnapshot.cursor_x`, `TextBox::cursor_byte()`
- Produces: 一个清晰的输入框光标来源，避免 app 层和 widget 层双写

- [ ] 决定并实施单一来源：
  - 推荐短期方案：TextBox 内部持有 cursor/selection，`SearchState.cursor_byte_pos` 不参与绘制，删除 app_renderer 的 `cursor_x` 注入死路径。
  - 若需要跨 Dock rebuild 保留光标，则给 `SearchBarSnapshot` 增加纯数据 `find_cursor_byte` / `replace_cursor_byte` / selection 字段，并由 `SearchBarAction` 回传 cursor 变化。
- [ ] 短期推荐方案下：
  - 删除 `SearchBarSnapshot.cursor_x` 与 `UiShell::update_search_cursor_x()`。
  - 删除 `app_renderer.rs` 中 `search_query_for_measure` 与 `cursor_byte_pos` 的测量逻辑。
  - 保留 `SearchState.cursor_byte_pos` 仅当有明确使用；若无使用，删除字段和相关测试。
- [ ] 如果保留 `SearchState.cursor_byte_pos`，必须让 `SearchBarWidget::set_input()` 真正消费它，并测试 query 清空后 cursor 留在 0 且焦点仍在输入框。
- [ ] 不在 `ui` 层引入 `SearchState`。

## Task 6: 光标绘制与 IME 验证

**Files:**

- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Test: `crates/app/src/app_window.rs`
- Test: `crates/app/src/app_renderer.rs`

**Interfaces:**

- Consumes: `update_ime_cursor_area()`, `cursor_vertices()`, `compute_next_wake_time()` / `about_to_wait`
- Produces: 输入框焦点下正文光标弱化或隐藏，输入框光标稳定闪烁，IME 候选窗跟随输入框

- [ ] `cursor_vertices()` 使用强类型焦点判断正文光标状态：
  - 搜索框焦点时正文光标弱化或不绘制。
  - 正文焦点时正常绘制正文光标。
- [ ] TextBox blink 不应依赖正文 cursor blink 的 redraw 跳过逻辑。
- [ ] 搜索框焦点下仍要安排下一次 blink wakeup，或明确让 TextBox 使用当前全局 blink phase 并由 app 请求 redraw。
- [ ] `update_ime_cursor_area()` 在搜索焦点下即使 query 为空也使用输入框 ime rect。
- [ ] IME preedit 与 commit 只发给当前焦点目标；搜索框焦点时不写正文，正文焦点时不写搜索框。

## Verification Matrix

- [ ] `cargo fmt --all`
- [ ] `cargo test -p edit-plus-ui --lib text_box search_bar`
- [ ] `cargo test -p edit-plus-app --lib app_lifecycle events search_state`
- [ ] `cargo test -p edit-plus-app --lib dispatch`
- [ ] `cargo check -p edit-plus-app`
- [ ] `./scripts/verify.sh`

## Manual Test Protocol

- [ ] 打开搜索框，输入 `abc`，Backspace 三次清空；输入框光标仍显示在 placeholder 位置，正文光标不恢复为主焦点。
- [ ] 搜索框为空时按 Backspace / Delete / Left / Right / Up / Down；正文内容和正文光标不变化。
- [ ] 输入 `hello world`，测试 Option+Left/Right 按单词移动，Cmd+Left/Right 到首尾。
- [ ] Cmd+A 后输入 `x`，query 变为 `x`，正文不被全选或替换。
- [ ] Cmd+A、Cmd+C，剪贴板得到 query 文本。
- [ ] Cmd+A、Cmd+X，query 清空但焦点仍在输入框。
- [ ] Cmd+V 粘贴到 find 输入框；打开 replace 模式后聚焦 replace 输入框，Cmd+V 粘贴到 replace 输入框。
- [ ] 点击搜索栏按钮、标签栏、侧边栏、滚动条，搜索框焦点不被正文夺走。
- [ ] 点击正文区，搜索框焦点释放，正文光标与输入焦点回到正文。
- [ ] 中文 IME 在搜索框内 preedit/commit，候选窗跟随搜索框光标；query 清空后仍跟随输入框。

## Self-Review

- 无待定占位符；每个任务都有明确文件、接口和测试。
- 方案遵守 `ui` / `app` 分层红线：`ui` 只定义纯 widget 行为和 snapshot，不读取 app 状态。
- 根因指向焦点路由、winit key 转换、TextBox 键盘覆盖、旧搜索输入路径和双光标状态，没有用防御性 fallback 掩盖问题。
- 任务超过 3 个文件，已拆成焦点模型、键盘归属、旧路径清理、光标状态、IME/绘制验证五个实现阶段。
