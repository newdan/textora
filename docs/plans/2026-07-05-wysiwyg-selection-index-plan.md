# WYSIWYG Selection Index Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Markdown WYSIWYG selection smooth and predictable by keeping source-byte selection semantics while adding cached byte-to-visual-position projection for rendering and dragging.

**Architecture:** Document/edit state continues to use source byte ranges because Markdown markers, blank lines, and folded syntax must remain editable and copyable. Rendering and hit-testing use grapheme-based `ViewPos` positions. `LazyLayout` should own both projection directions: `ViewPos -> source byte` and `source byte -> ViewPos`, rebuilt together whenever flat lines are rebuilt.

**Tech Stack:** Rust 2024, `textora-markdown`, `textora-app`, existing `LazyLayout<S: BlockSource>`, `FlatLineSourceMap`, `SelectionState`, `PreviewEngine<S>`.

## Global Constraints

- 全程保持 `ui` 与 `app` 解耦；`ui` 不得依赖 `DocumentView`、`Workspace` 或 app 状态。
- WYSIWYG 编辑协议必须保留 source byte，保证复制、删除、undo、Markdown marker 编辑不丢源码语义。
- 视觉选区、高亮、光标矩形和命中测试必须使用 grapheme 边界，不能切开组合字符或 ZWJ emoji。
- 修改超过 3 个文件时拆阶段执行；每阶段结束至少运行对应 crate 测试。
- 所有 Rust 代码必须通过 `cargo fmt`。

---

## Current State

`LazyLayout::build_flat_lines()` already builds `flat_line_source_maps`, which provide fast `ViewPos -> source byte` lookup through `source_bytes_by_visual_grapheme`.

The missing side is `source byte -> ViewPos`. `PreviewEngine::find_flat_and_grapheme_for_byte()` currently scans all `flat_line_source_maps` linearly. When a byte is not an exact visual grapheme entry, such as folded blockquote marker bytes, it calls `source_line_at_byte()` (which returns `SourceLineAtByte { index, start, end }`) for every flat line's first/last byte to check overlap — producing O(flat_lines × graphemes_per_line) `source_line_at_byte` calls.

Additionally, `find_flat_and_grapheme_at_or_after_byte()` and `find_flat_and_grapheme_at_or_before_byte()` (used by `visual_range_for_byte_selection()`) also perform full linear scans with `.flat_map()` + `.filter()` + `.min_by_key()` / `.max_by_key()`.

Selection state is currently split:

- `sel_anchor_byte` / `sel_cursor_byte`: source-byte truth used for app/plugin sync and document operations.
- `sel.anchor` / `sel.cursor`: visual `ViewPos` projection used for selection highlights.

This split is correct in principle, but the projection should be cached and invalidated with layout, not recomputed by scanning on every drag update.

WYSIWYG drag path has already been made render-free: `dispatch_editor_cursor_moved()` uses the lightweight `hit_test_plugin_byte_from_point()` (no synchronous plugin render). The existing test `wysiwyg_drag_move_does_not_synchronously_render_plugin` in `app_tests.rs` already covers this.

## Target Design

Introduce a cached reverse projection in `LazyLayout`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceByteVisualPosition {
    pub source_byte: usize,
    pub flat_line_idx: usize,
    pub grapheme_pos: usize,
}
```

`LazyLayout<S>` should store:

```rust
pub source_byte_visual_positions: Vec<SourceByteVisualPosition>,
```

Build this vector immediately after `self.flat_line_source_maps = source_maps;` inside `build_flat_lines()` (L242 of `types.rs`). Keep it sorted by `(source_byte, flat_line_idx, grapheme_pos)`.

Lookup rules:

- Exact byte: binary search to find the first matching source byte, then apply the existing wrapped-line sentinel rule.
- Non-exact marker byte: compute `target_source_line` **once** via `source_line_at_byte()`, then expand left/right from the binary search insertion point checking overlap via `SourceLineAtByte { start, end }` range — no per-flat-line `source_line_at_byte()` calls.
- If the byte falls before all mapped bytes: use first mapped position.
- If the byte falls after all mapped bytes: use last mapped position.

The source byte remains the canonical selection range. The cached `ViewPos` is only a projection.

## Files

- Modify: `crates/markdown/src/layout/types.rs`
  - Add `SourceByteVisualPosition` struct.
  - Add `source_byte_visual_positions: Vec<SourceByteVisualPosition>` field to `LazyLayout<S>`.
  - Initialize to `Vec::new()` in `LazyLayout::new()`.
  - Populate reverse projection at end of `build_flat_lines()` (after L242).

- Modify: `crates/markdown/src/view.rs`
  - Replace `find_flat_and_grapheme_for_byte()` linear scan with binary search over `lazy.source_byte_visual_positions`.
  - Replace `find_flat_and_grapheme_at_or_after_byte()` linear scan with binary search.
  - Replace `find_flat_and_grapheme_at_or_before_byte()` linear scan with binary search.
  - Keep existing source-line fallback behavior, but limit to **one** `source_line_at_byte()` call per lookup.
  - Preserve `sel_anchor_byte` / `sel_cursor_byte` as canonical state.

## Task 1: Add Reverse Projection Cache

**Files:**
- Modify: `crates/markdown/src/layout/types.rs`

**Interfaces:**
- Produces: `SourceByteVisualPosition`
- Produces: `LazyLayout<S>::source_byte_visual_positions`
- Consumes: existing `LazyLayout<S>::flat_line_source_maps`

- [ ] **Step 1: Write the failing test**

Add a test in the existing `mod tests` block (near L2145):

```rust
#[test]
fn build_flat_lines_creates_source_byte_visual_positions() {
    let (src, doc) = make_doc("> quoted text\n\nparagraph");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 800.0, &core::document::StringDocView::new(src));

    assert!(
        lazy.source_byte_visual_positions.iter().any(|entry| entry.source_byte == 2),
        "reverse projection should include the first visible quoted text byte"
    );
    assert!(
        lazy.source_byte_visual_positions
            .windows(2)
            .all(|pair| (pair[0].source_byte, pair[0].flat_line_idx, pair[0].grapheme_pos)
                <= (pair[1].source_byte, pair[1].flat_line_idx, pair[1].grapheme_pos)),
        "reverse projection must be sorted for binary search"
    );
}
```

Note: `from_doc()` already calls `build_flat_lines()` internally, so no separate call needed.

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p textora-markdown --lib build_flat_lines_creates_source_byte_visual_positions
```

Expected: compile failure because `source_byte_visual_positions` does not exist.

- [ ] **Step 3: Add cache type and field**

Add to `types.rs` (near `FlatLineSourceMap` at L36):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceByteVisualPosition {
    pub source_byte: usize,
    pub flat_line_idx: usize,
    pub grapheme_pos: usize,
}
```

Add to `LazyLayout<S>` struct (after `flat_line_source_maps` at L99):

```rust
/// Reverse projection: source byte → (flat_line_idx, grapheme_pos).
/// Sorted by (source_byte, flat_line_idx, grapheme_pos) for binary search.
/// Built alongside flat_line_source_maps in build_flat_lines().
pub source_byte_visual_positions: Vec<SourceByteVisualPosition>,
```

Initialize to `Vec::new()` in `LazyLayout::new()` (around L865).

- [ ] **Step 4: Populate cache in `build_flat_lines()`**

After `self.flat_line_source_maps = source_maps;` (L242), add:

```rust
let mut reverse_positions = Vec::new();
for source_map in &self.flat_line_source_maps {
    for (grapheme_pos, &source_byte) in
        source_map.source_bytes_by_visual_grapheme.iter().enumerate()
    {
        reverse_positions.push(SourceByteVisualPosition {
            source_byte,
            flat_line_idx: source_map.flat_idx,
            grapheme_pos,
        });
    }
}
reverse_positions.sort_by_key(|entry| (entry.source_byte, entry.flat_line_idx, entry.grapheme_pos));
reverse_positions.dedup_by_key(|entry| (entry.source_byte, entry.flat_line_idx, entry.grapheme_pos));
self.source_byte_visual_positions = reverse_positions;
```

- [ ] **Step 5: Run test to verify it passes**

Run:

```bash
cargo test -p textora-markdown --lib build_flat_lines_creates_source_byte_visual_positions
```

Expected: PASS.

## Task 2: Replace Linear Byte Lookup With Reverse Projection

**Files:**
- Modify: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `LazyLayout<S>::source_byte_visual_positions`
- Produces: faster `PreviewEngine<S>::find_flat_and_grapheme_for_byte(byte) -> Option<(usize, usize)>`
- Produces: faster `find_flat_and_grapheme_at_or_after_byte(byte)` and `find_flat_and_grapheme_at_or_before_byte(byte)`

- [ ] **Step 1: Verify existing performance-regression test infrastructure**

The test counter `SOURCE_LINE_AT_BYTE_CALLS` (AtomicUsize at L79) and helpers `reset_source_line_at_byte_call_count()` / `source_line_at_byte_call_count()` already exist. Add or locate a test in `view.rs` that covers blockquote marker byte lookup overhead:

```rust
#[test]
fn blockquote_marker_byte_selection_highlight_uses_bounded_source_line_lookups() {
    // Build a document with 180+ paragraphs and two blockquotes at indices 40 and 140.
    let mut source = String::new();
    for index in 0..180 {
        source.push_str(&format!("paragraph {index}\n\n"));
        if index == 40 || index == 140 {
            source.push_str(&format!("> quoted block {index}\n\n"));
        }
    }
    let first_quote = source.find("> quoted block 40").expect("fixture should contain quote");
    let second_quote = source.find("> quoted block 140").expect("fixture should contain quote");

    // Build the PreviewEngine with full layout.
    let mut engine = PreviewEngine::new();
    // ... set up engine with source, render once to build layout ...

    reset_source_line_at_byte_call_count();
    engine.set_sel_anchor_byte(Some(first_quote));
    engine.set_sel_cursor_byte(Some(second_quote));
    let _highlights = engine.selection_highlights([0.1, 0.2, 0.3, 1.0]);

    assert!(
        source_line_at_byte_call_count() <= 4,
        "blockquote marker byte fallback should not rescan source lines per mapped byte; calls={}",
        source_line_at_byte_call_count()
    );
}
```

Note: exact setup may need `MarkdownEditorView` wrapper or `PreviewEngine` test helpers matching the existing test patterns in `view.rs`.

- [ ] **Step 2: Run test to verify it fails on old lookup**

Run:

```bash
cargo test -p textora-markdown --lib blockquote_marker_byte_selection_highlight_uses_bounded_source_line_lookups
```

Expected on old code: FAIL with many `source_line_at_byte` calls due to per-flat-line overlap check.

- [ ] **Step 3: Implement binary-search exact match in `find_flat_and_grapheme_for_byte()`**

Replace the exact-match loop (L1297-L1313) with:

```rust
let positions = &lazy.source_byte_visual_positions;
let exact_start = positions.partition_point(|entry| entry.source_byte < byte);
let exact_end = positions.partition_point(|entry| entry.source_byte <= byte);
for entry in &positions[exact_start..exact_end] {
    let is_wrapped_sentinel = lazy
        .flat_line_source_maps
        .get(entry.flat_line_idx)
        .is_some_and(|source_map| {
            entry.grapheme_pos + 1 == source_map.source_bytes_by_visual_grapheme.len()
                && lazy
                    .flat_line_source_maps
                    .get(entry.flat_line_idx + 1)
                    .and_then(|next| next.source_bytes_by_visual_grapheme.first())
                    == Some(&byte)
        });
    if !is_wrapped_sentinel {
        return Some((entry.flat_line_idx, entry.grapheme_pos));
    }
}
```

- [ ] **Step 4: Implement nearest fallback using source-line bounds**

Replace the nearest-neighbor loop (L1314-L1339) with:

```rust
let target_source_line =
    self.edit_source.as_ref().and_then(|source| source_line_at_byte(source, byte));

let mut best: Option<(usize, usize, usize)> = None;  // (flat_idx, grapheme_pos, abs_diff)
let insertion_point = positions.partition_point(|entry| entry.source_byte < byte);

// Expand left and right from insertion_point, checking overlap with target source line.
for &direction in &[-1i64, 1i64] {
    let mut idx = if direction < 0 {
        insertion_point.wrapping_sub(1)
    } else {
        insertion_point
    };
    loop {
        if idx >= positions.len() { break; }
        let entry = &positions[idx];
        let dist = entry.source_byte.abs_diff(byte);
        if let Some((_, _, best_dist)) = best {
            if dist > best_dist { break; }
        }
        let overlaps = target_source_line.is_none_or(|line| {
            entry.source_byte >= line.start && entry.source_byte <= line.end
        });
        if overlaps && best.is_none_or(|(_, _, d)| dist < d) {
            best = Some((entry.flat_line_idx, entry.grapheme_pos, dist));
        }
        if direction < 0 {
            idx = idx.wrapping_sub(1);
        } else {
            idx += 1;
        }
    }
}
best.map(|(fi, vc, _)| (fi, vc))
```

Key difference from old code: the `source_line_at_byte()` call happens **once** for the target byte, and candidates are checked by comparing `entry.source_byte` against the target line's `start..end` range — no per-candidate `source_line_at_byte()`.

- [ ] **Step 5: Optimize `find_flat_and_grapheme_at_or_after_byte()` and `at_or_before_byte()`**

Replace the current `.flat_map().filter().min_by_key()` chains (L1359-L1387) with binary searches:

```rust
fn find_flat_and_grapheme_at_or_after_byte(&self, byte: usize) -> Option<(usize, usize)> {
    let lazy = self.lazy.as_ref()?;
    let positions = &lazy.source_byte_visual_positions;
    let idx = positions.partition_point(|entry| entry.source_byte < byte);
    positions.get(idx).map(|entry| (entry.flat_line_idx, entry.grapheme_pos))
}

fn find_flat_and_grapheme_at_or_before_byte(&self, byte: usize) -> Option<(usize, usize)> {
    let lazy = self.lazy.as_ref()?;
    let positions = &lazy.source_byte_visual_positions;
    let idx = positions.partition_point(|entry| entry.source_byte <= byte);
    if idx == 0 { return None; }
    positions.get(idx - 1).map(|entry| (entry.flat_line_idx, entry.grapheme_pos))
}
```

- [ ] **Step 6: Run target tests**

Run:

```bash
cargo test -p textora-markdown --lib blockquote_marker_byte_selection_highlight_uses_bounded_source_line_lookups
cargo test -p textora-markdown --lib blockquote
cargo test -p textora-markdown --lib find_flat_and_grapheme
```

Expected: all PASS.

## Task 3: Final Verification

**Files:**
- No new production files expected.

- [ ] **Step 1: Format**

Run:

```bash
cargo fmt
```

Expected: exit 0.

- [ ] **Step 2: Markdown full lib test**

Run:

```bash
cargo test -p textora-markdown --lib
```

Expected: all tests pass.

- [ ] **Step 3: App full lib test**

Run:

```bash
cargo test -p textora-app --lib
```

Expected: all tests pass, ignored tests unchanged.

- [ ] **Step 4: Optional full verification for major refactor**

Run when the reverse index touches more than the files listed above or changes public behavior:

```bash
./scripts/verify.sh
```

Expected: exit 0.

## Acceptance Criteria

- Setting selection endpoints across two blockquote regions does not scan source lines per mapped byte.
- `find_flat_and_grapheme_for_byte()` exact match is O(log N) via binary search instead of O(N).
- `find_flat_and_grapheme_at_or_after_byte()` and `at_or_before_byte()` are O(log N) via binary search.
- `source byte` remains canonical for edit/copy/delete.
- `ViewPos { flat_line_idx, grapheme_pos }` remains canonical for visual highlight/cursor projection.
- Existing blockquote, grapheme, emoji, trailing blank line, and WYSIWYG drag tests pass.

## Notes

- Do not replace source byte selection with grapheme-only state. Grapheme positions are layout-dependent and cannot represent folded Markdown marker bytes reliably.
- Do not move WYSIWYG selection state into `ui`; keep `ui::plugin` as pure protocol types.
- The reverse index belongs in `LazyLayout` because it is derived from flat lines and must be rebuilt with layout.
- WYSIWYG drag path is already render-free (Task 3 from original plan has been completed). The existing test `wysiwyg_drag_move_does_not_synchronously_render_plugin` in `app_tests.rs` validates this.
