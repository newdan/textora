# Table Layout Bug Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix table row height miscalculation, cell text wrap width, and equal-column-width allocation in markdown preview table rendering.

**Architecture:** Three independent fixes in `layout.rs` and `render.rs`: (1) pass actual measured row heights from layout to render via new fields on `LaidOutBlockKind::Table`, (2) refactor `wrap_text` to accept explicit max-width for table cell wrapping, (3) add two-pass column width allocation (measure content demand, then allocate proportionally). All three are necessary — wrong wrap width causes horizontal overflow, wrong row heights cause vertical misalignment, and equal column widths exacerbate both.

**Tech Stack:** Rust, cosmic-text (Shaper), existing `crates/markdown` pipeline (parser → builder → layout → render)

---

### Task 1: Refactor `wrap_text` to accept explicit max-width

**Files:**
- Modify: `crates/markdown/src/layout.rs:203-302`

- [ ] **Step 1: Extract `wrap_text` body into `wrap_text_with_width`**

Replace the existing `wrap_text` method (line 203) with two methods. The original `wrap_text` becomes a thin delegate that calls `wrap_text_with_width(self.available_width())`.

**Before** (line 203-204):
```rust
    fn wrap_text(&mut self, text: &str, font_size: f32) -> Vec<String> {
        let max_w = self.available_width();
        let mut lines = Vec::new();
```

**After**:
```rust
    /// Word wrap to the full available viewport width (default).
    fn wrap_text(&mut self, text: &str, font_size: f32) -> Vec<String> {
        self.wrap_text_with_width(text, font_size, self.available_width())
    }

    /// Word wrap with an explicit maximum width in pixels.
    /// Used for table cells, blockquotes, and other constrained contexts.
    fn wrap_text_with_width(&mut self, text: &str, font_size: f32, max_w: f32) -> Vec<String> {
        let mut lines = Vec::new>();
```

Keep the rest of the body (lines 205-302) unchanged — they already use `max_w` from the local binding, which now comes from the parameter.

- [ ] **Step 2: Build check**

```bash
cargo build -p markdown 2>&1 | head -20
```

Expected: compiles successfully. All existing callers of `wrap_text` still work via the delegate.

---

### Task 2: Fix Bug 2 — use `cell_inner_w` for table cell text wrapping

**Files:**
- Modify: `crates/markdown/src/layout.rs:616`

- [ ] **Step 1: Change the wrap_text call in layout_table**

**Before** (line 616):
```rust
                let wrapped = ctx.wrap_text(t, font_size);
```

**After**:
```rust
                let wrapped = ctx.wrap_text_with_width(t, font_size, cell_inner_w);
```

`cell_inner_w` is already computed on line 618 as `cell_w - pad * 2.0`.

- [ ] **Step 2: Build check**

```bash
cargo build -p markdown 2>&1 | head -20
```

Expected: compiles cleanly.

---

### Task 3: Add `header_height` and `row_heights` to `LaidOutBlockKind::Table`

**Files:**
- Modify: `crates/markdown/src/layout.rs:42-47` (enum variant)
- Modify: `crates/markdown/src/layout.rs:595-643` (layout_table body)
- Modify: `crates/markdown/src/layout.rs:650-658` (Table construction)

- [ ] **Step 1: Add fields to the Table variant**

**Before** (lines 42-47):
```rust
    Table {
        columns: usize,
        header: Vec<Vec<LaidOutLine>>,
        rows: Vec<Vec<Vec<LaidOutLine>>>,
        column_widths: Vec<f32>,
    },
```

**After**:
```rust
    Table {
        columns: usize,
        header: Vec<Vec<LaidOutLine>>,
        rows: Vec<Vec<Vec<LaidOutLine>>>,
        column_widths: Vec<f32>,
        /// Header row height in pixels. 0.0 if no header.
        header_height: f32,
        /// Body row heights in pixels, one per row.
        row_heights: Vec<f32>,
    },
```

- [ ] **Step 2: Collect row heights in layout_table**

**Before** (line 595-596, only `header` and `body_rows` are collected):
```rust
    let mut header: Vec<Vec<LaidOutLine>> = Vec::new();
    let mut body_rows: Vec<Vec<Vec<LaidOutLine>>> = Vec::new();
```

**After** — add a Vec for body row heights:
```rust
    let mut header: Vec<Vec<LaidOutLine>> = Vec::new();
    let mut body_rows: Vec<Vec<Vec<LaidOutLine>>> = Vec::new();
    let mut body_row_heights: Vec<f32> = Vec::new();
```

**Before** (lines 640-643, inside the `else` branch):
```rust
        } else {
            body_rows.push(row);
            body_rows_h += actual_row_h;
        }
```

**After**:
```rust
        } else {
            body_rows.push(row);
            body_rows_h += actual_row_h;
            body_row_heights.push(actual_row_h);
        }
```

- [ ] **Step 3: Pass new fields in Table construction**

**Before** (lines 650-656):
```rust
    ctx.push_block(
        LaidOutBlockKind::Table {
            columns,
            header,
            rows: body_rows,
            column_widths,
        },
        total_h,
    );
```

**After**:
```rust
    ctx.push_block(
        LaidOutBlockKind::Table {
            columns,
            header,
            rows: body_rows,
            column_widths,
            header_height: if header.is_empty() { 0.0 } else { header_actual_h },
            row_heights: body_row_heights,
        },
        total_h,
    );
```

- [ ] **Step 4: Build check**

```bash
cargo build -p markdown 2>&1 | head -30
```

Expected: compile errors in `render.rs` because the Table pattern match doesn't include the new fields yet. That's expected — Task 4 fixes it.

---

### Task 4: Fix Bug 1 — use actual row heights in render

**Files:**
- Modify: `crates/markdown/src/render.rs:130-163`

- [ ] **Step 1: Update Table pattern match and replace hardcoded heights**

**Before** (lines 130-163):
```rust
        LaidOutBlockKind::Table { columns, header, rows, column_widths } => {
            let mut cell_y = y;

            // Header
            if !header.is_empty() {
                let header_h = style.line_height + 4.0;
                dl.fill_rounded(Rect::new(x, cell_y, r.w, header_h), style.table_header_bg, 0.0);
                for cell_lines in header.iter() {
                    for line in cell_lines {
                        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                    }
                }
                cell_y += header_h;
                // Separator line
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Body rows with zebra stripes
            for (row_idx, row) in rows.iter().enumerate() {
                cell_y += 2.0;
                // Zebra stripe: odd rows get a subtle background
                if row_idx % 2 == 1 {
                    let row_h = style.line_height + 2.0;
                    dl.fill_rounded(Rect::new(x, cell_y, r.w, row_h), style.table_stripe_bg, 0.0);
                }
                for cell_lines in row.iter() {
                    for line in cell_lines {
                        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                    }
                }
                cell_y += style.line_height + 2.0;
                // Row separator
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Vertical grid lines
            for i in 1..*columns {
                let cx = x + column_widths[..i].iter().sum::<f32>();
                dl.fill(Rect::new(cx, y, 1.0, r.h), style.table_border);
            }
        }
```

**After**:
```rust
        LaidOutBlockKind::Table { columns, header, rows, column_widths,
                                   header_height, row_heights } => {
            let mut cell_y = y;

            // Header — use actual measured height
            if !header.is_empty() && *header_height > 0.0 {
                dl.fill_rounded(Rect::new(x, cell_y, r.w, *header_height),
                                style.table_header_bg, 0.0);
                for cell_lines in header.iter() {
                    for line in cell_lines {
                        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                    }
                }
                cell_y += *header_height;
                // Separator line
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Body rows with zebra stripes — use actual measured heights
            for (row_idx, (row, &row_h)) in rows.iter().zip(row_heights.iter()).enumerate() {
                cell_y += 2.0;
                // Zebra stripe: odd rows get a subtle background
                if row_idx % 2 == 1 {
                    dl.fill_rounded(Rect::new(x, cell_y, r.w, row_h),
                                    style.table_stripe_bg, 0.0);
                }
                for cell_lines in row.iter() {
                    for line in cell_lines {
                        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                    }
                }
                cell_y += row_h;
                // Row separator
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Vertical grid lines
            for i in 1..*columns {
                let cx = x + column_widths[..i].iter().sum::<f32>();
                dl.fill(Rect::new(cx, y, 1.0, r.h), style.table_border);
            }
        }
```

Key changes:
- Pattern match destructures `header_height` and `row_heights`
- Header: `style.line_height + 4.0` → `*header_height`
- Body row height: `style.line_height + 2.0` → `row_h` from iterator
- Zebra stripe bg height: `style.line_height + 2.0` → `row_h`

- [ ] **Step 2: Build check**

```bash
cargo build -p markdown 2>&1 | head -20
```

Expected: compiles cleanly.

---

### Task 5: Add dynamic column width functions

**Files:**
- Modify: `crates/markdown/src/layout.rs` — add two new free functions after `collect_text_lines_with_styles`

- [ ] **Step 1: Add `measure_column_demand` function**

Insert after the `collect_text_lines_with_styles` function. Find its closing brace:

```bash
grep -n "fn collect_text_lines_with_styles" crates/markdown/src/layout.rs
```

Insert after that function's closing `}`. Add:

```rust
/// Measure per-column content width demand for dynamic column sizing.
///
/// For each column, computes the maximum of:
///   - the longest non-breakable token width (space-delimited)
///   - the longest full-line width × 0.6
/// This ensures narrow content columns don't hog space while wide columns
/// get enough room to avoid excessive wrapping.
fn measure_column_demand(
    block: &BlockNode,
    columns: usize,
    font_size: f32,
    shaper: Option<&mut Shaper>,
) -> Vec<f32> {
    let mut demand = vec![0.0f32; columns];

    for child in &block.children {
        if !matches!(child.kind, BlockKind::TableRow_) {
            continue;
        }
        for (ci, cell) in child.children.iter().enumerate() {
            if ci >= columns {
                break;
            }
            let (texts, _) = collect_text_lines_with_styles(cell);
            for t in &texts {
                if t.is_empty() {
                    continue;
                }
                let max_token_w = shaper.as_mut().map(|s| {
                    s.set_font_size(font_size);
                    t.split(' ')
                        .filter_map(|tok| s.shape(tok).ok().map(|r| r.width))
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0)
                }).unwrap_or_else(|| {
                    // Fallback: rough character-count estimate
                    t.split(' ').map(|tok| tok.chars().count() as f32 * font_size * 0.55).max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0)
                });
                let full_w = shaper.as_mut().map(|s| {
                    s.set_font_size(font_size);
                    s.shape(t).ok().map(|r| r.width).unwrap_or(0.0)
                }).unwrap_or(t.chars().count() as f32 * font_size * 0.55);
                let d = max_token_w.max(full_w * 0.6);
                if d > demand[ci] {
                    demand[ci] = d;
                }
            }
        }
    }
    demand
}
```

- [ ] **Step 2: Add `allocate_column_widths` function**

Insert after `measure_column_demand`:

```rust
/// Allocate column widths from content demand and available space.
///
/// Each column gets at least `min_col_w` and at most `max_col_w` (before padding).
/// Remaining space is distributed proportionally to demand. If clamping creates
/// surplus or deficit, a second pass redistributes among eligible columns.
fn allocate_column_widths(
    demand: &[f32],
    available_w: f32,
    pad: f32,
    min_col_w: f32,
    max_col_w: f32,
) -> Vec<f32> {
    let cols = demand.len();
    if cols == 0 {
        return vec![];
    }

    let total_pad = pad * 2.0 * cols as f32;
    let net_w = (available_w - total_pad).max(0.0);
    let total_demand: f32 = demand.iter().sum();

    // Empty table — equal distribution
    if total_demand <= 0.0 {
        return vec![net_w / cols as f32 + pad * 2.0; cols];
    }

    // First pass: proportional allocation with min/max clamping
    let mut widths: Vec<f32> = demand
        .iter()
        .map(|&d| {
            let w = (net_w * d / total_demand).max(min_col_w).min(max_col_w);
            w + pad * 2.0 // add cell padding back to get full column width
        })
        .collect();

    // Second pass: redistribute surplus/deficit from clamping
    let allocated: f32 = widths.iter().sum::<f32>() - total_pad;
    let delta = net_w - allocated;

    if delta.abs() > 0.5 {
        let eligible: Vec<usize> = (0..cols)
            .filter(|&i| {
                let w = widths[i] - pad * 2.0;
                if delta > 0.0 {
                    w < max_col_w
                } else {
                    w > min_col_w
                }
            })
            .collect();
        let eligible_demand: f32 = eligible.iter().map(|&i| demand[i]).sum();
        if eligible_demand > 0.0 {
            for &i in &eligible {
                let share = delta * demand[i] / eligible_demand;
                let new_w = (widths[i] + share)
                    .max(min_col_w + pad * 2.0)
                    .min(max_col_w + pad * 2.0);
                widths[i] = new_w;
            }
        }
    }

    widths
}
```

- [ ] **Step 3: Build check**

```bash
cargo build -p markdown 2>&1 | head -20
```

Expected: compiles. New functions are unused for now — no warnings if they're `pub(crate)` or `#[allow(dead_code)]` annotated, but we'll wire them in Task 6 immediately.

---

### Task 6: Integrate dynamic column width into `layout_table`

**Files:**
- Modify: `crates/markdown/src/layout.rs:584-589`

- [ ] **Step 1: Replace equal-width allocation with dynamic allocation in layout_table**

**Before** (lines 584-589):
```rust
fn layout_table(block: &BlockNode, ctx: &mut LayoutCtx, columns: usize) {
    let font_size = ctx.style.body_font_size;
    let line_h = ctx.style.line_height;
    let pad = ctx.style.table_cell_padding;
    let col_w = ctx.available_width() / columns.max(1) as f32;
    let column_widths: Vec<f32> = (0..columns).map(|_| col_w).collect();
```

**After**:
```rust
fn layout_table(block: &BlockNode, ctx: &mut LayoutCtx, columns: usize) {
    let font_size = ctx.style.body_font_size;
    let line_h = ctx.style.line_height;
    let pad = ctx.style.table_cell_padding;
    let available_w = ctx.available_width().max(20.0);

    // Dynamic column width: measure content demand, then allocate proportionally
    let demand = measure_column_demand(block, columns, font_size, ctx.shaper.as_deref_mut());
    let min_col_w = font_size * 3.0;   // at least 3 characters wide
    let max_col_w = available_w * 0.6; // no single column exceeds 60%
    let column_widths = allocate_column_widths(&demand, available_w, pad, min_col_w, max_col_w);
```

Also remove the now-unused `col_w` fallback on line 610. **Before** (line 610):
```rust
            let cell_w = column_widths.get(ci).copied().unwrap_or(col_w);
```

**After** — since `column_widths` is now always populated with `columns` entries via `allocate_column_widths`, the fallback is unreachable. Replace with a direct index:
```rust
            let cell_w = column_widths[ci];
```

- [ ] **Step 2: Build check**

```bash
cargo build -p markdown 2>&1 | head -20
```

Expected: compiles cleanly with zero warnings.

---

### Task 7: Run tests and verify

**Files:**
- Modify: `crates/markdown/src/layout.rs` — add table-specific layout tests

- [ ] **Step 1: Add table layout tests**

Insert in the `#[cfg(test)] mod tests` block near the other layout tests (after line 942). Add:

```rust
    #[test]
    fn layout_table_has_rows() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
        let doc = make_doc(md);
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let table = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { rows, .. } = &b.kind {
                Some(rows.len())
            } else {
                None
            }
        });
        assert_eq!(table, Some(2), "table should have 2 body rows");
    }

    #[test]
    fn layout_table_row_heights_match_row_count() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
        let doc = make_doc(md);
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let table = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { rows, row_heights, .. } = &b.kind {
                Some((rows.len(), row_heights.len()))
            } else {
                None
            }
        });
        assert_eq!(table, Some((2, 2)), "row_heights must have one entry per row");
    }

    #[test]
    fn layout_table_header_height_nonzero() {
        let md = "| Name | Value |\n| --- | --- |\n| x | 1 |";
        let doc = make_doc(md);
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let header_h = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { header_height, .. } = &b.kind {
                Some(*header_height)
            } else {
                None
            }
        });
        assert!(header_h.unwrap_or(0.0) > 0.0, "header should have nonzero height");
    }

    #[test]
    fn layout_table_row_height_reflects_long_content() {
        // Single-column table with a very long cell that should wrap
        let md = "| Long text |\n| --- |\n| this is a very long piece of text that should wrap to multiple lines in a narrow column |";
        let doc = make_doc(md);
        // Use a narrow viewport to force wrapping
        let laid_out = layout_doc(&doc, &default_style(), 200.0);
        let row_h = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { row_heights, .. } = &b.kind {
                row_heights.first().copied()
            } else {
                None
            }
        });
        // Single line_height is 24px; wrapped content should be taller
        assert!(row_h.unwrap_or(0.0) > 30.0,
            "wrapped cell row height ({}) should exceed single line",
            row_h.unwrap_or(0.0));
    }

    #[test]
    fn layout_table_column_widths_sum_to_available() {
        let md = "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 | 3 |";
        let doc = make_doc(md);
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let total_w: f32 = laid_out.blocks.iter().filter_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.iter().sum::<f32>())
            } else {
                None
            }
        }).sum();
        // Available width is 400. Columns may not exactly equal 400 due to
        // min/max clamping and padding, but should be within 10px.
        assert!((total_w - 400.0).abs() < 10.0,
            "column widths ({}) should approximately fill available width (400)", total_w);
    }

    #[test]
    fn layout_table_wide_column_gets_more_space() {
        // Column 0: short, Column 1: very long
        let md = "| id | description |\n| --- | --- |\n| 1 | this is a very long description that needs more horizontal room |";
        let doc = make_doc(md);
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        let widths: Vec<f32> = laid_out.blocks.iter().filter_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        }).flatten().collect();
        assert!(widths.len() >= 2, "expected at least 2 columns, got {}", widths.len());
        if widths.len() >= 2 {
            assert!(widths[1] > widths[0],
                "wide-content column 1 ({}px) should be wider than narrow column 0 ({}px)",
                widths[1], widths[0]);
        }
    }

    #[test]
    fn layout_table_ascii_art_not_broken_arbitrarily() {
        // ASCII tree diagram in a table cell — column should get enough width
        let md = "| File tree |\n| --- |\n| ├── mod.rs        # Widget trait |\n| └── state.rs      # State management |";
        let doc = make_doc(md);
        let laid_out = layout_doc(&doc, &default_style(), 400.0);
        // The column should be wide enough to hold the longest line
        let max_demand: f32 = laid_out.blocks.iter().filter_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                column_widths.first().copied()
            } else {
                None
            }
        }).sum();
        // Even with one column, the full 400px should be allocated
        assert!(max_demand > 300.0,
            "single wide column should get most of the 400px viewport, got {}", max_demand);
    }
```

- [ ] **Step 2: Run all existing markdown tests**

```bash
cargo test -p markdown 2>&1
```

Expected: ALL tests pass. Check specifically that:
- No existing tests broke (regression check)
- New table tests all pass
- `wrap_text_cjk_*` tests still pass (verify wrap_text refactor didn't break word wrap)

- [ ] **Step 3: Commit**

```bash
git add crates/markdown/src/layout.rs crates/markdown/src/render.rs
git commit -m "fix(markdown): table row height, cell wrap width, and dynamic column allocation

Fix three bugs in markdown table rendering:

1. Row heights: LaidOutBlockKind::Table now carries header_height and
   row_heights from layout to render, replacing hardcoded line_height
   estimates that diverged by 6-10px per row.

2. Cell text wrap: layout_table now passes cell_inner_w to
   wrap_text_with_width instead of using the full available_width(),
   so text wraps at the cell boundary, not the table boundary.

3. Dynamic column widths: replaces equal-width allocation with a
   two-pass content-driven approach (measure_column_demand +
   allocate_column_widths) that gives narrow content less space and
   wide content more, reducing unnecessary line wrapping.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: Manual smoke test

No automated visual test exists for the full rendering pipeline. Build and check visually.

- [ ] **Step 1: Full build**

```bash
cargo build 2>&1 | tail -5
```

Expected: compiles cleanly.

- [ ] **Step 2: Visual verification**

Run the app and open a markdown file containing a table. Verify:
- Row separators align with actual text baselines (not offset by a few pixels)
- Long cell text wraps within the cell boundary
- Columns with more content are wider than columns with less
- ASCII tree diagrams in table cells display completely without arbitrary mid-character breaks
- Zebra stripe backgrounds cover the full row height
- Multi-line cells have all lines visible (not clipped)

Suggested test markdown:

```markdown
| File | Description |
|------|-------------|
| `mod.rs` | Widget trait implementation |
| `state.rs` | SidebarState + SidebarAction management |
| `layout.rs` | SidebarLayoutItem computation and caching |

| Tree | Notes |
|------|-------|
| ├── mod.rs        # Widget trait 实现 |
| ├── state.rs      # SidebarState + SidebarAction |
| ├── layout.rs     # SidebarLayoutItem 计算 |
| ├── paint.rs      # 绘制逻辑（整合 widget 动画层 + 旧 chrome 绘制） |
| ├── types.rs      # SidebarInput, SidebarCfg 等 |
| └── persistent.rs # SidebarPersistent |
```
