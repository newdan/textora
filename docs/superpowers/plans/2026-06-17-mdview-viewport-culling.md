# MdView Viewport Culling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add viewport culling to markdown preview rendering — only visible blocks/lines generate DrawCmds, replacing full-tree traversal.

**Architecture:** Build a flat `BlockRowIndex` during layout that maps each visual row to a tree path + line index. Render uses binary search to find the first visible row, then iterates only visible rows, skipping blocks already rendered. Upgrade scroll from raw `f32` to `Viewport` + `ScrollAnchor`, implementing `LineMap` on `BlockRowIndex`.

**Tech Stack:** Rust, edit-plus-markdown crate, edit-plus-ui crate (Viewport, ScrollAnchor, LineMap)

---

### Task 1: BlockRowIndex + build_row_index in layout.rs

**Files:**
- Modify: `crates/markdown/Cargo.toml`
- Modify: `crates/markdown/src/layout.rs`

- [ ] **Step 1: Add smallvec dependency to markdown crate**

In `crates/markdown/Cargo.toml`, add under `[dependencies]`:

```toml
smallvec = { workspace = true }
```

- [ ] **Step 2: Add BlockRowIndex struct and build function**

In `crates/markdown/src/layout.rs`, after the `LaidOutDoc` struct definition (line 15), add:

```rust
use smallvec::SmallVec;

/// Flat row index: maps visual row number → block tree path + line offset.
/// Built once after layout, enables O(log n) viewport lookup.
#[derive(Clone, Debug)]
pub struct BlockRowIndex {
    /// Cumulative y position at each row (for binary search).
    pub row_y_starts: Vec<f32>,
    /// Tree path to the block owning this row.
    /// path[0] = top-level block index, path[1..] = child indices into nested containers.
    pub row_block_paths: Vec<SmallVec<[usize; 4]>>,
    /// Index into the target block's lines[] vec.
    pub row_line_idxs: Vec<usize>,
}

impl BlockRowIndex {
    pub fn len(&self) -> usize {
        self.row_y_starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.row_y_starts.is_empty()
    }

    /// Binary search: find the first row whose y >= scroll_y.
    /// Returns 0 if scroll_y is before all rows.
    pub fn first_visible_row(&self, scroll_y: f32) -> usize {
        match self.row_y_starts.binary_search_by(|&y| {
            if y < scroll_y { std::cmp::Ordering::Less }
            else { std::cmp::Ordering::Greater }
        }) {
            Ok(i) | Err(i) => i.min(self.row_y_starts.len().saturating_sub(1)),
        }
    }
}

/// Recursively walk laid-out blocks, collecting rows into the index.
fn build_row_index(blocks: &[LaidOutBlock], path_prefix: &[usize]) -> BlockRowIndex {
    let mut row_y_starts = Vec::new();
    let mut row_block_paths = Vec::new();
    let mut row_line_idxs = Vec::new();

    for (bi, block) in blocks.iter().enumerate() {
        let mut path = SmallVec::from_slice(path_prefix);
        path.push(bi);

        match &block.kind {
            LaidOutBlockKind::Text { lines } | LaidOutBlockKind::CodeBlock { lines, .. } => {
                for (li, line) in lines.iter().enumerate() {
                    row_y_starts.push(line.rect.y);
                    row_block_paths.push(path.clone());
                    row_line_idxs.push(li);
                }
            }
            LaidOutBlockKind::BlockQuote { blocks } => {
                let child = build_row_index(blocks, &path);
                row_y_starts.extend(child.row_y_starts);
                row_block_paths.extend(child.row_block_paths);
                row_line_idxs.extend(child.row_line_idxs);
            }
            LaidOutBlockKind::ListItem { blocks, lines, .. } => {
                // Lines of the item itself
                for (li, line) in lines.iter().enumerate() {
                    row_y_starts.push(line.rect.y);
                    row_block_paths.push(path.clone());
                    row_line_idxs.push(li);
                }
                // Lines from nested children
                let child = build_row_index(blocks, &path);
                row_y_starts.extend(child.row_y_starts);
                row_block_paths.extend(child.row_block_paths);
                row_line_idxs.extend(child.row_line_idxs);
            }
            LaidOutBlockKind::Table { header, rows, .. } => {
                // Header lines
                for cell_lines in header.iter() {
                    for (li, line) in cell_lines.iter().enumerate() {
                        row_y_starts.push(line.rect.y);
                        row_block_paths.push(path.clone());
                        row_line_idxs.push(li);
                    }
                }
                // Body row lines
                for row in rows.iter() {
                    for cell_lines in row.iter() {
                        for (li, line) in cell_lines.iter().enumerate() {
                            row_y_starts.push(line.rect.y);
                            row_block_paths.push(path.clone());
                            row_line_idxs.push(li);
                        }
                    }
                }
            }
            LaidOutBlockKind::HorizontalRule => {
                // Horizontal rules are structural, no text lines. Skip.
            }
        }
    }

    BlockRowIndex { row_y_starts, row_block_paths, row_line_idxs }
}
```

- [ ] **Step 3: Integrate row index build into layout_doc_with_shaper**

In `crates/markdown/src/layout.rs`, modify `layout_doc_with_shaper` (line 349):

```rust
pub fn layout_doc_with_shaper(
    doc: &MarkdownDoc,
    style: &MarkdownStyle,
    viewport_w: f32,
    shaper: Option<&mut Shaper>,
) -> LaidOutDoc {
    let mut ctx = LayoutCtx::new(style, viewport_w, shaper);

    for block in &doc.blocks {
        layout_block(block, &mut ctx);
    }

    let blocks = ctx.output;
    let row_index = build_row_index(&blocks, &[]);

    LaidOutDoc {
        blocks,
        total_height: ctx.y,
        row_index,
    }
}
```

Note: `ctx.output` is moved into `blocks` before calling `build_row_index` because `build_row_index` borrows `&[LaidOutBlock]`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p edit-plus-markdown 2>&1`
Expected: compiles cleanly. Fix any errors.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/Cargo.toml crates/markdown/src/layout.rs
git commit -m "feat(markdown): add BlockRowIndex with build_row_index"
```

---

### Task 2: Viewport culling in render.rs

**Files:**
- Modify: `crates/markdown/src/render.rs`

- [ ] **Step 1: Add resolve_path helper and rewrite render_doc_with_offset**

Replace the existing `render_doc_with_offset` function (lines 26-45) and add `resolve_block` helper after it:

```rust
/// Render with pixel offset (used to position preview inside editor content area).
/// Only renders blocks and lines that intersect the viewport.
pub fn render_doc_with_offset(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
) {
    dl.cmds.push(DrawCmd::PushClip(Rect::new(offset_x, offset_y, f32::MAX, viewport_h)));

    let last_y = scroll_y + viewport_h;
    let first_row = doc.row_index.first_visible_row(scroll_y);
    let mut rendered_paths: Vec<SmallVec<[usize; 4]>> = Vec::new();

    let overscan = 3usize;
    let start_row = first_row.saturating_sub(overscan);
    let row_count = doc.row_index.len();

    for ri in start_row..row_count {
        let row_y = doc.row_index.row_y_starts[ri];
        if row_y > last_y {
            break;
        }

        let path = &doc.row_index.row_block_paths[ri];
        // Skip if this block was already rendered (multiple rows share a block)
        if rendered_paths.iter().any(|p| p.as_slice() == path.as_slice()) {
            continue;
        }

        let block = resolve_block(&doc.blocks, path);
        let block_bottom = block.rect.y + block.rect.h;
        if block_bottom < scroll_y || block.rect.y > last_y {
            continue;
        }

        render_block_with_offset(block, style, dl, scroll_y, viewport_h, offset_x, offset_y, shaper.as_deref_mut());
        rendered_paths.push(path.clone());
    }

    dl.cmds.push(DrawCmd::PopClip);
}

/// Resolve a block tree path to a &LaidOutBlock reference.
fn resolve_block<'a>(blocks: &'a [LaidOutBlock], path: &[usize]) -> &'a LaidOutBlock {
    let mut block = &blocks[path[0]];
    for &child_idx in &path[1..] {
        block = match &block.kind {
            LaidOutBlockKind::BlockQuote { blocks } => &blocks[child_idx],
            LaidOutBlockKind::ListItem { blocks, .. } => &blocks[child_idx],
            _ => panic!("resolve_block: path element beyond leaf block"),
        };
    }
    block
}
```

- [ ] **Step 2: Add viewport_h param + line-level culling in render_block_with_offset**

First, add `viewport_h: f32` to the signature of `render_block_with_offset`:

```rust
fn render_block_with_offset(
    block: &LaidOutBlock, style: &MarkdownStyle, dl: &mut DrawList,
    scroll_y: f32, viewport_h: f32, ox: f32, oy: f32,
    mut shaper: Option<&mut shaping::Shaper>,
) {
```

Then add y-axis guards in the `Text` and `CodeBlock` match arms. The `Text` arm becomes:

```rust
LaidOutBlockKind::Text { lines } => {
    for line in lines {
        let ly = line.rect.y - scroll_y + oy;
        if ly + line.rect.h < 0.0 { continue; }
        if ly > viewport_h { continue; }
        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
    }
}
```

The `CodeBlock` arm — add the same guard inside the `dl.clip()` closure:

```rust
LaidOutBlockKind::CodeBlock { lines, .. } => {
    dl.fill_rounded(Rect::new(x, y, r.w, r.h), style.code_bg, 4.0);
    dl.clip(Rect::new(x, y, r.w, r.h), |dl| {
        for line in lines {
            let ly = line.rect.y - scroll_y + oy;
            if ly + line.rect.h < 0.0 { continue; }
            if ly > viewport_h { continue; }
            render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
        }
    });
}
```

Update the recursive calls inside `BlockQuote`, `ListItem` to pass `viewport_h`:

```rust
LaidOutBlockKind::BlockQuote { blocks } => {
    // ... border/bg rendering unchanged ...
    for child in blocks {
        render_block_with_offset(child, style, dl, scroll_y, viewport_h, ox, oy, shaper.as_deref_mut());
    }
}
LaidOutBlockKind::ListItem { bullet, blocks, lines, .. } => {
    // ... bullet rendering unchanged ...
    for line in lines {
        let ly = line.rect.y - scroll_y + oy;
        if ly + line.rect.h < 0.0 { continue; }
        if ly > viewport_h { continue; }
        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
    }
    for child in blocks {
        render_block_with_offset(child, style, dl, scroll_y, viewport_h, ox, oy, shaper.as_deref_mut());
    }
}
```

And update the old `render_doc_with_offset` call inside `render_doc` (the public 6-param version) and all tests to pass `viewport_h`.

- [ ] **Step 3: Add imports**

At the top of `render.rs`, add:

```rust
use crate::layout::LaidOutDoc;
use smallvec::SmallVec;
```

(Already imports `LaidOutBlock`, `LaidOutBlockKind`, `LaidOutLine` — verify these are present.)

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p edit-plus-markdown 2>&1`
Expected: compiles cleanly. Fix any errors.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/render.rs
git commit -m "feat(markdown): add viewport culling to render pass"
```

---

### Task 3: ScrollAnchor + Viewport in md_preview.rs

**Files:**
- Modify: `crates/app/src/md_preview.rs`

- [ ] **Step 1: Replace scroll_y with viewport**

Replace the `scroll_y` and related fields:

```rust
// Remove:
//     pub scroll_y: f32,
//     pub content_height: f32,

// Add:
use ui::viewport::Viewport;
```

In the struct `MarkdownPreview`, replace `scroll_y: f32` with:

```rust
    /// Viewport for scroll tracking (reuses text editor's ScrollAnchor model).
    pub viewport: Viewport,
    /// Total content height in pixels (for scrollbar).
    pub content_height: f32,
```

- [ ] **Step 2: Update new()**

In `MarkdownPreview::new()`, replace `scroll_y: 0.0`:

```rust
// Remove:
//     scroll_y: 0.0,

// Add:
        viewport: Viewport::new(30),  // default 30 visible rows
```

- [ ] **Step 3: Update scroll() method**

Replace the `scroll` method (line 78) with one that delegates to `Viewport::scroll_pixels`:

```rust
    /// Scroll by delta pixels. Returns true if scroll position changed.
    /// No-op before first render (when row_index is None).
    pub fn scroll(&mut self, delta: f32, line_height: f32) -> bool {
        let Some(ri) = self.row_index() else { return false; };
        let old_line = self.viewport.scroll_anchor.doc_line;
        let old_offset = self.viewport.scroll_anchor.pixel_offset;

        self.viewport.scroll_pixels(delta, ri, line_height);
        self.viewport.clamp_anchor(ri, line_height);

        self.viewport.scroll_anchor.doc_line != old_line
            || (self.viewport.scroll_anchor.pixel_offset - old_offset).abs() > 0.5
    }

    /// Access the BlockRowIndex. Returns None before first render.
    pub(crate) fn row_index(&self) -> Option<&edit_plus_markdown::layout::BlockRowIndex> {
        self.cached_layout.as_ref().map(|l| &l.row_index)
    }
```
```

- [ ] **Step 4: Update render() to derive scroll_y from viewport**

In the `render` method, change how `scroll_y` is obtained and used:

Add before the line that calls `render_doc_with_offset`:

```rust
        // Derive scroll_y from the viewport anchor + row_index
        let laid_out = self.cached_layout.as_ref().unwrap();
        let lh = style.line_height;
        let scroll_y = if laid_out.row_index.is_empty() {
            0.0
        } else {
            // scroll_y = first row y of anchor doc_line + pixel_offset
            let doc = self.viewport.scroll_anchor.doc_line.min(laid_out.row_index.len().saturating_sub(1));
            laid_out.row_index.row_y_starts[doc] + self.viewport.scroll_anchor.pixel_offset
        };
```

Replace the `self.scroll_y` references in the cached_dl check (lines 125-133):

```rust
        // Reuse cached DrawList if only scroll changed slightly
        let vp_key = (viewport_w, viewport_h);
        if let Some(ref cached) = self.cached_dl {
            if self.cached_dl_scroll_y == scroll_y && self.cached_dl_viewport == vp_key {
                if self.cached_vertices.is_some() {
                    return (DrawList::new(), false);
                }
                return (cached.clone(), true);
            }
        }
```

And the render call (line 136):

```rust
        let mut dl = DrawList::new();
        edit_plus_markdown::render::render_doc_with_offset(laid_out, &style, &mut dl, scroll_y, viewport_h, offset_x, offset_y, shaper.as_deref_mut());
        self.cached_dl = Some(dl.clone());
        self.cached_dl_scroll_y = scroll_y;
```

- [ ] **Step 5: Update scrollbar calculation in app_renderer callers**

The scrollbar currently reads `mv.preview.scroll_y` and `mv.preview.content_height`. These need to use the viewport. Update `render()` to set content_height, and update the external callers to derive scroll position from the viewport.

In `render()`, keep `self.content_height = laid_out.total_height;` (already present at line 112).

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles with errors about `scroll_y` field access. Those will be fixed in the next tasks. If there are other errors, fix them.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "refactor(md_preview): replace scroll_y with Viewport/ScrollAnchor"
```

---

### Task 4: Adapt app_renderer.rs to new MarkdownPreview API

**Files:**
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: Update scrollbar input to use viewport-derived values**

Find the scrollbar setup code around line 220 in `app_renderer.rs` (in the `build_vertices` or equivalent method). Replace `mv.preview.scroll_y` accesses with viewport-derived values.

The existing code:
```rust
let scrollbar_input = if is_md_preview {
    if let crate::view::View::Markdown(mv) = v {
        let lh = Settings::with(|s| s.line_height);
        let total = (mv.preview.content_height / lh).ceil() as usize;
        let scroll_rows = mv.preview.scroll_y / lh;
```

Change to (with `Option` handling — no-op when row_index is None):

```rust
let scrollbar_input = if is_md_preview {
    if let crate::view::View::Markdown(mv) = v {
        let lh = Settings::with(|s| s.line_height);
        let total = (mv.preview.content_height / lh).ceil() as usize;
        let scroll_rows = if let Some(ri) = mv.preview.row_index() {
            mv.preview.viewport.derive_scroll_top(ri, lh);
            mv.preview.viewport.scroll_top
        } else {
            0.0
        };
```

- [ ] **Step 2: Update md_preview.render() call to pass line_height**

The `render()` method signature changed — it no longer directly exposes `scroll_y` for the scrollbar. The call at line 396 should remain the same since `render()` still takes `(theme, viewport_w, viewport_h, offset_x, offset_y, shaper)`. Verify no signature change is needed.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles cleanly. Fix any remaining `scroll_y` references.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "refactor(app_renderer): adapt to MarkdownPreview Viewport API"
```

---

### Task 5: Adapt app_scroll.rs to use viewport.scroll_pixels

**Files:**
- Modify: `crates/app/src/app_scroll.rs`

- [ ] **Step 1: Update markdown scroll handler**

Replace the markdown scroll code (lines 159-172):

```rust
        // Markdown preview: scroll preview content (per-tab via View::Markdown)
        if let Some(crate::view::View::Markdown(mv)) = self.workspace.views.get_mut(self.workspace.active_index) {
            let dy: f32 = match delta {
                MouseScrollDelta::LineDelta(_, y) => y * -60.0,
                MouseScrollDelta::PixelDelta(pos) => -(pos.y as f32),
            };
            let lh = Settings::with(|s| s.line_height);
            let viewport_h = self.ui_shell.editor_rect().h;
            if mv.preview.scroll(dy, lh) {
                if let Some(ri) = mv.preview.row_index() {
                    mv.preview.viewport.derive_scroll_top(ri, lh);
                }
                let total_rows = (mv.preview.content_height / lh).ceil() as usize;
                let scroll_rows = mv.preview.viewport.scroll_top;
                self.ui_shell.set_scrollbar_input((viewport_h / lh) as f64, total_rows, scroll_rows);
                self.needs_redraw = true;
            }
            return;
        }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: zero errors, zero warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/app_scroll.rs
git commit -m "refactor(app_scroll): use Viewport::scroll_pixels for markdown scrolling"
```

---

### Task 6: LineMap impl + lib.rs exports

**Files:**
- Modify: `crates/markdown/src/lib.rs`

- [ ] **Step 1: Add LineMap implementation on BlockRowIndex**

In `crates/markdown/src/lib.rs`, add after the existing module declarations:

```rust
use ui::viewport::LineMap;
use crate::layout::BlockRowIndex;

impl LineMap for BlockRowIndex {
    fn map_line_count(&self) -> usize {
        self.row_y_starts.len()
    }

    fn map_total_rows(&self) -> usize {
        self.row_y_starts.len()
    }

    fn map_display_to_doc(&self, display_row: usize) -> usize {
        display_row.min(self.row_y_starts.len().saturating_sub(1))
    }

    fn map_doc_to_display(&self, doc_line: usize) -> usize {
        doc_line.min(self.row_y_starts.len().saturating_sub(1))
    }

    fn visual_line_count(&self, _doc_line: usize) -> u16 {
        1
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p edit-plus-markdown -p edit-plus-app 2>&1`
Expected: zero errors, zero warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/markdown/src/lib.rs
git commit -m "feat(markdown): impl LineMap for BlockRowIndex"
```

---

### Task 7: Culling correctness tests

**Files:**
- Modify: `crates/markdown/src/layout.rs` (add tests)
- Modify: `crates/markdown/src/render.rs` (add tests)

- [ ] **Step 1: Test row index build for flat document**

In `crates/markdown/src/layout.rs`, in the `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn row_index_flat_document() {
        let doc = make_doc("# Title\n\nParagraph text.");
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let ri = &laid_out.row_index;
        assert!(ri.len() > 0, "row index should have rows");
        // Row y positions must be non-decreasing
        for i in 1..ri.len() {
            assert!(ri.row_y_starts[i] >= ri.row_y_starts[i-1],
                "row y must be non-decreasing");
        }
        // All paths should be length 1 for a flat doc
        for path in &ri.row_block_paths {
            assert_eq!(path.len(), 1, "flat doc blocks should have path length 1");
        }
    }

    #[test]
    fn row_index_nested_blocks() {
        let doc = make_doc("> quoted text\n> more quote\n\n- list item\n  - nested");
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let ri = &laid_out.row_index;
        assert!(ri.len() > 0);
        // Should have at least one path longer than 1 (nested inside blockquote or list)
        let has_nested = ri.row_block_paths.iter().any(|p| p.len() > 1);
        assert!(has_nested, "nested doc should have multi-segment paths");
    }

    #[test]
    fn row_index_first_visible_binary_search() {
        let doc = make_doc(&"line\n".repeat(100));
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let ri = &laid_out.row_index;

        let first = ri.first_visible_row(0.0);
        assert_eq!(first, 0);

        // Scroll to middle — should land on or after the first line
        let mid_y = ri.row_y_starts[50];
        let mid = ri.first_visible_row(mid_y);
        assert!(mid >= 50 && mid <= 52, "binary search near known y");

        // Scroll past end — should clamp to last row
        let end = ri.first_visible_row(ri.row_y_starts[ri.len()-1] + 1000.0);
        assert_eq!(end, ri.len() - 1);
    }
```

- [ ] **Step 2: Run layout tests**

Run: `cargo test -p edit-plus-markdown --lib layout::tests::row_index 2>&1`
Expected: 3 tests pass.

- [ ] **Step 3: Test culling excludes blocks outside viewport**

In `crates/markdown/src/render.rs`, in the `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn culling_excludes_blocks_above_viewport() {
        // Render with scroll_y such that first block is above viewport
        let md = "# Top\n\n## Bottom";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        // Scroll past first heading
        let scroll_y = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 10.0;
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(&laid_out, &style, &mut dl, scroll_y, 600.0, 0.0, 0.0, Some(&mut shaper));
        // "Top" text should not be in the output
        let texts: Vec<&str> = dl.cmds.iter().filter_map(|c| {
            if let DrawCmd::TextLayout { layout, .. } = c { Some(layout.text.as_str()) } else { None }
        }).collect();
        assert!(!texts.iter().any(|t| t.contains("Top")), "block above viewport should be excluded");
        assert!(texts.iter().any(|t| t.contains("Bottom")), "visible block should be included");
    }

    #[test]
    fn culling_excludes_blocks_below_viewport() {
        let md = "# Top\n\n## Bottom";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        // Viewport only shows first part
        let scroll_y = 0.0;
        let small_vp = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 5.0; // just past first block
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(&laid_out, &style, &mut dl, scroll_y, small_vp, 0.0, 0.0, Some(&mut shaper));
        let texts: Vec<&str> = dl.cmds.iter().filter_map(|c| {
            if let DrawCmd::TextLayout { layout, .. } = c { Some(layout.text.as_str()) } else { None }
        }).collect();
        assert!(!texts.iter().any(|t| t.contains("Bottom")), "block below viewport should be excluded");
        assert!(texts.iter().any(|t| t.contains("Top")), "visible block should be included");
    }

    #[test]
    fn line_culling_excludes_single_line_outside_viewport() {
        // Multi-line paragraph where some lines are offscreen
        let md = "line 1\nline 2\nline 3\nline 4\nline 5";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        // Scroll to show only lines 3-5
        let scroll_y = laid_out.row_index.row_y_starts[2]; // start at line 3
        let small_vp = 50.0; // small viewport
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(&laid_out, &style, &mut dl, scroll_y, small_vp, 0.0, 0.0, Some(&mut shaper));
        let texts: Vec<&str> = dl.cmds.iter().filter_map(|c| {
            if let DrawCmd::TextLayout { layout, .. } = c { Some(layout.text.as_str()) } else { None }
        }).collect();
        assert!(!texts.iter().any(|t| t.contains("line 1")), "off-screen line should be excluded");
        assert!(!texts.iter().any(|t| t.contains("line 2")), "off-screen line should be excluded");
    }
```

- [ ] **Step 4: Run culling tests**

Run: `cargo test -p edit-plus-markdown --lib render::tests::culling 2>&1`
Expected: 3 tests pass.

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p edit-plus-markdown --lib 2>&1 && cargo test -p edit-plus-app --lib 2>&1`
Expected: all existing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/layout.rs crates/markdown/src/render.rs
git commit -m "test(markdown): add row index and viewport culling tests"
```

---

### Task 8: Remove dead code and final verification

**Files:**
- Modify: `crates/markdown/src/render.rs` (remove old unused function if any)
- Verify: `crates/app/src/md_preview.rs`
- Verify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: Check for dead code warnings**

Run: `cargo check -p edit-plus-markdown -p edit-plus-app 2>&1`
Expected: zero warnings. If any `scroll_y` references remain, fix them.

- [ ] **Step 2: Verify no scroll_y field access remains**

Run: `grep -rn "\.scroll_y" crates/app/src/md_preview.rs crates/app/src/app_renderer.rs crates/app/src/app_scroll.rs 2>/dev/null`
Expected: empty (no remaining direct field accesses).

- [ ] **Step 3: Verify workspace builds**

Run: `cargo check 2>&1`
Expected: zero errors, zero warnings.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "chore: final verification — zero warnings, all tests pass"
```
