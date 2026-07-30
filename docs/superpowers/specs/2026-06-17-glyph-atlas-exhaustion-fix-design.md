# Fix Glyph Atlas Exhaustion Causing Text Drop

## Goal

Prevent text (characters) from disappearing when the shared glyph atlas fills up across multiple open documents.

## Root Cause

`GlyphAtlas` uses a shelf-packing allocator that never reclaims space from evicted glyphs. When the atlas texture (single 2048x2048 page, `max_pages=1`) fills up, `allocate()` returns `None`, and `resolve_glyph` silently drops the glyph. This is exacerbated by CJK characters and subpixel-positioned glyphs, which consume atlas space rapidly.

## Design

### 1. Increase ATLAS_SIZE to 4096

`ATLAS_SIZE` goes from 2048 to 4096, providing 4x atlas capacity. This defers exhaustion significantly; with 4096x4096 and subpixel phases, the atlas can hold ~16k distinct glyphs before filling.

**Files:** `crates/app/src/render_state.rs`, `crates/ui/src/gutter.rs`

### 2. Add allocation_failed Flag + clear() to GlyphAtlas

- `GlyphAtlas::allocation_failed: bool` — set `true` when `allocate()` returns `None` on the last allowed page (i.e., atlas is completely full)
- `GlyphAtlas::clear(&mut self)` — resets all pages (recreating the shelf state), clears the LRU slot cache, and clears the `oversized` set

**File:** `crates/render/src/lib.rs`

### 3. Recover on Exhaustion at End of Frame

In `App::render()`, after GPU submission, check `allocation_failed`. If true:

1. Call `atlas.clear()` — resets atlas allocator state (existing GPU texture pixels are overwritten on next rasterize, no GPU resource recreation needed)
2. Call `invalidate_all()` on every open document's `RenderCache` (`dv.display.render_cache`)
3. Call `invalidate_all()` on `PreviewRenderCache`
4. Increment `atlas_generation`
5. Set `self.needs_redraw = true`

The next frame re-rasterizes visible glyphs into the cleared atlas. One frame may show missing text (the frame that triggered exhaustion), but text reappears immediately in the following frame.

**File:** `crates/app/src/app_renderer.rs`

## Verification

1. Open multiple documents with distinct character sets (e.g., CJK files)
2. Scroll/switch tabs until atlas fills
3. Confirm text does not permanently disappear — at most one frame with missing glyphs, then full recovery

## Future Improvements (Out of Scope)

- **Multi-page GPU support**: Allow `max_pages > 1` by using a texture array (`texture_2d_array` in shader), enabling incremental growth without full clear
- **Space reclamation on eviction**: Track allocated rectangles and free them on LRU eviction, eliminating the need for full atlas clears
- **Targeted cache invalidation**: Use `insert_with_eviction` returned evicted keys to invalidate only affected `CachedLine` entries
