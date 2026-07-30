# Subpixel Variant Caching for Text Rendering

**Date:** 2026-06-15
**Status:** design-approved
**Scope:** text rendering — subpixel glyph positioning (optimization 1 of 2 from `plans-text-rendering-optimization.md`)

## Problem

`generate_vertices()` and vertex emission in `render_cache.rs` apply `.round()` to glyph origin coordinates, snapping every glyph to an integer pixel. This discards subpixel kerning/tracking accumulated through floating-point advance sums, causing perceptually uneven character spacing in long lines of text.

The atlas `GlyphKey` already has a `subpixel_phase: u8` field, but it is hardcoded to `0` — every glyph gets exactly one rasterized variant.

## Solution

Quantize glyph origins to a 1/4-pixel grid (X-axis only, 4 variants), use the fractional phase to select or rasterize a subpixel-offset bitmap via swash's native `Render::offset()`, and draw the quad at the integer-truncated position.

### Variant count

- X-axis: 4 phases (0/4, 1/4, 2/4, 3/4 pixel)
- Y-axis: 1 (no vertical subpixel — vertical offsets are rarely perceptible for horizontal text)
- `GlyphKey.subpixel_phase` encodes `(x_phase << 2 | y_phase)` as a single `u8`, preserving Y-axis expansion space

## Data flow

```
GlyphCluster { advance, x_offset, ... }
  │
  ▼
Vertex computation (render_cache / paint_backend / render::lib.rs)
  │  x_accum = Σ advances  (e.g. 10.3)
  │  phase_x = (x_accum.fract() * 4.0) as u8  → 1
  │  quantized = (x_accum * 4.0).round() / 4.0  → 10.25
  │  px = quantized.trunc()  → 10  (integer quad position)
  │  GlyphKey { ..., subpixel_phase: encode(1, 0) }
  ▼
text_rasterize::resolve_glyph(key)
  │  atlas hit  → return cached GlyphSlot
  │  atlas miss ↓
  ▼
shaping::rasterize_glyph(font_id, glyph_id, size, subpixel_offset)
  │  Render::new(&[...])
  │    .format(Format::Alpha)
  │    .offset(Vector::new(phase_x / 4.0, 0.0))
  │    .render(&mut scaler, glyph_id)
  ▼
GlyphAtlas::insert() → upload to GPU
  │
  ▼
Quad drawn at integer px=10 with offset-variant bitmap
  → effective physical position = 10.25
```

## Files changed

| File | Change |
|------|--------|
| `crates/shaping/src/lib.rs` | `rasterize_glyph()` gains `subpixel_offset: (f32, f32)` parameter, passed to `Render::offset()` |
| `crates/app/src/text_rasterize.rs` | `resolve_glyph()` accepts phase, encodes it into `GlyphKey`, passes offset to shaper on miss |
| `crates/render/src/lib.rs` | `GlyphKey.subpixel_phase` populated from actual phase (was hardcoded 0); `generate_vertices` splits float coords into integer + phase |
| `crates/app/src/render_cache.rs` | `emit_vertices_for_visual_line`: same coordinate split |
| `crates/app/src/paint_backend.rs` | `emit_text`: same coordinate split |

### Key design decisions

- **`Format::Alpha` + `offset`, not `Format::Subpixel`.** swash's `Render::offset()` shifts the outline before rasterization at the vector level, producing a clean alpha bitmap. LCD subpixel rendering (RGB stripes) is a separate concern and adds atlas format complexity.
- **Existing `GlyphKey` field reused.** `subpixel_phase: u8` already exists in the struct and in atlas hashing. Currently hardcoded to 0. Encoding: `(x_phase << 2) | y_phase`.
- **Quantization: `round(coord * 4) / 4`.** Maximum quantization error is 0.125 px — invisible at typical DPIs.
- **Atlas capacity.** 4× entries per glyph in the worst case. Current atlas is 2048×2048 with 4096-entry LRU. Monitor LRU eviction rate after deployment; increase atlas pages if needed.

## Not in scope

- Y-axis subpixel variants (key structure supports future addition)
- Dynamic contrast enhancement (separate optimization, tracked in `plans-text-rendering-optimization.md` section 2)
- LCD subpixel rendering (RGB stripe)
