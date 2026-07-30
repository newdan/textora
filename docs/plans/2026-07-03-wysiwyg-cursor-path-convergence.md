# WYSIWYG Cursor Path Convergence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Markdown WYSIWYG 编辑中的 cursor、selection、plugin sync 收敛到少数明确入口，消除“视觉光标在 A，真实编辑落点在 B”的复发路径。

**Architecture:** `DocumentView` 继续作为真实 byte cursor/selection 的唯一权威；Markdown 插件只负责视觉映射、增强编辑建议和渲染状态。app 层新增小型收敛 helper，把 `replace_range + insert_text + cursor_byte_after`、snapped cursor、selection plugin message 和 `AppEffect` 统一处理，Enter/Backspace/导航/鼠标路径逐步迁入。

**Tech Stack:** Rust workspace；`crates/app` 应用层；`crates/markdown` WYSIWYG plugin；测试使用现有 `cargo test -p textora-app --lib` 与 `cargo test -p textora-markdown --lib`。

## Global Constraints

- 全程保持 `ui` 与 `app` 解耦：禁止 `ui` 或 `crates/markdown` 依赖 `DocumentView`、`Workspace`、`Commands`。
- 每阶段最多改动 3 个源码文件；超过 3 个文件必须拆成下一阶段。
- 先写失败测试再改实现；同一症状超过两次修复失败，停止叠补丁并重新审视架构。
- 严禁硬编码无语义的 magic value；新 helper 命名必须精准自解释。
- Rust 不使用随意 `.unwrap()`；需要 panic 的测试代码使用 `.expect("...")` 说明理由。
- 每次实现后运行 `cargo fmt`；每阶段至少运行对应聚焦测试与 `cargo check -p textora-app`。
- 重大阶段完成后运行 `./scripts/verify.sh`。

---

## Current State

当前已添加诊断日志，默认不输出：

```bash
EDIT_PLUS_WYSIWYG_CURSOR_LOG=1 cargo run -p textora-app -- /path/to/file.md
```

日志标签：

- `[wysiwyg:augment]`: 插件返回的 `EditAugmentation`
- `[wysiwyg:cursor]`: `DocumentView` 当前 cursor/selection
- `[wysiwyg:effect]`: WYSIWYG dispatch 返回的 `AppEffect`
- `[wysiwyg:sync]`: app 与 plugin 的 source/cursor/selection sync 边界

已清理的干扰日志：

- `[delete_backward]`
- `[dv::move_vis]`

## Target Invariants

1. `DocumentView.cursor_offset()` 是真实编辑落点的唯一权威。
2. app 向 plugin 发送 cursor 时，必须发送 `DocumentView` snapped 后的 byte。
3. app 向 plugin 发送 selection 时，必须与 `DocumentView.selection_range()` 或当前操作的 explicit anchor/cursor 一致。
4. `EditAugmentation.replace_range = Some(range)` 表示插件要求 app 先把编辑目标定位到该 range。
5. `EditAugmentation.insert_text = Some("")` 表示“用空字符串替换目标”；当 `replace_range` 非空时必须删除该 range。
6. `replace_range = None` 且 `insert_text = Some("")` 表示纯 cursor move；如果 cursor 改变，返回 `AppEffect::REDRAW`。
7. `insert_text = None` 表示使用调用方 fallback command，例如 Enter fallback 为 `InsertNewline`，Backspace fallback 为 `Backspace`。
8. `sync_plugin_state()` 不应无条件把 stale plugin selection 反向写回 `DocumentView`。

## File Map

- Modify: `crates/app/src/dispatch/wysiwyg.rs`
  - 收敛 Enter/Backspace augmentation 应用逻辑。
  - 新增 snapped cursor/selection plugin 通知 helper。
  - 保留 `EDIT_PLUS_WYSIWYG_CURSOR_LOG` 诊断日志，直到手动复现通过。
- Modify: `crates/app/src/app_tests.rs`
  - 增加 app 层 WYSIWYG augmentation 与鼠标 snapped byte 回归测试。
  - 复用现有 `RecordingWysiwygPlugin`。
- Modify: `crates/app/src/dispatch/mouse.rs`
  - 让 WYSIWYG mouse hit-test 返回 snapped byte。
  - 改用统一 cursor/selection sync helper。
- Modify: `crates/app/src/app_renderer.rs`
  - 收紧 `sync_plugin_state()` 中 plugin selection 反拉策略。
- Modify: `crates/markdown/src/view.rs`
  - 修复 byte selection 映射失败时 visual selection endpoint 残留。
  - 澄清或修正 `CursorScreenPos(byte)` 参数语义。

---

## Task 1: Codify WYSIWYG Augmentation Semantics

**Files:**
- Modify: `crates/app/src/app_tests.rs`
- Read: `crates/app/src/dispatch/wysiwyg.rs`
- Read: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: existing `RecordingWysiwygPlugin`, `RecordingWysiwygState`, `App`, `DocumentView`, `DocItem`
- Produces: failing tests that define app-layer behavior before refactor

- [ ] **Step 1: Extend `RecordingWysiwygState` with an augmentation response**

Add fields near the existing WYSIWYG test state:

```rust
#[derive(Default)]
struct RecordingWysiwygState {
    source_text: String,
    generation: u32,
    cursor_byte: Option<usize>,
    sel_anchor_byte: Option<usize>,
    sel_cursor_byte: Option<usize>,
    hit_test_byte: Option<usize>,
    visual_move_result: Option<usize>,
    visual_move_query: Option<(usize, ui::plugin::MoveDirection, Option<f32>)>,
    cursor_rect: Option<(f32, f32, f32, f32)>,
    selection_range: Option<(usize, usize)>,
    augmentation: Option<ui::plugin::EditAugmentation>,
    preedit_text: String,
    preedit_cursor: Option<(usize, usize)>,
}
```

- [ ] **Step 2: Implement `augmenter()` in `RecordingWysiwygPlugin`**

Add this method inside `impl ui::plugin::ViewPlugin for RecordingWysiwygPlugin`:

```rust
fn augmenter(&self) -> &dyn ui::plugin::EditAugmenter {
    self
}
```

Then implement the augmenter trait:

```rust
impl ui::plugin::EditAugmenter for RecordingWysiwygPlugin {
    fn augment(&self, _ctx: &ui::plugin::AugmentContext<'_>) -> Option<ui::plugin::EditAugmentation> {
        self.state.borrow().augmentation.clone()
    }
}
```

- [ ] **Step 3: Write failing test for non-empty replace range plus empty insert**

Add test in the WYSIWYG source/cursor sync test section:

```rust
#[test]
fn wysiwyg_enter_empty_insert_deletes_nonempty_replace_range() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        augmentation: Some(ui::plugin::EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: Some(0..2),
            cursor_byte_after: 0,
        }),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["- ".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(2);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(RecordingWysiwygPlugin::new(state.clone())),
    ));
    let _ = app.workspace.switch_to(0);

    let effect = app.dispatch_wysiwyg_augmented_enter_for_test();

    assert!(effect.redraw, "deleting the list marker must redraw WYSIWYG content");
    let tab = app.workspace.active_entry().expect("active tab should exist");
    assert_eq!(tab.doc.buffer_len(), 0);
    assert_eq!(tab.doc.cursor_offset().to_usize(), 0);
    let recorded = state.borrow();
    assert_eq!(recorded.cursor_byte, Some(0));
    assert_eq!(recorded.sel_anchor_byte, None);
    assert_eq!(recorded.sel_cursor_byte, None);
}
```

This test references a test-only entry point that Task 2 will add:

```rust
#[cfg(test)]
pub(crate) fn dispatch_wysiwyg_augmented_enter_for_test(&mut self) -> AppEffect
```

- [ ] **Step 4: Write failing test for cursor-only augmentation returning redraw**

```rust
#[test]
fn wysiwyg_enter_cursor_only_augmentation_redraws_and_syncs_cursor() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        augmentation: Some(ui::plugin::EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: None,
            cursor_byte_after: 4,
        }),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["| A | B |".to_string()], 80, 10.0);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(RecordingWysiwygPlugin::new(state.clone())),
    ));
    let _ = app.workspace.switch_to(0);

    let effect = app.dispatch_wysiwyg_augmented_enter_for_test();

    assert!(effect.redraw, "pure WYSIWYG cursor movement must request redraw");
    let tab = app.workspace.active_entry().expect("active tab should exist");
    assert_eq!(tab.doc.cursor_offset().to_usize(), 4);
    assert_eq!(state.borrow().cursor_byte, Some(4));
}
```

- [ ] **Step 5: Run tests and confirm they fail for the intended reason**

Run:

```bash
cargo test -p textora-app --lib -- wysiwyg_enter_empty_insert_deletes_nonempty_replace_range wysiwyg_enter_cursor_only_augmentation_redraws_and_syncs_cursor
```

Expected before implementation:

- The tests fail to compile because `dispatch_wysiwyg_augmented_enter_for_test` does not exist; or
- After adding the test-only wrapper, the first test fails because the marker is not deleted; the second fails because effect is `AppEffect::NONE`.

Do not change production behavior in this task.

---

## Task 2: Centralize Augmentation Application

**Files:**
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
- Modify: `crates/app/src/app_tests.rs`

**Interfaces:**
- Consumes: failing tests from Task 1
- Produces:
  - `fn dispatch_wysiwyg_augmented_edit(&mut self, event_loop: Option<&ActiveEventLoop>, kind: AugmentKind, fallback: EditCommand) -> AppEffect`
  - `fn execute_augmentation_text_change(app: &mut App, augmented: &EditAugmentation, fallback: EditCommand, event_loop: Option<&ActiveEventLoop>) -> AppEffect`
  - `#[cfg(test)] pub(crate) fn dispatch_wysiwyg_augmented_enter_for_test(&mut self) -> AppEffect`
  - `#[cfg(test)] pub(crate) fn dispatch_wysiwyg_augmented_backspace_for_test(&mut self) -> AppEffect`

- [ ] **Step 1: Add a dispatch helper that accepts optional event loop**

Confirmed API: `crates/app/src/commands.rs::EditOutcome` exposes `dirty_lines: Option<Range<usize>>`; there is no `is_dirty()` method. In test-only paths, treat `dirty_lines.is_some()` as the redraw signal for text edits. Cursor-only movement is handled separately by the outer augmentation function.

Add in `crates/app/src/dispatch/wysiwyg.rs`:

```rust
fn dispatch_wysiwyg_command(
    app: &mut App,
    command: EditCommand,
    event_loop: Option<&ActiveEventLoop>,
) -> AppEffect {
    match event_loop {
        Some(event_loop) => app.dispatch_edit_command(command, event_loop),
        None => {
            let Some(tab) = app.workspace.active_entry_mut() else {
                return AppEffect::NONE;
            };
            let outcome = crate::commands::execute_edit_command_v2(&command, &mut tab.doc, &[]);
            if outcome.dirty_lines.is_some() { AppEffect::REDRAW } else { AppEffect::NONE }
        }
    }
}
```

- [ ] **Step 2: Add exact augmentation edit semantics**

Add helper in `crates/app/src/dispatch/wysiwyg.rs`:

```rust
/// Applies only the text-editing part of a WYSIWYG augmentation.
///
/// For `replace_range = Some(non_empty)` plus `insert_text = Some(non_empty)`,
/// deletion is intentionally consumed by `InsertText` through the selection
/// prepared by `position_document_for_wysiwyg_replace_range`; this helper does
/// not issue an explicit `DeleteRange` for that combination.
///
/// For `replace_range = Some(non_empty)` plus `insert_text = Some("")`, there
/// is no replacing insert command, so this helper must issue `DeleteRange`
/// explicitly.
fn execute_augmentation_text_change(
    app: &mut App,
    augmented: &EditAugmentation,
    fallback: EditCommand,
    event_loop: Option<&ActiveEventLoop>,
) -> AppEffect {
    let mut result = AppEffect::NONE;

    if let Some(range) = augmented.replace_range.as_ref()
        && !range.is_empty()
        && augmented.insert_text.as_deref() == Some("")
    {
        return result.merge(dispatch_wysiwyg_command(
            app,
            EditCommand::DeleteRange(range.clone()),
            event_loop,
        ));
    }

    match augmented.insert_text.as_ref() {
        Some(insert_text) if !insert_text.is_empty() => result.merge(dispatch_wysiwyg_command(
            app,
            EditCommand::InsertText(insert_text.clone()),
            event_loop,
        )),
        Some(_) => result,
        None => result.merge(dispatch_wysiwyg_command(app, fallback, event_loop)),
    }
}
```

Important: this helper only performs the edit. Cursor positioning, final cursor application, logging and plugin sync stay in the outer centralized function so the sequence is identical for Enter and Backspace.

- [ ] **Step 3: Replace duplicated Enter/Backspace bodies with one centralized function**

Add:

```rust
fn cursor_changed_after_augmentation(dv: &DocumentView, cursor_byte_after: usize) -> bool {
    dv.cursor_offset().to_usize() != cursor_byte_after
}

impl App {
    fn dispatch_wysiwyg_augmented_edit(
        &mut self,
        event_loop: Option<&ActiveEventLoop>,
        kind: AugmentKind,
        fallback: EditCommand,
    ) -> AppEffect {
        let current_byte = match self.workspace.active_doc() {
            Some(dv) => dv.cursor_offset().to_usize(),
            None => return dispatch_wysiwyg_command(self, fallback, event_loop),
        };

        let aug = self.wysiwyg_query_augment(current_byte, kind);
        log_wysiwyg_augmentation(kind, current_byte, aug.as_ref());
        self.wysiwyg_recursing = true;

        let mut result = AppEffect::NONE;
        if let Some(augmented) = aug {
            if let Some(range) = augmented.replace_range.clone()
                && let Some(dv) = self.workspace.active_doc_mut()
            {
                position_document_for_wysiwyg_replace_range(dv, range);
                log_wysiwyg_cursor_state("augment.after_replace_range", dv);
            }

            result = result.merge(execute_augmentation_text_change(
                self,
                &augmented,
                fallback,
                event_loop,
            ));

            if let Some(dv) = self.workspace.active_doc_mut()
                && cursor_changed_after_augmentation(dv, augmented.cursor_byte_after)
            {
                dv.cursor_move_to_offset(augmented.cursor_byte_after);
                result = result.merge(AppEffect::REDRAW);
            }
            if let Some(dv) = self.workspace.active_doc() {
                log_wysiwyg_cursor_state("augment.before_sync", dv);
            }
        } else {
            result = result.merge(dispatch_wysiwyg_command(self, fallback, event_loop));
        }

        self.wysiwyg_recursing = false;
        self.sync_plugin_state();
        if let Some(dv) = self.workspace.active_doc() {
            log_wysiwyg_cursor_state("augment.after_sync", dv);
        }

        result
    }
}
```

- [ ] **Step 4: Wire public Enter/Backspace functions through the centralized function**

Replace the bodies:

```rust
pub(crate) fn dispatch_wysiwyg_augmented_enter(
    &mut self,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    self.dispatch_wysiwyg_augmented_edit(
        Some(event_loop),
        AugmentKind::Enter,
        EditCommand::InsertNewline,
    )
}

pub(crate) fn dispatch_wysiwyg_augmented_backspace(
    &mut self,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    self.dispatch_wysiwyg_augmented_edit(
        Some(event_loop),
        AugmentKind::Backspace,
        EditCommand::Backspace,
    )
}
```

Add test-only wrappers:

```rust
#[cfg(test)]
impl App {
    pub(crate) fn dispatch_wysiwyg_augmented_enter_for_test(&mut self) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(None, AugmentKind::Enter, EditCommand::InsertNewline)
    }

    pub(crate) fn dispatch_wysiwyg_augmented_backspace_for_test(&mut self) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(None, AugmentKind::Backspace, EditCommand::Backspace)
    }
}
```

- [ ] **Step 5: Verify Task 1 tests now pass**

Run:

```bash
cargo test -p textora-app --lib -- wysiwyg_enter_empty_insert_deletes_nonempty_replace_range wysiwyg_enter_cursor_only_augmentation_redraws_and_syncs_cursor
```

Expected: both tests pass.

- [ ] **Step 6: Run focused WYSIWYG app tests**

Run:

```bash
cargo test -p textora-app --lib -- wysiwyg
```

Expected: all filtered WYSIWYG tests pass.

---

## Task 3: Centralize Snapped Cursor And Selection Plugin Sync

**Files:**
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/app/src/app_tests.rs`

**Interfaces:**
- Consumes: Task 2 centralized augmentation flow
- Produces:
  - `fn set_wysiwyg_cursor_and_selection(tab: &mut DocItem, requested_cursor_byte: usize, selection_anchor: Option<usize>) -> usize`
  - Mouse WYSIWYG hit-test returns snapped byte

- [ ] **Step 1: Add shared snapped cursor helper**

Place helper in `crates/app/src/dispatch/wysiwyg.rs` near `position_document_for_wysiwyg_replace_range`:

Confirmed type: `DocItem` is `crate::tab::DocItem`.

```rust
pub(crate) fn set_wysiwyg_cursor_and_selection(
    tab: &mut crate::tab::DocItem,
    requested_cursor_byte: usize,
    selection_anchor: Option<usize>,
) -> usize {
    tab.doc.cursor_move_to_offset(requested_cursor_byte);
    let snapped_cursor_byte = tab.doc.cursor_offset().to_usize();
    tab.doc.cursor_mut().selection_anchor = selection_anchor;
    tab.plugin
        .handle_message(PluginMessage::SetCursorByte(snapped_cursor_byte), &mut tab.doc);
    tab.plugin
        .handle_message(PluginMessage::SetSelAnchorByte(selection_anchor), &mut tab.doc);
    tab.plugin.handle_message(
        PluginMessage::SetSelCursorByte(selection_anchor.map(|_| snapped_cursor_byte)),
        &mut tab.doc,
    );
    snapped_cursor_byte
}
```

- [ ] **Step 2: Use helper in keyboard WYSIWYG navigation**

The helper only moves/snaps cursor and synchronizes plugin cursor/selection bytes. It must not touch `self.wysiwyg_preferred_x`. Keep preferred-x side effects in the callers:

- `dispatch_wysiwyg_navigation` must still query `CursorScreenPos(snapped_byte)` after the helper and update `self.wysiwyg_preferred_x = Some(x)`.
- `wysiwyg_navigate_to_doc_boundary` must still clear `self.wysiwyg_preferred_x = None`.
- `wysiwyg_page` must still clear `self.wysiwyg_preferred_x = None`.

In `dispatch_wysiwyg_navigation`, replace manual cursor/plugin message sequence with:

```rust
let snapped_byte = set_wysiwyg_cursor_and_selection(tab, new_byte, selection_anchor);
```

In `wysiwyg_navigate_to_doc_boundary`, replace manual sequence with:

```rust
let snapped = set_wysiwyg_cursor_and_selection(tab, byte, selection_anchor);
```

In `wysiwyg_page`, use:

```rust
let _snapped = set_wysiwyg_cursor_and_selection(tab, new_byte, None);
self.wysiwyg_preferred_x = None;
```

- [ ] **Step 3: Write failing page selection clearing test**

Add to `crates/app/src/app_tests.rs`:

```rust
#[test]
fn wysiwyg_page_down_clears_plugin_selection_bytes() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(5),
        sel_anchor_byte: Some(1),
        sel_cursor_byte: Some(4),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    doc.cursor_mut().selection_anchor = Some(1);
    doc.cursor_move_to_offset(4);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(RecordingWysiwygPlugin::new(state.clone())),
    ));
    let _ = app.workspace.switch_to(0);

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::PageDown);

    assert!(effect.redraw, "page navigation should redraw WYSIWYG content");
    let recorded = state.borrow();
    assert_eq!(recorded.cursor_byte, Some(5));
    assert_eq!(recorded.sel_anchor_byte, None);
    assert_eq!(recorded.sel_cursor_byte, None);
}
```

This locks the intentional behavior change: after paging, the plugin receives `SetSelAnchorByte(None)` and `SetSelCursorByte(None)`.

- [ ] **Step 4: Write failing mouse snapped byte test**

Add to `crates/app/src/app_tests.rs`:

```rust
#[test]
fn wysiwyg_single_click_uses_snapped_byte_for_selection_anchor() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let content = format!("x{emoji}y");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(3),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec![content], 80, 10.0);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(RecordingWysiwygPlugin::new(state.clone())),
    ));
    let _ = app.workspace.switch_to(0);

    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, 40.0, 24.0, None);

    let tab = app.workspace.active_entry().expect("active tab should exist");
    let snapped = tab.doc.cursor_offset().to_usize();
    assert_eq!(tab.doc.cursor().selection_anchor, Some(snapped));
    let recorded = state.borrow();
    assert_eq!(recorded.cursor_byte, Some(snapped));
    assert_eq!(recorded.sel_anchor_byte, Some(snapped));
    assert_eq!(recorded.sel_cursor_byte, Some(snapped));
}
```

- [ ] **Step 5: Fix `set_plugin_cursor_from_point` to return snapped final byte**

In `crates/app/src/dispatch/mouse.rs`, change the final phase:

```rust
let snapped_final = if final_byte != snapped_candidate {
    tab.doc.cursor_move_to_offset(final_byte);
    let snapped_final = tab.doc.cursor_offset().to_usize();
    tab.plugin.handle_message(PluginMessage::SetCursorByte(snapped_final), &mut tab.doc);
    snapped_final
} else {
    snapped_candidate
};

self.wysiwyg_preferred_x = None;
snapped_final
```

- [ ] **Step 6: Replace mouse selection message duplication where safe**

For single-click and drag paths, prefer the shared helper when setting cursor plus selection together. Preserve existing double-click word selection behavior until a separate test covers it.

- [ ] **Step 7: Verify mouse and navigation tests**

Run:

```bash
cargo test -p textora-app --lib -- wysiwyg_page_down_clears_plugin_selection_bytes wysiwyg_single_click_uses_snapped_byte_for_selection_anchor wysiwyg_navigation_sends_snapped_cursor_byte_to_plugin wysiwyg_extend_right_preserves_anchor_and_notifies_plugin_selection
```

Expected: all pass.

---

## Task 4: Harden `sync_plugin_state` Selection Direction

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_tests.rs`

**Interfaces:**
- Consumes: existing `sync_plugin_state_pulls_selection_range_into_document`
- Produces: explicit policy for when plugin selection may be pulled into `DocumentView`

- [ ] **Step 1: Add explicit pull gate function**

In `crates/app/src/app_renderer.rs`:

```rust
fn plugin_selection_pull_is_safe(
    mouse_is_down: bool,
    plugin_needs_source_update: bool,
) -> bool {
    !mouse_is_down && !plugin_needs_source_update
}
```

This function is only called after `tab.doc.selection_range()` has already returned `None`, so it intentionally does not take a `document_has_selection` parameter.

- [ ] **Step 2: Add tests for the gate**

Inside existing `app_renderer::tests`:

```rust
#[test]
fn plugin_selection_pull_gate_blocks_stale_plugin_source() {
    assert!(!super::plugin_selection_pull_is_safe(false, true));
}

#[test]
fn plugin_selection_pull_gate_allows_clean_idle_plugin_selection() {
    assert!(super::plugin_selection_pull_is_safe(false, false));
}

#[test]
fn plugin_selection_pull_gate_blocks_mouse_drag() {
    assert!(!super::plugin_selection_pull_is_safe(true, false));
}
```

- [ ] **Step 3: Use the gate in `sync_plugin_state`**

Replace the selection branch with:

```rust
if should_sync_selection {
    if let Some((start, end)) = tab.doc.selection_range() {
        // existing push-to-plugin branch unchanged
    } else if plugin_selection_pull_is_safe(self.mouse.is_down, needs_update) {
        // existing pull-from-plugin branch unchanged
    }
}
```

If this changes `sync_plugin_state_pulls_selection_range_into_document`, keep that test focused on the allowed case by constructing a clean plugin state where `NeedsSourceUpdate` returns false. If stale-source behavior needs coverage, add a separate test named `sync_plugin_state_does_not_pull_selection_when_plugin_needs_source_update`; do not add conditional assertions to the existing positive test.

- [ ] **Step 4: Verify sync tests**

Run:

```bash
cargo test -p textora-app --lib -- sync_plugin_state plugin_selection_pull_gate
```

Expected: all filtered tests pass.

---

## Task 5: Fix Markdown Plugin Internal Selection Consistency

**Files:**
- Modify: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: app-level byte selection messages
- Produces: no stale visual selection endpoint when byte-to-visual mapping fails

- [ ] **Step 1: Write failing markdown test for stale visual endpoint**

Add under `wysiwyg_tests` in `crates/markdown/src/view.rs`:

```rust
#[test]
fn set_selection_byte_clears_visual_endpoint_when_byte_cannot_map() {
    let mut view = make_view("hello");
    let mut doc = StubDoc::new("hello");
    let bounds = ui::core::geom::Rect::new(0.0, 0.0, 400.0, 300.0);
    let mut shaper = shaping::Shaper::new(14.0, "");

    view.handle_message(PluginMessage::UpdateSource {
        text: "hello".to_string(),
        generation: 1,
    }, &mut doc);
    let _ = view.render(&doc, bounds, &ui::Theme::dark(), &mut shaper, 1.0);

    view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut doc);
    view.handle_message(PluginMessage::SetSelCursorByte(Some(5)), &mut doc);
    assert!(matches!(
        view.query(PluginQuery::HasSelection, &doc),
        PluginResponse::Bool(true)
    ));

    view.handle_message(PluginMessage::SetSelCursorByte(Some(usize::MAX)), &mut doc);

    assert!(matches!(
        view.query(PluginQuery::SelCursor, &doc),
        PluginResponse::Position(None)
    ));
}
```

Use the exact existing `StubDoc`, `make_view`, `PluginMessage`, `PluginQuery`, and `PluginResponse` imports already present in the test module.

- [ ] **Step 2: Clear visual endpoints on mapping failure**

Change `PreviewEngine::set_sel_cursor_byte`:

```rust
pub fn set_sel_cursor_byte(&mut self, byte: Option<usize>) {
    if let Some(b) = byte {
        self.sel_cursor_byte = Some(b);
        self.sel.cursor = self
            .find_flat_and_grapheme_for_byte(b)
            .map(|(l, c)| ViewPos { flat_line_idx: l, grapheme_pos: c });
    } else {
        self.sel.cursor = None;
        self.sel_cursor_byte = None;
    }
}
```

Change `PreviewEngine::set_sel_anchor_byte` similarly:

```rust
pub fn set_sel_anchor_byte(&mut self, byte: Option<usize>) {
    if let Some(b) = byte {
        self.sel_anchor_byte = Some(b);
        self.sel.anchor = self
            .find_flat_and_grapheme_for_byte(b)
            .map(|(l, c)| ViewPos { flat_line_idx: l, grapheme_pos: c });
    } else {
        self.sel.anchor = None;
        self.sel_anchor_byte = None;
    }
}
```

- [ ] **Step 3: Verify markdown selection tests**

Run:

```bash
cargo test -p textora-markdown --lib -- set_selection_byte_clears_visual_endpoint_when_byte_cannot_map selection
```

Expected: all filtered tests pass.

---

## Task 6: Clarify `CursorScreenPos(byte)` Semantics

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/app/src/dispatch/wysiwyg.rs`

**Interfaces:**
- Consumes: current app navigation calling `SetCursorByte(snapped)` before `CursorScreenPos(snapped)`
- Produces: either parameter-honoring query or explicitly current-cursor-only query usage

- [ ] **Step 1: Add markdown test that exposes current semantics**

Add under `wysiwyg_tests`:

```rust
#[test]
fn cursor_screen_pos_query_uses_requested_byte() {
    let mut view = make_view("abc\ndef");
    let mut doc = StubDoc::new("abc\ndef");
    let bounds = ui::core::geom::Rect::new(0.0, 0.0, 400.0, 300.0);
    let mut shaper = shaping::Shaper::new(14.0, "");

    view.handle_message(PluginMessage::UpdateSource {
        text: "abc\ndef".to_string(),
        generation: 1,
    }, &mut doc);
    view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
    let _ = view.render(&doc, bounds, &ui::Theme::dark(), &mut shaper, 1.0);

    let first = match view.query(PluginQuery::CursorScreenPos(0), &doc) {
        PluginResponse::CursorScreenRect(Some(rect)) => rect,
        response => panic!("expected first cursor rect, got {response:?}"),
    };
    let second = match view.query(PluginQuery::CursorScreenPos(4), &doc) {
        PluginResponse::CursorScreenRect(Some(rect)) => rect,
        response => panic!("expected second cursor rect, got {response:?}"),
    };

    assert_ne!(first.1, second.1, "different requested bytes should resolve different rows");
}
```

- [ ] **Step 2: Implement requested-byte cursor rect without mutating edit context**

Extract pure helper in `PreviewEngine`:

```rust
fn cursor_screen_pos_for_byte(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
    let lazy = self.lazy.as_ref()?;
    if let Some(rect) = self.empty_source_line_cursor_screen_pos(cursor_byte) {
        return Some(rect);
    }
    let (flat_idx, visual_grapheme) = self.find_flat_and_grapheme_for_byte(cursor_byte)?;
    let fl = lazy.flat_lines.get(flat_idx)?;
    let x = fl.rect.x
        + crate::layout::grapheme_x(fl, visual_grapheme)
        + self.trailing_stripped_space_advance(flat_idx, cursor_byte);
    let cursor_height = fl.font_size.min(fl.rect.h);
    let text_baseline_y = fl.rect.y + fl.font_size - self.scroll_y;
    let cursor_y = text_baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;
    Some((x, cursor_y, 2.0, cursor_height))
}
```

Then make existing `cursor_screen_pos()` call this helper with `ctx.cursor_byte`.

- [ ] **Step 3: Use query parameter in common query**

Define the exact behavior:

- If `byte == edit_ctx.cursor_byte`, return `visual_cursor_screen_pos().or_else(|| cursor_screen_pos_for_byte(byte))`; this preserves IME preedit cursor rendering at the active editing point.
- If `byte != edit_ctx.cursor_byte`, return `cursor_screen_pos_for_byte(byte)`; requested-byte probes must not accidentally use the current preedit cursor.

Current app navigation calls `SetCursorByte(snapped_byte)` before querying `CursorScreenPos(snapped_byte)`, so the navigation path remains in the first branch and keeps preedit-aware behavior. Future callers that probe another byte get the second branch.

Change:

```rust
PluginQuery::CursorScreenPos(_) => Some(PluginResponse::CursorScreenRect(
    self.visual_cursor_screen_pos().or_else(|| self.cursor_screen_pos()),
)),
```

To:

```rust
PluginQuery::CursorScreenPos(byte) => Some(PluginResponse::CursorScreenRect(
    self
        .edit_ctx
        .as_ref()
        .filter(|ctx| ctx.cursor_byte == *byte)
        .and_then(|_| self.visual_cursor_screen_pos())
        .or_else(|| self.cursor_screen_pos_for_byte(*byte)),
)),
```

- [ ] **Step 4: Verify markdown cursor tests**

Run:

```bash
cargo test -p textora-markdown --lib -- cursor_screen_pos_query_uses_requested_byte cursor_screen_pos
```

Expected: all filtered tests pass.

---

## Task 7: Manual Reproduction And Log Retirement Decision

**Files:**
- Modify: `docs/manual_test_protocol.md`
- Optionally modify: `crates/app/src/dispatch/wysiwyg.rs`
- Optionally modify: `crates/app/src/app_renderer.rs`

**Interfaces:**
- Consumes: diagnostic logs and fixes from Tasks 1-6
- Produces: manual reproduction checklist and decision to keep or remove gated logs

- [ ] **Step 1: Add manual test protocol section**

Append to `docs/manual_test_protocol.md`:

````markdown
## Markdown WYSIWYG Cursor Convergence

Run with:

```bash
EDIT_PLUS_WYSIWYG_CURSOR_LOG=1 cargo run -p textora-app -- /tmp/wysiwyg-cursor.md
```

Cases:

1. Heading interior Enter: `# hello| world` inserts newline at heading line end, not old visual cursor byte.
2. Empty bullet Enter: `- |` deletes `- ` and cursor remains at byte 0.
3. Empty blockquote Enter: `> |` deletes `> ` and cursor remains at byte 0.
4. Table cell Enter: cursor moves to next cell and redraws immediately.
5. Emoji click: clicking inside `x👨‍👩‍👧y` uses snapped byte for cursor and selection anchor.
6. Drag selection after source update: stale plugin selection does not reappear after mouse release.

Expected logs:

- `[wysiwyg:augment]` reports the plugin augmentation.
- `[wysiwyg:cursor] ... after_sync` cursor equals the visible cursor target.
- `[wysiwyg:sync] pull_plugin_selection` appears only for intentional plugin-owned selection.
````

- [ ] **Step 2: Run full verification after manual pass**

Run:

```bash
cargo fmt
cargo test -p textora-app --lib
cargo test -p textora-markdown --lib
cargo check -p textora-app
```

Expected:

- `textora-app`: all tests pass, ignored count unchanged unless intentionally updated.
- `textora-markdown`: all tests pass.
- `cargo check`: exits 0.

- [ ] **Step 3: Decide log lifecycle**

Keep `EDIT_PLUS_WYSIWYG_CURSOR_LOG` if another reproduction still needs field logs. Remove it only when:

- All manual cases pass.
- No current issue requires field logs.
- The removal commit still passes the full verification commands in Step 2.

If keeping logs, ensure they remain gated and default-off.

---

## Final Verification

After all tasks are complete:

```bash
./scripts/verify.sh
```

Expected: script exits 0.

If `./scripts/verify.sh` fails due to environment-only constraints, record the exact failing command and rerun the relevant package-level commands above before reporting status.

## Rollback Points

- After Task 2, Enter/Backspace behavior is unified. If regression appears, revert only `crates/app/src/dispatch/wysiwyg.rs` and the new app tests from Tasks 1-2.
- After Task 3, mouse/navigation sync is unified. If regression appears, revert Task 3 without touching Task 2.
- After Task 4, selection pull behavior is stricter. If copy/drag selection regresses, revert Task 4 only.
- After Tasks 5-6, markdown plugin selection/cursor query semantics are stricter. If rendering-only cursor tests regress, revert markdown changes independently from app dispatch changes.

## Self-Review

- Spec coverage: plan covers original heading Enter mismatch, empty-list deletion gap, table-cell cursor-only redraw, mouse snapped byte, stale selection pull, markdown internal visual/byte selection drift, and `CursorScreenPos(byte)` semantics.
- Placeholder scan: no placeholder markers or broad unfinished instructions remain.
- Type consistency: helper names are stable across tasks: `dispatch_wysiwyg_augmented_edit`, `execute_augmentation_text_change`, `set_wysiwyg_cursor_and_selection`, `plugin_selection_pull_is_safe`, `cursor_screen_pos_for_byte`.
