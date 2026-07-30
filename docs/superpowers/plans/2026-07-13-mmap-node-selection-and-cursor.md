# mmap 节点选择与光标 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 mmap 节点遵循“首次点击选中、标题点击编辑、仅已选中非编辑节点可拖动”的交互，并反映相应鼠标光标。

**Architecture:** app 鼠标分发在按下前读取对象选区，并把该快照保存进画布拖动会话；仅快照表明节点早已选中时才进入插件拖动协议。事件层复用 `HitTestEditTarget` 的语义命中与当前选区，以 mmap 插件名为边界生成精确的光标图标，不扩展 UI 插件协议。

**Tech Stack:** Rust 2024、`winit::window::CursorIcon`、现有 `ViewPlugin` / `EditHitTarget` 语义命中协议、app 内联单元测试。

## Global Constraints

- 产品名为 textora，mmap 编辑器包名为 `textora-markdown`。
- 不修改 `crates/markdown/src/mmf/canvas.rs` 或 `test_data/sample.mmap.md` 的既有未提交改动。
- 不改变 UI/app 依赖层次，不让 `ui` 依赖 app 状态。
- 不使用 `.unwrap()`；使用清晰命名、提前返回，并保持 `cargo fmt`。
- 每次提交前运行编译检查；生产代码前必须先写并观察失败测试。

---

## 文件结构

- `crates/app/src/mouse.rs`：为 `CanvasDragSession` 保存“按下前已选中”这一不可变交互事实。
- `crates/app/src/dispatch/mouse.rs`：比较对象范围与按下前选区，限制拖动会话启动，并覆盖鼠标分发回归测试。
- `crates/app/src/app_tests.rs`：将 mmap 拖动集成夹具改为先选中、再拖动的两次点击流程。
- `crates/app/src/events.rs`：读取 mmap 语义命中，选择 `Text`、`Pointer`、`Move` 或 `Default` 光标，并覆盖事件测试。

### Task 1: 仅让已选中节点进入画布拖动

**Files:**
- Modify: `crates/app/src/mouse.rs:17-23`
- Modify: `crates/app/src/dispatch/mouse.rs:180-201, 468-485, 648-930`
- Modify: `crates/app/src/app_tests.rs:2546-2568`
- Test: `crates/app/src/dispatch/mouse.rs:757-930` and `crates/app/src/app_tests.rs:2574-2670`

**Interfaces:**
- Consumes: `EditHitTarget::SourceObject { source_range: Range<usize> }` 和 `DocumentView::cursor().selection_anchor`。
- Produces: `CanvasDragEligibility::{Enabled, Disabled}` on `CanvasDragSession`；mmap 未在按下前选中的节点保存为 `Disabled`，`dispatch_canvas_drag_moved` 不发送任何拖动阶段请求。

- [ ] **Step 1: 写入失败测试，表达首次点击不能拖动**

在 `CanvasDragTestState` 增加构造辅助，并添加以下测试。测试插件继续返回 `SourceObject { source_range: 1..2 }`，第一次按下前没有选区：

```rust
#[test]
fn canvas_drag_does_not_start_when_node_was_not_selected_before_press() {
    let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
        CanvasDragResponse::Ignore,
    )));
    let mut app = app_with_canvas_drag_plugin("abc", state.clone());
    let bounds = app.plugin_render_bounds();

    app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0, None);
    app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);

    assert!(state.borrow().requests.is_empty());
}
```

同时将现有 `canvas_drag_starts_after_threshold_and_applies_one_drop_transaction` 的准备改为先把文档选区设为 `1..2`，表示节点已选中；集成辅助 `drag_mmap_node` 先完成一次按下/释放选中，再执行第二次按下、移动、释放拖动。

- [ ] **Step 2: 运行测试，确认它因缺少选中资格约束而失败**

Run: `cargo test -p textora-app --lib canvas_drag_does_not_start_when_node_was_not_selected_before_press`

Expected: FAIL，断言 `requests.is_empty()` 失败，因为当前首次按下后移动会发送 `CanvasDragPhase::Start`。

- [ ] **Step 3: 实现按下前选中快照与启动守卫**

在 `CanvasDragSession` 新增类型驱动的资格状态：

```rust
pub(crate) enum CanvasDragEligibility {
    Enabled,
    Disabled,
}

pub eligibility: CanvasDragEligibility,
```

在处理 `SourceObject` 按下时、调用 `apply_edit_hit_target` 前比较当前选区与命中范围：

```rust
let selected_before_press = tab
    .doc
    .cursor()
    .selection_anchor
    .map(|anchor| {
        let cursor = tab.doc.cursor_offset().to_usize();
        anchor.min(cursor)..anchor.max(cursor)
    })
    .as_ref()
    == Some(&source_range);
```

当插件名为 `PLUGIN_MINDMAP` 且 `selected_before_press` 为 `false` 时创建 `Disabled` 状态，其余自绘插件保持 `Enabled`。在 `dispatch_canvas_drag_moved` 的会话读取分支中，在阈值判断前加入：

```rust
if session.eligibility == CanvasDragEligibility::Disabled {
    return AppEffect::NONE;
}
```

保留按下时的 `apply_edit_hit_target`，从而首次点击仍即时选中节点；标题文字继续走既有 `TextCaret` 分支，不创建会话。

- [ ] **Step 4: 运行定向测试，确认转绿**

Run: `cargo test -p textora-app --lib canvas_drag`

Expected: PASS，既有“已选中后拖动”测试和新的首次选择测试均通过。

- [ ] **Step 5: 格式化、编译并提交该独立阶段**

Run: `cargo fmt --check && cargo check -p textora-app`

Expected: PASS。

```bash
git add crates/app/src/mouse.rs crates/app/src/dispatch/mouse.rs
git commit -m "fix(app): require selected mmap node before drag"
```

### Task 2: 根据 mmap 语义命中设置光标

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs:405-423`
- Modify: `crates/app/src/events.rs:13-20, 245-282, 604-710`
- Test: `crates/app/src/events.rs:604-710`

**Interfaces:**
- Consumes: `App::query_plugin_edit_hit_target(px, py) -> Option<Option<EditHitTarget>>`（调整为 `pub(crate)`）与当前文档选择范围。
- Produces: 仅 mmap 自绘编辑器下的 `AppAction::SetCursor(CursorIcon)`：标题 `Text`、未选中卡片 `Pointer`、已选中卡片 `Move`、空白 `Default`。

- [ ] **Step 1: 写入失败测试，表达四种 mmap 光标**

在 `events.rs` 测试模块中加入能配置 `HitTestEditTarget` 和插件名为 `PLUGIN_MINDMAP` 的轻量测试插件。测试 `mmap_cursor_reflects_edit_and_drag_targets` 为四个场景调用 `handle_cursor_moved` 并断言最后一个 `SetCursor`：

```rust
assert_cursor_icon(
    handle_cursor_moved(&mut app, x, y),
    CursorIcon::Text,
);
assert_cursor_icon(
    handle_cursor_moved(&mut app, x, y),
    CursorIcon::Pointer,
);
assert_cursor_icon(
    handle_cursor_moved(&mut selected_app, x, y),
    CursorIcon::Move,
);
assert_cursor_icon(
    handle_cursor_moved(&mut app, x, y),
    CursorIcon::Default,
);
```

分别配置 `TextCaret`、未选中 `SourceObject`、选区与 `SourceObject` 相同的已选中节点、`ClearFocus`。测试辅助函数从 actions 的逆序中提取第一条 `SetCursor`，避免前置 widget 动作干扰。

- [ ] **Step 2: 运行测试，确认它因全局文本光标策略而失败**

Run: `cargo test -p textora-app --lib mmap_cursor`

Expected: FAIL，除标题外的 mmap 场景仍收到 `CursorIcon::Text`。

- [ ] **Step 3: 实现 mmap 专用光标解析**

将 `query_plugin_edit_hit_target` 改为 `pub(crate)`，供 `events.rs` 使用。新增一个仅在 `plugin.name() == PLUGIN_MINDMAP && plugin.handles_own_rendering() && plugin.allows_editing()` 时调用的事件层辅助函数。其逻辑为：

```rust
match app.query_plugin_edit_hit_target(px, py) {
    Some(Some(EditHitTarget::TextCaret { .. })) => Some(CursorIcon::Text),
    Some(Some(EditHitTarget::SourceObject { source_range })) => {
        Some(if active_selection_range(app).as_ref() == Some(&source_range) {
            CursorIcon::Move
        } else {
            CursorIcon::Pointer
        })
    }
    Some(Some(EditHitTarget::ClearFocus)) | Some(None) => Some(CursorIcon::Default),
    None => None,
}
```

其中 `active_selection_range` 作为事件模块内的私有函数，读取活动文档锚点与当前光标并归一化范围：

```rust
fn active_selection_range(app: &App) -> Option<std::ops::Range<usize>> {
    let tab = app.workspace.active_entry()?;
    let anchor = tab.doc.cursor().selection_anchor?.to_usize();
    let cursor = tab.doc.cursor_offset().to_usize();
    (anchor != cursor).then(|| anchor.min(cursor)..anchor.max(cursor))
}
```

不为通用插件添加状态。将 `handle_cursor_moved` 里现有 `allows_editing` 的文本光标分支替换为：优先使用该辅助函数返回的图标，返回 `None` 时原样回退到现有边界与文本光标规则。

- [ ] **Step 4: 运行定向测试，确认转绿**

Run: `cargo test -p textora-app --lib mmap_cursor`

Expected: PASS，四种场景分别得到 `Text`、`Pointer`、`Move`、`Default`。

- [ ] **Step 5: 回归验证、格式化并提交该独立阶段**

Run: `cargo fmt --check && cargo test -p textora-app --lib canvas_drag && cargo test -p textora-app --lib mmap_cursor && cargo check -p textora-app`

Expected: PASS。

```bash
git add crates/app/src/dispatch/mouse.rs crates/app/src/events.rs
git commit -m "fix(app): reflect mmap interaction in cursor"
```

## 最终验证

- [ ] 执行 `cargo test -p textora-app --lib`。
- [ ] 执行 `cargo check -p textora-app`。
- [ ] 检查 `git status --short`，确认仅保留用户原有的 `crates/markdown/src/mmf/canvas.rs` 与 `test_data/sample.mmap.md` 修改。
