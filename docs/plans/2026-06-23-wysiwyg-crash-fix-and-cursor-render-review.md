# Code Review: `codex/wysiwyg-crash-fix-and-cursor-render`

## Overview

This branch implements the WYSIWYG markdown editor foundation — source span mapping, EditContext-driven span unfolding, visual navigation, smart Enter/Backspace, cursor rendering, IME positioning, and app-layer dispatch following the "拦截→篡改→放行" pattern. 22 commits, ~2000 lines added across 20 files.

---

## 1. Architecture & Design Quality

**Strengths:**

- **Clean plugin separation.** `plugin.rs` additions (`MoveDirection`, `AugmentKind`, `EditAugmentation`, new `PluginQuery`/`PluginResponse` variants, `is_wysiwyg()`) are well-typed, self-documenting, and follow the existing message/query/response pattern exactly.

- **`dispatch/wysiwyg.rs` is a model of clarity.** The 206-line file has a clear module doc, clean method boundaries (`dispatch_wysiwyg_navigation`, `dispatch_wysiwyg_augmented_enter`, `dispatch_wysiwyg_augmented_backspace`), and the Phase 1/2/3 immutable-then-mutable borrow pattern is well-structured.

- **Recursive fall-through is correct.** Smart Enter rewrites `InsertNewline` → `InsertText("\n- ")` then recurses via `dispatch_edit_command`, ensuring `execute_edit_command_v2` produces `Outcome` → cache invalidation. The `wysiwyg_recursing` flag prevents infinite recursion. This is the right design.

- **`query_common`/`handle_message_common` extraction** into `PreviewEngine` eliminates ~80 lines of duplicated match arms across `MarkdownView` and `MarkdownEditorView`. Good DRY refactor.

- **`catch_unwind` on `window_event`** is correctly motivated — macOS objc2 callbacks abort on unwind in Rust 2024 edition. The `#[cfg(debug_assertions)]` guard on the error log prevents release binary bloat.

- **`edit.rs`** has excellent test coverage — 16 tests covering boundary conditions (inclusive end, empty line, multi-span, cursor-at-boundary, inline code).

---

## 2. Issues & Recommendations

### 2.1 🔴 Critical: Potential mismatch in `byte_in_line` computation for wrapped lines

In `byte_from_flat_line_and_char()` (view.rs), after computing `adjusted` char_offset (accounting for preceding wrapped segments), you use `block.text_lines[line_idx]` to map char→byte:

```rust
let full_line_text = &block.text_lines[line_idx];
let byte_in_line = full_line_text
    .char_indices()
    .nth(adjusted)
    .map(|(i, _)| i)
    .unwrap_or(full_line_text.len());
```

This looks correct for folded lines, but when a span is **expanded** (source text with markers replacing folded text), `full_line_text` is still the folded doc line. The `adjusted` char offset may exceed `full_line_text.len()` when markers are added, because the expanded line has more characters. The code handles the `unwrap_or` case (returns `full_line_text.len()`), but then the subsequent span-expansion logic runs:

```rust
if let Some(ctx) = self.edit_ctx.as_ref()
    && let Some(spans) = block.text_styles.get(line_idx)
{
    for span in spans {
        if !crate::edit::cursor_in_span(span, ctx.cursor_byte) {
            continue;
        }
        // ...attempts to map expanded offset back to source byte
    }
}
```

This fallback logic maps `byte_in_line` within the expanded span to source bytes, which serves as a correction. However, if `byte_in_line == full_line_text.len()` (overflow), the correction might not be reachable. This would manifest as an incorrect byte mapping when clicking at the right edge of an expanded span on a wrapped line segment.

**Recommendation:** Add a unit test specifically for `byte_from_flat_line_and_char` with a wrapped, expanded line scenario.

### 2.2 🟡 Important: `MoveWordLeft` / `MoveWordRight` map to simple char navigation, not word boundaries

```rust
// wysiwyg.rs
EditCommand::MoveWordLeft => MoveDirection::Left,   // FIXME: word-aware later
EditCommand::MoveWordRight => MoveDirection::Right,  // FIXME: word-aware later
```

The `FIXME` comments acknowledge this. The current behavior means Ctrl+Left/Right in WYSIWYG mode jumps one character at a time instead of one word, which is a UX regression compared to the standard editor.

**Recommendation:** Either:
- (a) Fall through to standard `execute_edit_command_v2` for `MoveWordLeft`/`MoveWordRight` (since word boundaries don't depend on markdown layout), or
- (b) Implement word-aware navigation in `visual_move` by using `word_at_pos` query results.

Option (a) is simpler and correct for MVP.

### 2.3 🟡 Important: Smart Backspace is a no-op hook

```rust
// wysiwyg.rs
if let Some(augmented) = aug
    && augmented.delete_range.is_some()
{
    // TODO: convert range-delete to selection + Backspace in a
    // future iteration. For now, fall through to standard Backspace.
    self.dispatch_edit_command(EditCommand::Backspace, event_loop)
} else {
    self.dispatch_edit_command(EditCommand::Backspace, event_loop)
}
```

Both branches do the same thing. The `if`/`else` is dead weight — it adds a `wysiwyg_query_augment` call that always returns `None` (because `augment_edit` returns `None` for `Backspace`). This burns a PluginQuery for every backspace without any benefit.

**Recommendation:** Either remove the AugmentEdit(Backspace) query path entirely for MVP, or gate it behind a feature flag so the query cost doesn't affect every keystroke. The same applies at the editor.rs dispatch gate:

```rust
EditCommand::Backspace => {
    return self.dispatch_wysiwyg_augmented_backspace(event_loop);
}
```

Consider removing this gate until Backspace augmentation is actually implemented.

### 2.4 🟡 Important: `line_source_byte_start` O(n²) complexity

```rust
fn line_source_byte_start(&self, block: &BlockNode, line_idx: usize) -> Option<usize> {
    let mut byte_offset = block.source_range.start;
    for (i, line_text) in block.text_lines.iter().enumerate() {
        if i == line_idx {
            return Some(byte_offset);
        }
        // ... walks spans computing source length per line
        byte_offset += src_len + 1; // +1 for newline
    }
    None
}
```

This walks every line up to `line_idx` every time it's called. It's called from `char_offset_from_byte` and `byte_from_flat_line_and_char`, which are called on every cursor movement. For large documents with many blocks, this could add up.

**Recommendation:** Precompute a `Vec<usize>` of per-line source byte offsets during `build_flat_lines()` and store it on `LazyLayout` or return it alongside `flat_lines`. Then this becomes O(1).

### 2.5 🟢 Minor: `flat_line` prefixed with `_` but used

In `byte_from_flat_line_and_char`:

```rust
let _flat_line = lazy.flat_lines.get(flat_line_idx)?;
```

The `_` prefix suppresses the unused warning, but the `?` operator is the only reason it's there (bounds check). This is technically correct but slightly confusing to read.

**Recommendation:** Replace with an explicit bounds check or comment:

```rust
// Bounds check only — flat_line data is accessed below via block_line_map
let _ = lazy.flat_lines.get(flat_line_idx)?;
```

### 2.6 🟢 Minor: `page_up`/`page_down` for WYSIWYG uses standard methods

```rust
fn wysiwyg_page(&mut self, direction: isize) -> AppEffect {
    if direction < 0 {
        tab.doc.page_up(line_height);
    } else {
        tab.doc.page_down(line_height);
    }
```

This delegates to `DocumentView::page_up`/`page_down`, which use the standard line-based scrolling. These methods may not account for the markdown layout's content height. Consider using `PluginQuery::ContentHeight` and `PluginMessage::Scroll` for more accurate paging in the markdown viewport.

### 2.7 🟢 Minor: Comment references old name

In `plugin.rs`, the `is_wysiwyg()` comment says:

```rust
/// the host queries [`PluginQuery::AugmentEdit`] before dispatching
/// edit commands, and uses [`PluginQuery::VisualMove`] for arrow-key navigation.
```

This is fine, but the VisualMove path is now only for arrows — mouse clicks use `HitTestByte`. The doc comment could be slightly more complete.

---

## 3. Rendering Path Correctness

The change from `!allows_editing()` to `!allows_editing() || is_wysiwyg()` in `app_renderer.rs` is applied consistently across all four gates:

1. Source update from document buffer (line 413)
2. Plugin render + selection highlights (line 456/492)
3. IME cursor area (app_window.rs)

This is correct — WYSIWYG views need the same "preview rendering path" (plugin-driven DrawList) as read-only views, while still passing the `allows_editing()` gate in dispatch.

The cursor rendering block (app_renderer.rs:573-615) is well-implemented:
- Queries `CursorScreenPos` for accurate position
- Uses `compute_cursor_phase` for standard blink timing
- Renders as a 2-pixel-wide quad with theme cursor color
- Correctly applies 10% top/bottom margins on the line height

---

## 4. Test Coverage

| Module | Tests | Quality |
|--------|-------|---------|
| `edit.rs` | 16 | Excellent — boundaries, multi-span, empty, expanded |
| `builder.rs` (source_range) | 6 | Good — bold/italic/code/plain/paragraph/heading |
| `plugin.rs` | 3 | Good — defaults tested |
| `wysiwyg.rs` | 0 | **Missing** — navigation, augmentation dispatch untested |
| `view.rs` (byte mapping) | 0 | **Missing** — the most complex logic is untested |

**Critical gaps:**
- `char_offset_from_byte` / `byte_from_flat_line_and_char` / `line_source_byte_start` — the core byte↔screen mapping has no unit tests
- `visual_move` — no tests for Left/Right/Up/Down/LineStart/LineEnd
- `dispatch_wysiwyg_navigation` / `dispatch_wysiwyg_augmented_enter` — no integration tests

---

## 5. Commit Quality

The 22 commits are well-structured and tell a coherent story:
- Task 1 (spans) → Task 2 (plugins) → Task 3 (engine) → Task 4 (view) → Task 5 (dispatch) → multiple review/fix rounds
- Fix commits (1d324a05, a587698a, 7abbf72d) address specific review findings with clear commit messages
- The `catch_unwind` commit (715e2ec1) correctly identifies the macOS ObjC FFI abort risk

One concern: commit `433e24db` ("回车栈溢出 + 光标显示") changes `shows_cursor()` from `false` to `true`, which contradicts the spec decision that the plugin renders its own cursor via `CursorScreenPos`. The app_renderer.rs cursor rendering block handles cursor display, so `shows_cursor()` should remain `false`. This might be a vestigial change.

---

## 7. 2026-06-25 Follow-up Resolution

- `materialize_text()` is now wired into layout through `LayoutCtx`, so cursor span expansion affects actual `FlatLine` text.
- WYSIWYG cursor mapping uses materialized source maps instead of ad hoc byte arithmetic.
- Cursor movement no longer marks the full Markdown engine as source-dirty.
- WYSIWYG keyboard movement is routed through `dispatch_wysiwyg_navigation()`.
- Cursor blink ownership is explicit through plugin wakeup and cursor visibility messages.

---

## 6. Summary

This branch is **approvable with minor fixes**. The architecture is sound, the "intercept→augment→relay" pattern is correctly implemented, and the rendering integration is complete. The main risks are:

1. **Untested byte mapping** (2.1, 2.4) — the core piece with most edge cases
2. **Dead Backspace augmentation path** (2.3) — burns cycles on every backspace
3. **Missing word navigation** (2.2) — UX regression for Ctrl+Left/Right
4. **Possible `shows_cursor()` regression** (Section 5) — may cause double cursor rendering
