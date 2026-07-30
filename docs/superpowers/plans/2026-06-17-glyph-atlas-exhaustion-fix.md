# Glyph Atlas Exhaustion Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent text from permanently disappearing when the shared glyph atlas fills up across multiple open documents.

**Architecture:** Increase atlas texture from 2048^2 to 4096^2 (4x capacity). When the atlas does fill up, set a flag. At end of frame, check the flag and if set: clear the atlas allocator state, invalidate all render caches, bump atlas generation, and request redraw. The next frame re-rasterizes visible glyphs.

**Tech Stack:** Rust, wgpu, existing `GlyphAtlas` + `RenderCache` infrastructure

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `crates/render/src/lib.rs` | Modify | Add `allocation_failed` flag + `clear()` method + tests |
| `crates/app/src/render_state.rs` | Modify | `ATLAS_SIZE` 2048 → 4096 |
| `crates/ui/src/gutter.rs` | Modify | `ATLAS_SIZE` 2048 → 4096 |
| `crates/app/src/app_renderer.rs` | Modify | End-of-frame exhaustion recovery |

---

### Task 1: Increase ATLAS_SIZE

**Files:**
- Modify: `crates/app/src/render_state.rs:14`
- Modify: `crates/ui/src/gutter.rs:6`

- [ ] **Step 1: Bump ATLAS_SIZE in render_state.rs**

Change line 14:
```rust
pub(crate) const ATLAS_SIZE: u32 = 2048;
```
to:
```rust
pub(crate) const ATLAS_SIZE: u32 = 4096;
```

- [ ] **Step 2: Bump ATLAS_SIZE in gutter.rs**

Change line 6:
```rust
pub const ATLAS_SIZE: u32 = 2048;
```
to:
```rust
pub const ATLAS_SIZE: u32 = 4096;
```

- [ ] **Step 3: Build check**

Run: `cargo check -p app -p ui 2>&1`
Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/render_state.rs crates/ui/src/gutter.rs
git commit -m "fix: increase ATLAS_SIZE from 2048 to 4096 to defer glyph atlas exhaustion"
```

---

### Task 2: Add allocation_failed Flag + clear() to GlyphAtlas

**Files:**
- Modify: `crates/render/src/lib.rs`

- [ ] **Step 1: Add `allocation_failed` field to `GlyphAtlas` struct**

Add `pub allocation_failed: bool` after the `max_pages` field (line 118):
```rust
pub struct GlyphAtlas {
    pages: Vec<AtlasPage>,
    slots: LruCache<GlyphKey, GlyphSlot>,
    oversized: HashSet<GlyphKey>,
    max_pages: usize,
    page_width: u32,
    page_height: u32,
    /// True when the atlas is completely full (all pages exhausted).
    pub allocation_failed: bool,
}
```

- [ ] **Step 2: Initialize `allocation_failed` in `GlyphAtlas::new`**

In `new()`, add `allocation_failed: false` to the `Self { ... }` block (after line 136):
```rust
Self {
    max_pages: max_pages.max(1),
    pages: vec![first_page],
    slots: LruCache::new(capacity),
    oversized: HashSet::new(),
    page_width,
    page_height,
    allocation_failed: false,
}
```

- [ ] **Step 3: Set `allocation_failed = true` when max_pages is hit**

In `insert()`, the block at line 171-173 currently reads:
```rust
if self.pages.len() >= self.max_pages {
    eprintln!("[atlas] page limit reached ({}/{} pages), glyph insertion skipped", self.pages.len(), self.max_pages);
    return None;
}
```
Change to:
```rust
if self.pages.len() >= self.max_pages {
    self.allocation_failed = true;
    return None;
}
```
(Remove the `eprintln!` — the flag replaces the log.)

- [ ] **Step 4: Add `clear()` method to `GlyphAtlas` impl**

Add after `glyph_count()` (after line 218):
```rust
/// Clear all atlas pages, slot cache, and oversized set.
/// Resets the allocator state. GPU texture pixels are overwritten
/// on the next rasterize pass — no GPU resource is recreated.
pub fn clear(&mut self) {
    let mut first_page = AtlasPage::new(0, self.page_width, self.page_height);
    first_page.reserve(1, 1); // preserve cursor white pixel at origin
    self.pages = vec![first_page];
    self.slots.clear();
    self.oversized.clear();
    self.allocation_failed = false;
}
```

- [ ] **Step 5: Add tests**

Add the following test functions to the `#[cfg(test)] mod tests` block, before the closing `}` of the module (before line 496):

```rust
#[test]
fn allocation_failed_set_when_max_pages_exhausted() {
    // Tiny page (32x32), max_pages=1. Each glyph 10x10.
    // Page 0 can fit ~6 glyphs (32/11≈2 per row, ~3 rows).
    // After that, allocation_failed should be set.
    let mut atlas = GlyphAtlas::new(32, 32, 20, 1);
    for i in 0..20u32 {
        let key = GlyphKey { glyph_id: i, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
        atlas.insert(key, 10, 10, 0.0, 0.0);
    }
    assert!(atlas.allocation_failed, "allocation_failed should be true after atlas fills");
}

#[test]
fn clear_resets_atlas_state() {
    let mut atlas = GlyphAtlas::new(256, 256, 10, 1);
    let key = GlyphKey { glyph_id: 1, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
    atlas.insert(key, 10, 10, 0.0, 0.0);
    assert_eq!(atlas.glyph_count(), 1);
    assert_eq!(atlas.page_count(), 1);

    atlas.clear();

    assert_eq!(atlas.glyph_count(), 0);
    assert_eq!(atlas.page_count(), 1);
    assert!(!atlas.allocation_failed);
    // Previously inserted glyph should be gone
    assert!(atlas.get(&key).is_none());
}

#[test]
fn clear_resets_allocation_failed_flag() {
    // Exhaust a tiny atlas
    let mut atlas = GlyphAtlas::new(32, 32, 20, 1);
    for i in 0..20u32 {
        let key = GlyphKey { glyph_id: i, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
        atlas.insert(key, 10, 10, 0.0, 0.0);
    }
    assert!(atlas.allocation_failed);

    atlas.clear();
    assert!(!atlas.allocation_failed);

    // Should be able to insert again after clear
    let key = GlyphKey { glyph_id: 99, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
    assert!(atlas.insert(key, 10, 10, 0.0, 0.0).is_some());
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p render 2>&1`
Expected: All tests pass, including the 3 new ones.

- [ ] **Step 7: Commit**

```bash
git add crates/render/src/lib.rs
git commit -m "fix: add allocation_failed flag and clear() to GlyphAtlas for exhaustion recovery"
```

---

### Task 3: End-of-Frame Exhaustion Recovery

**Files:**
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: Add recovery block at end of `App::render`**

In `App::render()` (file `crates/app/src/app_renderer.rs`), after the GPU submit/present block and the timing print, but before the final `Some(())`, add the recovery check.

The insertion point is after line 692 (`output.present();`) and before line 712 (`if self.ui_shell.scrollbar_is_dragging()`). Insert between the timing print block and the post-shape section:

```rust
        // ── Atlas exhaustion recovery ─────────────────────────────────────
        if let Some(text) = &mut self.text {
            if text.atlas.allocation_failed {
                text.atlas.clear();
                for view in &mut self.workspace.views {
                    if let Some(dv) = view.doc_mut() {
                        dv.display.render_cache.invalidate_all();
                    }
                }
                text.preview_cache.invalidate_all();
                text.atlas_generation = text.atlas_generation.wrapping_add(1);
                self.needs_redraw = true;
            }
        }

        // Post-shape: refine autoscroll with visual-line precision
        self.post_shape_update();
```

The context before insertion is the timing block ending around line 704:
```rust
        if _total_render_us > 1000 || _frame_interval_us > 20000 {
            let dv = self.workspace.views.get(self.workspace.active_index).map(|v| v.doc());
            let scroll_y = dv.map(|d| d.display.viewport.scroll_top as usize).unwrap_or(0);
            let vis = dv.map(|d| d.display.viewport.visible_rows).unwrap_or(0);
            println!(
                "[frame] total={:.0}us interval={:.0}us scroll_y={} visible={}",
                _total_render_us, _frame_interval_us, scroll_y, vis,
            );
        }

        // Insert the recovery block here, BEFORE post_shape_update()

        // Post-shape: refine autoscroll with visual-line precision
        self.post_shape_update();
```

- [ ] **Step 2: Build check**

Run: `cargo check -p app 2>&1`
Expected: Compiles without errors. If borrow-checker issues arise from `&mut self.text` + `&mut self.workspace.views`, restructure to:
```rust
let needs_recovery = self.text.as_mut().map_or(false, |t| t.atlas.allocation_failed);
if needs_recovery {
    if let Some(text) = &mut self.text {
        text.atlas.clear();
        text.preview_cache.invalidate_all();
    }
    for view in &mut self.workspace.views {
        if let Some(dv) = view.doc_mut() {
            dv.display.render_cache.invalidate_all();
        }
    }
    if let Some(text) = &mut self.text {
        text.atlas_generation = text.atlas_generation.wrapping_add(1);
    }
    self.needs_redraw = true;
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "fix: recover from glyph atlas exhaustion at end of frame"
```

---

## Final Verification

- [ ] Run full test suite: `cargo test 2>&1`
- [ ] Manual test: open multiple CJK documents, scroll/switch tabs rapidly, confirm text never permanently disappears
