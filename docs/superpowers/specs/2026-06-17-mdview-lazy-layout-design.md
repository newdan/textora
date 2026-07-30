# MdView Lazy Layout Design

**Goal:** Replace full-document word-wrap + rect calculation with two-phase lazy layout. First frame only does precise layout on visible blocks + buffer; remaining blocks use estimated heights. Scroll triggers on-demand precise layout. Target: reduce 5.2MB doc layout time from ~187ms to <30ms.

## Architecture

```
First render:
  MarkdownDoc (BlockNode tree, retained)
       │
       ▼
  Estimation Pass ──→ LaidOutDoc (all blocks, estimated heights, precise=false)
       │
       ▼
  Precision Pass ──→ precise layout for blocks in [scroll_y - 2vh, scroll_y + vh + 3vh]
       │
       ▼
  render ──→ DrawList (existing viewport culling)

Scroll:
  1. Save BlockAnchor (current visible block + offset)
  2. Detect blocks newly entering buffer → batch Precision Pass
  3. If any height change occurred above viewport → restore scroll_y from anchor
  4. render

Edit:
  1. Re-run Precision Pass on the edited block only
  2. Propagate height_delta to y_delta[i+1..] (single O(N) pass)
  3. No full re-estimation needed for localized edits
```

### Core data structure

```rust
struct LazyLayout {
    doc: MarkdownDoc,       // retained source tree for deferred layout
    laid_out: LaidOutDoc,   // mixed: some precise, some estimated
    precise: Vec<bool>,     // per top-level block: has precise layout been done?
    y_delta: Vec<f32>,      // cumulative y offset per block (estimation error correction)
}
```

`y_delta[i]` is the accumulated height correction from all blocks before `i`. A block's actual visual Y = `laid_out.blocks[i].rect.y + y_delta[i]`.

### Monotonicity invariant for binary search

The render pass performs binary search on `rect.y + y_delta[i]`. This sequence is guaranteed monotonic because:
- `rect.y[i]` is monotonic (layout always produces increasing Y)
- `y_delta[i]` is cumulative: `y_delta[i+1] = y_delta[i] + height_delta[i]`
- `rect.y[i] + height_delta[i] >= 0` (block height can never go negative)
- Therefore `rect.y[i+1] + y_delta[i+1] >= rect.y[i] + y_delta[i]`

If future features add collapsible/hidden blocks, they must either preserve this invariant or replace binary search with linear scan.

## Estimation Pass (~10-20ms)

Reuses existing `wrap_text` fast path: ASCII width cache + CJK advance lookups, no HarfBuzz calls.

- **Paragraph/Heading:** `wrap_text` fast path → line count × line_height
- **CodeBlock:** split by newline → line count × code_line_height (no wrapping)
- **BlockQuote:** recursively estimate children; outer height = sum(child heights)
- **ListItem:** estimate item text + recursively estimate nested children
- **Table:** dynamic column width allocation runs once during estimation. Column widths are **saved and reused** in the precision pass to prevent visual misalignment from width changes. Each cell uses fast-path wrapping.
- **HorizontalRule:** fixed height

Each block is stored with `shaped: None`, `text_layout: None`, `style_segments: vec![]`. `precise` is `false`.

`total_height` and approximate `rect.y` for all blocks are available after this pass — enough for scrollbar rendering.

## Precision Pass

For blocks in `[scroll_y - 2vh, scroll_y + viewport_h + 3vh]`:

1. Run full `layout_block()` including HarfBuzz shaping and style_segments computation
2. For tables: **reuse column widths from estimation pass** — do not recompute them
3. Accumulate `height_delta = new_height - estimated_height` per block
4. Mark `precise: true`

**Batch y_delta propagation (single O(N) pass per frame):**

After all blocks in the current frame are precision-laid-out, compute a single pass over the `y_delta` array:

```rust
fn apply_deltas(y_delta: &mut [f32], height_deltas: &[(usize, f32)]) {
    // height_deltas: (block_idx, delta), sorted by block_idx
    // y_delta[i] corrects the Y of block i based on deltas from blocks BEFORE i.
    // A block's own height change only shifts blocks i+1 and beyond.
    let mut cum = 0.0;
    let mut di = 0;
    for i in 0..y_delta.len() {
        while di < height_deltas.len() && height_deltas[di].0 < i {
            cum += height_deltas[di].1;
            di += 1;
        }
        y_delta[i] += cum; // cum only includes deltas from blocks [0..i-1]
    }
}
```

This avoids O(K*N) when K blocks enter the buffer in one frame.

**Nested blocks:** BlockQuote and ListItem are precision-laid-out as a whole unit, including all their children recursively. No separate precise tracking for child blocks — the container is the granularity.

### Buffer: 2vh above, 3vh below viewport

Covers fast scrolling/wheel gestures. Buffer blocks get shaped and styled but may not emit DrawCmds if still off-screen (existing viewport culling handles the render skip).

## Scroll jump prevention (upward scroll)

When precision-laying-out a block **above** the current viewport causes a height change, all blocks below it shift — including the visible content on screen. This causes a visual jump if `scroll_y` is not corrected.

**Protocol for every scroll-triggered precision pass:**

1. **Before** precision pass: capture `BlockAnchor` from current `scroll_y`
2. Run batch precision pass on newly-entered blocks
3. **After** precision pass: call `restore_anchor(&anchor)` to adjust `scroll_y` so the content the user was looking at stays fixed on screen

This applies regardless of scroll direction — the anchor absorbs any y-delta from blocks above the viewport.

```rust
// In md_preview.rs render() or scroll handler:
let anchor = self.lazy_layout.anchor_at(self.scroll_y);
self.lazy_layout.ensure_precise_range(self.scroll_y, viewport_h);
self.restore_anchor(&anchor);
```

## Edit-time behavior

For localized edits (single character input/delete in a 5.2MB doc):

1. **No full re-estimation.** The `BlockNode` tree is updated in-place by the builder.
2. Re-run **Precision Pass** on the edited block only (it's already in or near the visible range).
3. Compute `height_delta` vs the block's previous height.
4. Single O(N) pass to propagate `height_delta` into `y_delta[i+1..]`.
5. Updated `total_height = old_total + height_delta`.

If the edit adds/removes entire blocks (e.g., new paragraph break), a full re-estimation is needed because the block count changes. But this is a rare operation compared to character-level typing.

This keeps per-keystroke layout overhead near zero — only the edited block gets re-laid-out.

## Render integration

`render_doc_with_offset` uses `block.rect.y + y_delta[i]` instead of `block.rect.y` for:
- The binary search to find `first_visible_block_idx`
- The Y positioning of each block

Otherwise unchanged — existing line-level culling still applies.

## What we do NOT do

- Do NOT change the `LaidOutBlock` / `LaidOutDoc` types (add fields only as needed)
- Do NOT change the render path (only Y offset source changes)
- Do NOT change `scroll_y: f32` pixel scrolling
- Do NOT add a separate "offset table" — `y_delta` is enough
- Do NOT garbage-collect precise blocks (once laid out, kept)

## Implementation approach

Modify `crates/markdown/src/layout.rs`:
- Add `LazyLayout` struct, estimation helper, batch precision pass
- `apply_deltas` helper for single-pass y_delta propagation
- Modify `layout_doc_with_shaper` or add parallel lazy entry point
- Ensure existing `layout_doc` (non-lazy) still works for tests

Modify `crates/app/src/md_preview.rs`:
- Hold `LazyLayout` instead of bare `LaidOutDoc`
- Anchor capture → precision pass → anchor restore on every scroll frame
- On edit: precision-only update on changed block + y_delta propagation
- Wire `y_delta` into render call via `block.rect.y + y_delta[i]`

## Known edge cases

- **Table column width:** estimation pass computes column widths once and they are reused in precision pass. This prevents visual misalignment from width divergence.
- **Wrapping difference:** fast-path wrapping may produce different line breaks than shaped wrapping for mixed CJK+ASCII text. Height may change slightly after precision, absorbed by y_delta and scroll anchor.
- **Edit that changes block count:** when an edit adds or removes top-level blocks, full re-estimation is required (rare, acceptable).
