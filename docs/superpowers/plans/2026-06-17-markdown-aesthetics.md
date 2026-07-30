# Markdown Rendering Aesthetics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the markdown rendering aesthetics spec — warm minimal color palette (peach accent), unified border-radius system, code block borders, blockquote vertical padding, list spacing, and horizontal rule polish.

**Architecture:** Extend MarkdownStyle with border-radius, inline code background, code block border, list item spacing, and rule spacing fields. Update render.rs to use style values instead of hardcoded constants. Fix blockquote vertical padding and add list item spacing in layout.rs. All markdown-specific values live in MarkdownStyle, not Theme.

**Tech Stack:** Rust, edit-plus-markdown crate (style.rs, render.rs, layout.rs)

---

### Task 1: Extend MarkdownStyle with new fields and updated colors

**File:** `crates/markdown/src/style.rs`

- [ ] **Step 1: Add new fields to struct**

Add after `pub code_bg: [f32; 4]` (line 17):

```rust
    pub inline_code_bg: [f32; 4],
```

Add before `pub paragraph_spacing: f32` (line 28):

```rust
    pub border_radius_base: f32,
    pub border_radius_small: f32,
    pub code_block_border: [f32; 4],
    pub list_item_spacing: f32,
    pub rule_spacing: f32,
```

- [ ] **Step 2: Set defaults for new fields in `from_theme`**

After the `code_bg` block (after line 69):

```rust
            inline_code_bg: if theme.is_dark {
                [0.8706, 0.4510, 0.3373, 0.15]
            } else {
                [0.8706, 0.4510, 0.3373, 0.1]
            },
```

Before `paragraph_spacing` line (~line 107):

```rust
            border_radius_base: 8.0,
            border_radius_small: 4.0,
            code_block_border: if theme.is_dark {
                [0.1804, 0.1765, 0.1725, 1.0]
            } else {
                [0.9137, 0.9020, 0.8824, 1.0]
            },
            list_item_spacing: line_height * 0.3,
            rule_spacing: 24.0,
```

- [ ] **Step 3: Update blockquote colors**

Replace `blockquote_bg` block (lines 81-85):

```rust
            blockquote_bg: if theme.is_dark {
                [0.8706, 0.4510, 0.3373, 0.08]
            } else {
                [0.8706, 0.4510, 0.3373, 0.05]
            },
```

Replace `blockquote_border` block (lines 86-90):

```rust
            blockquote_border: theme.sidebar_accent,
```

- [ ] **Step 4: Update code_bg to warm-toned**

Replace `code_bg` block (lines 65-69):

```rust
            code_bg: if theme.is_dark {
                [0.1098, 0.1059, 0.1020, 1.0]
            } else {
                [0.9725, 0.9686, 0.9608, 1.0]
            },
```

- [ ] **Step 5: Update rule_color to subtle separator-like**

Replace `rule_color` block (lines 76-79):

```rust
            rule_color: if theme.is_dark {
                [0.2118, 0.2353, 0.2745, 0.6]
            } else {
                [0.8745, 0.8745, 0.8784, 0.6]
            },
```

- [ ] **Step 6: Update blockquote_padding default**

Change `blockquote_padding: 12.0` to `blockquote_padding: 16.0` (line 111).

- [ ] **Step 7: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly (new fields may have dead-code warnings until used in later tasks).

---

### Task 2: Extract inline code background helper + apply new style

**File:** `crates/markdown/src/render.rs`

- [ ] **Step 1: Add `draw_inline_code_bg` helper**

Place after `render_line_with_offset` (after line 320, before `end_x_for_offset`):

```rust
fn draw_inline_code_bg(style: &MarkdownStyle, dl: &mut DrawList, x: f32, y: f32, w: f32, h: f32) {
    dl.fill_rounded(Rect::new(x, y, w, h), style.inline_code_bg, style.border_radius_small);
}
```

- [ ] **Step 2: Replace first inline code background call (non-precomputed path)**

Line 221 — replace:
```rust
                dl.fill_rounded(Rect::new(bg_x, bg_y, bg_w, bg_h), style.code_bg, 3.0);
```
with:
```rust
                draw_inline_code_bg(style, dl, bg_x, bg_y, bg_w, bg_h);
```

- [ ] **Step 3: Replace second inline code background call (precomputed path)**

Line 275 — replace:
```rust
            dl.fill_rounded(Rect::new(bg_x, bg_y, bg_w, bg_h), style.code_bg, 3.0);
```
with:
```rust
            draw_inline_code_bg(style, dl, bg_x, bg_y, bg_w, bg_h);
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 3: CodeBlock — apply border_radius_base + stroke border

**File:** `crates/markdown/src/render.rs`

- [ ] **Step 1: Update CodeBlock rendering**

Lines 60-68 — replace the existing CodeBlock arm:

```rust
        LaidOutBlockKind::CodeBlock { lines, .. } => {
            dl.fill_rounded(Rect::new(x, y, r.w, r.h), style.code_bg, style.border_radius_base);
            dl.stroke_rounded(Rect::new(x, y, r.w, r.h), style.code_block_border, style.border_radius_base, 1.0);
            dl.clip(Rect::new(x, y, r.w, r.h), |dl| {
                for line in lines {
                    render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                }
            });
        }
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 4: BlockQuote — painter's algorithm for border + bg

**File:** `crates/markdown/src/render.rs`

> **Why painter's algorithm:** Drawing the border strip on top of a full-width rounded background avoids a visible gap at the top-left/bottom-left corners where the background's rounded arc pulls away from the straight border edge. The border overlays the left-side rounded corners, leaving only the right-side rounding visible.

- [ ] **Step 1: Replace BlockQuote rendering**

Lines 70-78 — replace the existing BlockQuote arm:

```rust
        LaidOutBlockKind::BlockQuote { blocks } => {
            dl.fill_rounded(Rect::new(x, y, r.w, r.h), style.blockquote_bg, style.border_radius_base);
            dl.fill(Rect::new(x, y, 4.0, r.h), style.blockquote_border);
            for child in blocks {
                render_block_with_offset(child, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
            }
        }
```

Key changes from current:
- `fill_rounded` covers full width first (was at `x + 3.0` with reduced width)
- `fill` border strip drawn on top (4px wide instead of 3px)
- Radius changed from `4.0` to `style.border_radius_base`

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 5: Table — straight border (no rounded corners)

**File:** `crates/markdown/src/render.rs`

> **Why straight border:** `PushClip` uses GPU scissor (orthogonal rect only). Rounded borders with rectangular internal fills would bleed content past the curved corners. Use straight border until the renderer supports rounded clips via stencil/SDF.

- [ ] **Step 1: Replace Table rendering**

Lines 131-173 — replace the existing Table arm:

```rust
        LaidOutBlockKind::Table { columns, header, rows, column_widths,
                                   header_height, row_heights } => {
            let mut cell_y = y;

            // Outer border
            dl.stroke(Rect::new(x, y, r.w, r.h), style.table_border, 1.0);

            // Header
            if !header.is_empty() && *header_height > 0.0 {
                dl.fill(Rect::new(x, cell_y, r.w, *header_height), style.table_header_bg);
                for cell_lines in header.iter() {
                    for line in cell_lines {
                        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                    }
                }
                cell_y += *header_height;
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Body rows with zebra stripes
            for (row_idx, row) in rows.iter().enumerate() {
                let row_h = row_heights.get(row_idx).copied().unwrap_or(style.line_height + 2.0);
                cell_y += 2.0;
                if row_idx % 2 == 1 {
                    dl.fill(Rect::new(x, cell_y, r.w, row_h), style.table_stripe_bg);
                }
                for cell_lines in row.iter() {
                    for line in cell_lines {
                        render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
                    }
                }
                cell_y += row_h;
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Vertical grid lines
            for i in 1..*columns {
                let cx = x + column_widths[..i].iter().sum::<f32>();
                dl.fill(Rect::new(cx, y, 1.0, r.h), style.table_border);
            }
        }
```

Key changes from current:
- Added `dl.stroke(...)` outer border before header
- `fill_rounded(..., 0.0)` → `fill(...)` (header bg and zebra stripe)

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 6: BlockQuote vertical padding in layout

**File:** `crates/markdown/src/layout.rs`

- [ ] **Step 1: Add vertical padding to blockquote layout**

Lines 544-567 — replace the BlockQuote layout logic:

```rust
        BlockKind::BlockQuote => {
            let saved_indent = ctx.indent;
            ctx.indent += ctx.style.blockquote_padding;
            let start_y = ctx.y;
            ctx.y += ctx.style.blockquote_padding;
            let saved_color_fade = ctx.color_fade;
            ctx.color_fade = 0.25;

            // Collect child blocks into a sub-layout
            let mut sub_blocks = Vec::new();
            let saved_output = std::mem::take(&mut ctx.output);
            for child in &block.children {
                layout_block(child, ctx);
            }
            sub_blocks.extend(ctx.output.drain(..));
            ctx.output = saved_output;

            let content_h = ctx.y - start_y + ctx.style.blockquote_padding;
            ctx.y = start_y;
            ctx.indent = saved_indent;
            ctx.color_fade = saved_color_fade;

            ctx.push_block(LaidOutBlockKind::BlockQuote { blocks: sub_blocks }, content_h);
            ctx.last_block_was_heading = false;
            ctx.block_count += 1;
        }
```

Key changes:
- Added `ctx.y += ctx.style.blockquote_padding;` before child layout (top padding)
- `content_h` changed from `ctx.y - start_y` to `ctx.y - start_y + ctx.style.blockquote_padding` (bottom padding)

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 7: List item spacing + bullet color fade

**Files:** `crates/markdown/src/layout.rs`, `crates/markdown/src/render.rs`

> **Problem:** Consecutive list items within the same list have no vertical gap — they abut directly. Adding a small gap (0.3 × line_height) improves readability. Also, bullet glyphs should be slightly faded to reduce their visual weight vs. the item text.

- [ ] **Step 1: Track list item adjacency in layout context**

In `crates/markdown/src/layout.rs`, add a field to `LayoutCtx` after `color_fade` (line 269):

```rust
    between_list_items: bool,
```

Initialize in `LayoutCtx::new()` after `list_depth: 0` (line 270):

```rust
            between_list_items: false,
```

- [ ] **Step 2: Add spacing between consecutive list items**

In the ListItem handler (lines 569-622), add spacing at the top when the previous sibling was also a ListItem. Insert after `let start_y = ctx.y;` (line 575):

```rust
            if ctx.between_list_items {
                ctx.y += ctx.style.list_item_spacing;
            }
```

After `ctx.push_block(...)` inside the ListItem handler (after line 619), add:

```rust
            ctx.between_list_items = true;
```

- [ ] **Step 3: Reset `between_list_items` for non-list blocks**

Add `ctx.between_list_items = false;` as the first line in each of these handler arms:

| Handler | Insert after line |
|---|---|
| Container | 479 |
| Paragraph | 484 |
| Heading | 490 |
| CodeBlock | 506 |
| BlockQuote | 544 |
| TableWrapper | 623 |
| HorizontalRule | 633 |

Each insertion looks like this (example for Paragraph at line 484):

```rust
        BlockKind::Paragraph => {
            ctx.between_list_items = false;
            layout_text_block(block, ctx, ctx.style.body_font_size, ctx.style.text_color);
```

Repeat the same `ctx.between_list_items = false;` line for each handler listed above.

- [ ] **Step 4: Fade bullet color in render**

In `crates/markdown/src/render.rs`, in the ListItem arm, replace bullet text color from `style.text_color` to a faded variant. For the Bullet/Ordered arms (lines 92, 96), wrap the color:

```rust
// Before each text_shaped call for bullets, compute faded color:
let bullet_color = blend_toward_bg(style.text_color, style.background_color, 0.3);
```

Add the import at the top of render.rs:

```rust
use crate::style::blend_toward_bg;
```

Then replace `style.text_color` with `bullet_color` in the two bullet text_shaped calls (lines 92 and 96):

```rust
if let Some(ref mut s) = shaper { dl.text_shaped(bullet_x + 4.0, y + font_size, font_size, bullet_color, symbol, s); }
// and:
if let Some(ref mut s) = shaper { dl.text_shaped(bullet_x + 4.0, y + font_size, font_size, bullet_color, &label, s); }
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 8: Horizontal rule spacing + color polish

**Files:** `crates/markdown/src/layout.rs`, `crates/markdown/src/style.rs`

> **Problem:** The current `<hr>` height is only `rule_thickness + paragraph_spacing` (1 + ~15.6 = ~16.6px). The spec calls for 24px top and bottom padding, giving the section break more visual breathing room.

- [ ] **Step 1: Update HorizontalRule layout height**

In `crates/markdown/src/layout.rs`, line 634 — replace:

```rust
            ctx.push_block(LaidOutBlockKind::HorizontalRule, ctx.style.rule_thickness + ctx.style.paragraph_spacing);
```

with:

```rust
            ctx.push_block(LaidOutBlockKind::HorizontalRule, ctx.style.rule_spacing + ctx.style.rule_thickness + ctx.style.rule_spacing);
```

- [ ] **Step 2: Update render to center the rule vertically within its block**

In `crates/markdown/src/render.rs`, lines 174-178 — replace the HorizontalRule arm:

```rust
        LaidOutBlockKind::HorizontalRule => {
            let rule_w = r.w * style.rule_width_ratio;
            let rule_x = x + (r.w - rule_w) / 2.0;
            let rule_y = y + (r.h - style.rule_thickness) / 2.0;
            dl.fill(Rect::new(rule_x, rule_y, rule_w, style.rule_thickness), style.rule_color);
        }
```

Key change: `y` → `rule_y` so the rule is centered in the expanded height block.

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly.

---

### Task 9: Typography adjustments in from_theme

**File:** `crates/markdown/src/style.rs`

- [ ] **Step 1: Update heading scale**

Line 50 — replace:
```rust
        let heading_scale = [1.8, 1.5, 1.25, 1.1, 1.0, 0.9];
```
with:
```rust
        let heading_scale = [2.2, 1.75, 1.4, 1.15, 1.0, 0.9];
```

(at 16px base: H1 35.2, H2 28.0, H3 22.4, H4 18.4)

- [ ] **Step 2: Update paragraph_spacing**

Line 107 — replace:
```rust
            paragraph_spacing: line_height * 0.6,
```
with:
```rust
            paragraph_spacing: line_height * 0.9,
```

- [ ] **Step 3: Update heading spacing**

Lines 108-109 — replace:
```rust
            heading_spacing_top: line_height * 1.5,
            heading_spacing_bottom: line_height * 0.4,
```
with:
```rust
            heading_spacing_top: line_height * 1.8,
            heading_spacing_bottom: line_height * 0.5,
```

- [ ] **Step 4: Verify compilation — confirm zero warnings**

```bash
cargo check -p edit-plus-markdown 2>&1
```

Expected: compiles cleanly, zero warnings.

---

### Task 10: Run existing tests, fix failures

- [ ] **Step 1: Run full markdown test suite**

```bash
cargo test -p edit-plus-markdown --lib 2>&1
```

Expected: most tests pass. Likely failures:

- `blockquote_text_color_is_faded` — may be affected by vertical padding change (block height increased). Update rect height assertions.
- `blockquote_nested_in_list_preserves_color_fade` — same as above.
- Any test checking rect dimensions that include old blockquote_padding (12→16), border width (3→4), or rule spacing.
- Tests checking `rule_thickness + paragraph_spacing` for HorizontalRule height (now adds `rule_spacing * 2`).

- [ ] **Step 2: Fix any test failures**

For each failure, read the test, understand the assertion, and update expected values to match the new defaults:
- If a rect height test expects old `rule_thickness + paragraph_spacing`, update to `rule_spacing + rule_thickness + rule_spacing`.
- If a blockquote test expects old padding (12px), update to 16px.
- If a list test checks spacing, confirm the `between_list_items` flag correctly separates items.

- [ ] **Step 3: Re-run tests after fixes**

```bash
cargo test -p edit-plus-markdown --lib 2>&1
```

Expected: all tests pass.

- [ ] **Step 4: Run full workspace check**

```bash
cargo check -p edit-plus-app 2>&1
```

Expected: compiles cleanly.

---

### Task 11: Final verification and commit

- [ ] **Step 1: Run workspace tests**

```bash
cargo test -p edit-plus-markdown --lib 2>&1
cargo test -p edit-plus-app --lib 2>&1
```

Expected: all tests pass, zero warnings.

- [ ] **Step 2: Commit**

```bash
git add crates/markdown/src/style.rs crates/markdown/src/render.rs crates/markdown/src/layout.rs
git commit -m "feat(markdown): warm aesthetic — accent colors, border-radius system, list/horizontal-rule spacing, blockquote padding"
```
