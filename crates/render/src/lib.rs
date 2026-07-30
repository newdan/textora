//! edit+ render: glyph atlas and wgpu rendering pipeline.
//!
//! Manages a glyph texture atlas and renders shaped text.

use hashlink::LruCache;
use std::collections::HashSet;

/// Subpixel phase for glyph rasterization (0-3, quarter-pixel grid).
pub type SubpixelPhase = u8;

/// Key for a cached glyph in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// Glyph ID from the shaper.
    pub glyph_id: u32,
    /// Font ID (to distinguish glyphs from different fonts in fallback).
    pub font_id: usize,
    /// Font size in pixels (quantized to 64ths).
    pub font_size: u32,
    /// Subpixel x-phase (0-3, quarter-pixel grid from `split_subpixel`).
    pub subpixel_phase: SubpixelPhase,
}

/// Position of a glyph in the atlas texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSlot {
    /// X offset in the atlas texture (pixels).
    pub x: u32,
    /// Y offset in the atlas texture (pixels).
    pub y: u32,
    /// Width of the glyph bitmap (pixels).
    pub width: u32,
    /// Height of the glyph bitmap (pixels).
    pub height: u32,
    /// Which atlas page this glyph lives on.
    pub page: u32,
    /// Horizontal offset from the pen to the glyph bitmap.
    pub bearing_x: f32,
    /// Vertical offset from the baseline to the glyph bitmap.
    pub bearing_y: f32,
}

/// A single page in the glyph atlas.
#[derive(Debug)]
pub struct AtlasPage {
    /// Page index.
    pub index: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Current write cursor (x, y).
    cursor_x: u32,
    cursor_y: u32,
    /// Row height of the current row being packed.
    row_height: u32,
}

impl AtlasPage {
    pub fn new(index: u32, width: u32, height: u32) -> Self {
        Self { index, width, height, cursor_x: 0, cursor_y: 0, row_height: 0 }
    }

    /// Reserve a rectangle at the current position without returning it.
    /// Used to protect special pixels (e.g., cursor white pixel at origin).
    pub fn reserve(&mut self, width: u32, height: u32) {
        let padded_w = width + 1;
        let padded_h = height + 1;
        self.cursor_x += padded_w;
        self.row_height = self.row_height.max(padded_h);
    }

    /// Try to allocate a rectangle on this page. Returns `Some((x, y))` if it fits.
    pub fn allocate(&mut self, glyph_width: u32, glyph_height: u32) -> Option<(u32, u32)> {
        // Add 1px padding between glyphs
        let padded_w = glyph_width + 1;
        let padded_h = glyph_height + 1;

        // Try current row
        if self.cursor_x + padded_w <= self.width {
            if self.cursor_y + padded_h <= self.height {
                let pos = (self.cursor_x, self.cursor_y);
                self.cursor_x += padded_w;
                self.row_height = self.row_height.max(padded_h);
                return Some(pos);
            } else {
                // If it fits horizontally but not vertically, it's too tall for the remaining
                // space on this page.
                return None;
            }
        }

        // Try next row
        let next_y = self.cursor_y + self.row_height;
        if next_y + padded_h <= self.height {
            self.cursor_y = next_y;
            self.cursor_x = padded_w;
            self.row_height = padded_h;
            return Some((0, self.cursor_y));
        }

        None // Page full
    }
}

/// Glyph atlas: manages multiple texture pages for glyph bitmaps.
///
/// Uses a simple shelf-packing algorithm with LRU eviction.
pub struct GlyphAtlas {
    /// Atlas pages.
    pages: Vec<AtlasPage>,
    /// Glyph key → slot mapping with O(1) LRU eviction.
    slots: LruCache<GlyphKey, GlyphSlot>,
    /// Glyphs that are too large for any atlas page (negative cache).
    oversized: HashSet<GlyphKey>,
    /// Maximum number of atlas pages (prevents unbounded GPU memory growth).
    max_pages: usize,
    /// Page dimensions.
    page_width: u32,
    page_height: u32,
    /// True when the atlas is completely full (all pages exhausted).
    pub allocation_failed: bool,
}

impl GlyphAtlas {
    /// Create a new atlas with the given page size and capacity.
    /// Reserves origin pixel (0,0) for cursor/caret rendering.
    pub fn new(page_width: u32, page_height: u32, capacity: usize, max_pages: usize) -> Self {
        let mut first_page = AtlasPage::new(0, page_width, page_height);
        first_page.reserve(1, 1);
        Self {
            max_pages: max_pages.max(1),
            pages: vec![first_page],
            slots: LruCache::new(capacity),
            oversized: HashSet::new(),
            page_width,
            page_height,
            allocation_failed: false,
        }
    }

    /// Look up a glyph in the atlas.
    pub fn get(&mut self, key: &GlyphKey) -> Option<&GlyphSlot> {
        self.slots.get(key)
    }

    /// Insert a glyph into the atlas. Returns the slot.
    /// Evicts LRU entries if at capacity.
    pub fn insert(
        &mut self,
        key: GlyphKey,
        width: u32,
        height: u32,
        bearing_x: f32,
        bearing_y: f32,
    ) -> Option<GlyphSlot> {
        // Skip glyphs known to be too large for any page.
        if self.oversized.contains(&key) {
            return None;
        }

        // Try to allocate on existing pages
        for page in &mut self.pages {
            if let Some((x, y)) = page.allocate(width, height) {
                let slot =
                    GlyphSlot { x, y, width, height, page: page.index, bearing_x, bearing_y };
                self.slots.insert(key, slot);
                return Some(slot);
            }
        }

        // All pages full — create a new page (respect max_pages limit)
        let page_index = self.pages.len() as u32;
        if self.pages.len() >= self.max_pages {
            eprintln!(
                "[atlas] exhausted ({}/{} pages), triggering clear",
                self.pages.len(),
                self.max_pages
            );
            self.allocation_failed = true;
            return None;
        }
        let mut new_page = AtlasPage::new(page_index, self.page_width, self.page_height);
        if let Some((x, y)) = new_page.allocate(width, height) {
            let slot = GlyphSlot { x, y, width, height, page: page_index, bearing_x, bearing_y };
            self.pages.push(new_page);
            self.slots.insert(key, slot);
            Some(slot)
        } else {
            {
                self.oversized.insert(key);
                None // Glyph too large for a page
            }
        }
    }

    /// Insert a glyph and return any evicted key (for RenderCache invalidation).
    /// Uses manual LRU eviction to track which glyph was removed.
    pub fn insert_with_eviction(
        &mut self,
        key: GlyphKey,
        width: u32,
        height: u32,
        bearing_x: f32,
        bearing_y: f32,
    ) -> (Option<GlyphSlot>, Option<GlyphKey>) {
        // Pre-evict LRU if at capacity to make room (and track evicted key)
        let evicted_key = if self.slots.len() >= self.slots.capacity() {
            self.slots.remove_lru().map(|(k, _)| k)
        } else {
            None
        };

        let slot = self.insert(key, width, height, bearing_x, bearing_y);

        (slot, evicted_key)
    }

    /// Number of pages in the atlas.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Number of glyphs in the atlas.
    pub fn glyph_count(&self) -> usize {
        self.slots.len()
    }
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

    /// Number of subpixel phases supported.
    pub const SUBPIXEL_PHASES: u8 = 8;

    /// Quantize a subpixel x-offset to a phase (0-7).
    pub fn quantize_subpixel(x_subpixel: f32) -> SubpixelPhase {
        let phase = (x_subpixel * 8.0).round() as i32 % 8;
        (phase & 7) as SubpixelPhase
    }
}

/// Split a coordinate into integer pixel position and subpixel phase (0-3).
///
/// Quantizes to 1/4-pixel grid: `(coord * 4.0).round() / 4.0`, then returns the
/// integer-truncated position and the fractional phase (0..4).
pub fn split_subpixel(coord: f32) -> (f32, u8) {
    let sub = (coord * 4.0).round() / 4.0;
    let int_part = sub.floor();
    let phase = ((sub - int_part) * 4.0) as u8;
    (int_part, phase)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AtlasPage tests ────────────────────────────────────────────────────

    #[test]
    fn page_allocate_basic() {
        let mut page = AtlasPage::new(0, 256, 256);
        assert_eq!(page.allocate(10, 10), Some((0, 0)));
        assert_eq!(page.allocate(10, 10), Some((11, 0))); // +1 padding
    }

    #[test]
    fn page_allocate_next_row() {
        let mut page = AtlasPage::new(0, 32, 32);
        // First glyph: row 0
        assert_eq!(page.allocate(20, 10), Some((0, 0)));
        // Second glyph: does not fit on row 0 (21+21=42 > 32), goes to row 1
        assert!(page.allocate(20, 10).is_some());
    }

    #[test]
    fn page_allocate_overflow() {
        let mut page = AtlasPage::new(0, 16, 16);
        // First glyph fits
        assert!(page.allocate(10, 10).is_some());
        // Second glyph: row 0 full (11+11=22 > 16), row 1 too (11+11=22 > 16)
        assert!(page.allocate(10, 10).is_none());
    }

    // ── GlyphAtlas tests ───────────────────────────────────────────────────

    #[test]
    fn glyph_atlas_lru_eviction() {
        let mut atlas = GlyphAtlas::new(256, 256, 3, 4);

        let key1 = GlyphKey { glyph_id: 1, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
        let key2 = GlyphKey { glyph_id: 2, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
        let key3 = GlyphKey { glyph_id: 3, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
        let key4 = GlyphKey { glyph_id: 4, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };

        assert!(atlas.insert(key1, 10, 10, 0.0, 0.0).is_some());
        assert!(atlas.insert(key2, 10, 10, 0.0, 0.0).is_some());
        assert!(atlas.insert(key3, 10, 10, 0.0, 0.0).is_some());

        // All 3 are in the atlas
        assert_eq!(atlas.glyph_count(), 3);
        assert!(atlas.get(&key1).is_some());

        // Insert key4 — should evict key2 (LRU, since key1 was accessed recently)
        assert!(atlas.insert(key4, 10, 10, 0.0, 0.0).is_some());
        assert_eq!(atlas.glyph_count(), 3);
        assert!(atlas.get(&key2).is_none(), "key2 should have been evicted");
        assert!(atlas.get(&key3).is_some());
        assert!(atlas.get(&key4).is_some());
    }

    #[test]
    fn atlas_overflow_creates_new_page() {
        // Tiny pages: 32x32. Each glyph 20x10 (+1 padding = 21x11).
        // One page can fit 1 glyph per row, 2 rows = 2 glyphs.
        // Third glyph needs a new page.
        let mut atlas = GlyphAtlas::new(32, 32, 100, 4);

        let key1 = GlyphKey { glyph_id: 1, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
        let key2 = GlyphKey { glyph_id: 2, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
        let key3 = GlyphKey { glyph_id: 3, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };

        assert!(atlas.insert(key1, 20, 10, 0.0, 0.0).is_some());
        assert_eq!(atlas.page_count(), 1);

        assert!(atlas.insert(key2, 20, 10, 0.0, 0.0).is_some());
        assert_eq!(atlas.page_count(), 1); // fits on same page (second row)

        // Third glyph: page 0 has row 0 (y=0, h=11) and row 1 (y=11, h=11).
        // next_y = 11+11 = 22. 22+11 = 33 > 32. Page 0 full.
        assert!(atlas.insert(key3, 20, 10, 0.0, 0.0).is_some());
        assert_eq!(atlas.page_count(), 2, "should have created a second page");
    }

    #[test]
    fn atlas_subpixel_phases() {
        let mut atlas = GlyphAtlas::new(256, 256, 100, 4);

        // Insert the same glyph with all 8 subpixel phases
        for phase in 0..8u8 {
            let key = GlyphKey {
                glyph_id: 42,
                font_id: 0usize,
                font_size: 14 * 64,
                subpixel_phase: phase,
            };
            assert!(atlas.insert(key, 10, 10, 0.0, 0.0).is_some(), "phase {phase} failed");
        }

        assert_eq!(atlas.glyph_count(), 8, "should have 8 entries (one per phase)");

        // Verify all phases are present
        for phase in 0..8u8 {
            let key = GlyphKey {
                glyph_id: 42,
                font_id: 0usize,
                font_size: 14 * 64,
                subpixel_phase: phase,
            };
            assert!(atlas.get(&key).is_some(), "phase {phase} missing");
        }
    }

    #[test]
    fn subpixel_quantize() {
        assert_eq!(GlyphAtlas::quantize_subpixel(0.0), 0);
        assert_eq!(GlyphAtlas::quantize_subpixel(0.5), 4);
        assert_eq!(GlyphAtlas::quantize_subpixel(0.125), 1);
        assert_eq!(GlyphAtlas::quantize_subpixel(0.875), 7);
    }

    #[test]
    fn atlas_insert_and_lookup() {
        let mut atlas = GlyphAtlas::new(256, 256, 100, 4);
        let key = GlyphKey { glyph_id: 1, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };

        let slot = atlas.insert(key, 10, 12, 1.5, -2.0).unwrap();
        assert_eq!(slot.width, 10);
        assert_eq!(slot.height, 12);
        assert_eq!(slot.bearing_x, 1.5);
        assert_eq!(slot.bearing_y, -2.0);
        assert_eq!(slot.page, 0);

        let looked_up = atlas.get(&key).unwrap();
        assert_eq!(*looked_up, slot);
    }

    #[test]
    fn oversized_glyph_cached_as_negative() {
        // Tiny page: 32x32. Glyph 40x40 is too large for any page.
        let mut atlas = GlyphAtlas::new(32, 32, 100, 4);
        let key = GlyphKey { glyph_id: 99, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };

        // First attempt: creates a page, glyph doesn't fit, returns None.
        assert!(atlas.insert(key, 40, 40, 0.0, 0.0).is_none());
        let pages_after_first = atlas.page_count();

        // Second attempt: should NOT create another page (negative cached).
        assert!(atlas.insert(key, 40, 40, 0.0, 0.0).is_none());
        assert_eq!(
            atlas.page_count(),
            pages_after_first,
            "oversized glyph should not create new pages on retry"
        );
    }

    #[test]
    fn glyph_key_different_font_ids_are_different() {
        // Bug fix: GlyphKey must include font_id to prevent atlas collision
        // between different fonts that happen to have the same glyph_id.
        // e.g., Menlo '6' (glyph_id=22) and a CJK fallback with glyph_id=22.
        let key_a =
            GlyphKey { glyph_id: 22, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
        let key_b =
            GlyphKey { glyph_id: 22, font_id: 1usize, font_size: 14 * 64, subpixel_phase: 0 };
        assert_ne!(key_a, key_b, "same glyph_id but different font_id must be distinct keys");

        // Both should be insertable and retrievable independently
        let mut atlas = GlyphAtlas::new(256, 256, 100, 4);
        assert!(atlas.insert(key_a, 10, 10, 0.0, 0.0).is_some());
        assert!(atlas.insert(key_b, 12, 14, 1.0, -1.0).is_some());
        assert_eq!(atlas.glyph_count(), 2);

        let width_a = atlas.get(&key_a).unwrap().width;
        let width_b = atlas.get(&key_b).unwrap().width;
        assert_eq!(width_a, 10);
        assert_eq!(width_b, 12, "different fonts should store different glyph bitmaps");
    }

    #[test]
    fn max_pages_prevents_unbounded_growth() {
        // Tiny page (32x32) and max_pages=2. Glyphs are 10x10.
        // Each page can hold ~6 glyphs (32/11 ≈ 2 per row, ~3 rows).
        // After 2 pages fill, new insertions should return None.
        let mut atlas = GlyphAtlas::new(32, 32, 20, 2);
        let mut count = 0;
        for i in 0..20u32 {
            let key = GlyphKey { glyph_id: i, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
            if atlas.insert(key, 10, 10, 0.0, 0.0).is_some() {
                count += 1;
            }
        }
        // Should have created at most 2 pages
        assert!(
            atlas.page_count() <= 2,
            "max_pages limit violated: got {} pages",
            atlas.page_count()
        );
        // Some glyphs should have been inserted successfully
        assert!(count > 0, "at least some glyphs should fit");
    }

    #[test]
    fn max_pages_one_limits_to_single_page() {
        let mut atlas = GlyphAtlas::new(256, 256, 100, 1);
        assert_eq!(atlas.page_count(), 1);
        let key = GlyphKey { glyph_id: 99, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
        assert!(atlas.insert(key, 200, 200, 0.0, 0.0).is_some());
        assert_eq!(atlas.page_count(), 1, "should stay at 1 page");
    }

    #[test]
    fn split_subpixel_exact_integer() {
        assert_eq!(split_subpixel(10.0), (10.0, 0));
        assert_eq!(split_subpixel(0.0), (0.0, 0));
    }

    #[test]
    fn split_subpixel_quarter() {
        assert_eq!(split_subpixel(10.25), (10.0, 1));
        assert_eq!(split_subpixel(10.5), (10.0, 2));
        assert_eq!(split_subpixel(10.75), (10.0, 3));
    }

    #[test]
    fn split_subpixel_rounding() {
        // 10.3 → 10.25 (quantized) → int=10, phase=1
        let (int_part, phase) = split_subpixel(10.3);
        assert_eq!(int_part, 10.0);
        assert_eq!(phase, 1);
        // 10.1 → 10.0
        let (int_part, phase) = split_subpixel(10.1);
        assert_eq!(int_part, 10.0);
        assert_eq!(phase, 0);
    }

    #[test]
    fn split_subpixel_negative() {
        // -0.3 → -0.25 (quantized) → floor = -1.0, phase = 3
        let (int_part, phase) = split_subpixel(-0.3);
        assert_eq!(int_part, -1.0);
        assert_eq!(phase, 3);
        // -0.5 → -0.5 → floor = -1.0, phase = 2
        let (int_part, phase) = split_subpixel(-0.5);
        assert_eq!(int_part, -1.0);
        assert_eq!(phase, 2);
    }

    #[test]
    fn split_subpixel_boundary_075() {
        assert_eq!(split_subpixel(10.75), (10.0, 3));
        // 10.875 → quantized to 10.75 → phase 3
        let (int_part, phase) = split_subpixel(10.875);
        assert_eq!(int_part, 11.0);
        assert_eq!(phase, 0);
    }

    #[test]
    fn split_subpixel_large_value() {
        let (int_part, phase) = split_subpixel(12345.5);
        assert_eq!(int_part, 12345.0);
        assert_eq!(phase, 2);
    }

    #[test]
    fn gamma_uniform_size() {
        // GammaUniform has two f32 fields: contrast + gamma = 8 bytes
        assert_eq!(std::mem::size_of::<GammaUniform>(), 8);
        assert_eq!(std::mem::align_of::<GammaUniform>(), 4);
    }
    #[test]
    fn allocation_failed_set_when_max_pages_exhausted() {
        // Tiny page (32x32), max_pages=1. Each glyph 10x10.
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

    #[test]
    fn insert_with_eviction_propagates_allocation_failed() {
        // Tiny atlas: 1 page, 3 capacity. insert_with_eviction calls insert() internally.
        let mut atlas = GlyphAtlas::new(32, 32, 10, 1);
        for i in 0..20u32 {
            let key = GlyphKey { glyph_id: i, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
            atlas.insert_with_eviction(key, 10, 10, 0.0, 0.0);
        }
        assert!(atlas.allocation_failed, "insert_with_eviction should propagate the flag");
    }

    #[test]
    fn clear_resets_multi_page_atlas() {
        // max_pages=3, fill enough to expand to multiple pages, then clear.
        let mut atlas = GlyphAtlas::new(64, 64, 100, 3);
        for i in 0..50u32 {
            let key = GlyphKey { glyph_id: i, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
            atlas.insert(key, 10, 10, 0.0, 0.0);
        }
        let pages_before = atlas.page_count();
        assert!(pages_before > 1, "should have expanded to multiple pages, got {pages_before}");

        atlas.clear();

        assert_eq!(atlas.page_count(), 1);
        assert_eq!(atlas.glyph_count(), 0);
        assert!(!atlas.allocation_failed);
    }
}

// ── GPU Renderer ───────────────────────────────────────────────────────────

/// Vertex for glyph rendering.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphVertex {
    pub position: [f32; 2],
    pub tex_coords: [f32; 2],
    pub color: [f32; 4],
}

/// Gamma correction uniform for dynamic contrast enhancement.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GammaUniform {
    /// Contrast enhancement strength (0.0 = none, 1.0 = full).
    pub contrast: f32,
    /// Gamma correction exponent (1.0 = none, 2.2 = sRGB).
    pub gamma: f32,
}

/// GPU glyph renderer.
///
/// Manages a texture atlas on the GPU and renders shaped text as textured quads.
pub struct GlyphRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl GlyphRenderer {
    /// Create a new glyph renderer for the given surface format.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyph shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyph pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 4, ..Default::default() },
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self { pipeline, bind_group_layout, sampler }
    }

    /// Generate vertices for a list of glyph positions.
    /// Returns triangles (2 triangles per glyph = 6 vertices per glyph).
    pub fn generate_vertices(
        glyphs: &[(GlyphSlot, f32, f32)],
        atlas_width: u32,
        atlas_height: u32,
        screen_width: f32,
        screen_height: f32,
        color: [f32; 4],
    ) -> Vec<GlyphVertex> {
        let mut vertices = Vec::with_capacity(glyphs.len() * 6);

        for &(slot, x, y) in glyphs {
            let px = x.round();
            let py = y.round();
            let left = (px + slot.bearing_x) / screen_width * 2.0 - 1.0;
            let top = 1.0 - (py - slot.bearing_y) / screen_height * 2.0;
            let right = (px + slot.bearing_x + slot.width as f32) / screen_width * 2.0 - 1.0;
            let bottom = 1.0 - (py - slot.bearing_y + slot.height as f32) / screen_height * 2.0;

            let uv_left = slot.x as f32 / atlas_width as f32;
            let uv_top = slot.y as f32 / atlas_height as f32;
            let uv_right = (slot.x + slot.width) as f32 / atlas_width as f32;
            let uv_bottom = (slot.y + slot.height) as f32 / atlas_height as f32;

            // Two triangles per glyph
            vertices.push(GlyphVertex {
                position: [left, top],
                tex_coords: [uv_left, uv_top],
                color,
            });
            vertices.push(GlyphVertex {
                position: [right, top],
                tex_coords: [uv_right, uv_top],
                color,
            });
            vertices.push(GlyphVertex {
                position: [left, bottom],
                tex_coords: [uv_left, uv_bottom],
                color,
            });
            vertices.push(GlyphVertex {
                position: [right, top],
                tex_coords: [uv_right, uv_top],
                color,
            });
            vertices.push(GlyphVertex {
                position: [right, bottom],
                tex_coords: [uv_right, uv_bottom],
                color,
            });
            vertices.push(GlyphVertex {
                position: [left, bottom],
                tex_coords: [uv_left, uv_bottom],
                color,
            });
        }

        vertices
    }

    /// Get the bind group layout for creating custom bind groups.
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    /// Get the sampler.
    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Get the render pipeline.
    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }
}

/// WGSL shader for glyph rendering.
const SHADER_SRC: &str = r#"
fn color_brightness(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.299, 0.587, 0.114));
}

fn enhance_contrast(alpha: f32, k: f32) -> f32 {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    return out;
}

struct GammaUniform {
    contrast: f32,
    gamma: f32,
};

@group(0) @binding(0) var atlas_texture: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var<uniform> gamma_params: GammaUniform;

fn light_on_dark_contrast(base: f32, text_rgb: vec3<f32>) -> f32 {
    // Dark text on light backgrounds gets full contrast enhancement;
    // light text on dark backgrounds gets reduced enhancement to avoid over-bolding.
    let text_brightness = color_brightness(text_rgb);
    return mix(base, base * 0.3, text_brightness);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas_texture, atlas_sampler, in.tex_coords).r;

    // Dynamic contrast: brightness-aware stem darkening.
    // Dark text on light backgrounds gets full contrast enhancement;
    // light text on dark backgrounds gets reduced enhancement to avoid over-bolding.
    let dilation = light_on_dark_contrast(gamma_params.contrast, in.color.rgb);
    let alpha_corrected = enhance_contrast(coverage, dilation);

    // Gamma correction to counter sRGB linear blending.
    let final_alpha = pow(alpha_corrected, 1.0 / gamma_params.gamma);
    return vec4<f32>(in.color.rgb, in.color.a * final_alpha);
}
"#;

#[cfg(test)]
mod renderer_tests {
    use super::*;

    #[test]
    fn generate_vertices_basic() {
        let slot = GlyphSlot {
            x: 0,
            y: 0,
            width: 10,
            height: 12,
            page: 0,
            bearing_x: 1.0,
            bearing_y: 10.0,
        };
        let glyphs = vec![(slot, 100.0, 200.0)];
        let verts =
            GlyphRenderer::generate_vertices(&glyphs, 256, 256, 800.0, 600.0, [1.0, 1.0, 1.0, 1.0]);

        // 2 triangles = 6 vertices per glyph
        assert_eq!(verts.len(), 6);

        // All vertices should have the specified color
        for v in &verts {
            assert_eq!(v.color, [1.0, 1.0, 1.0, 1.0]);
        }

        // UV coords should be in [0, 1]
        for v in &verts {
            assert!(v.tex_coords[0] >= 0.0 && v.tex_coords[0] <= 1.0);
            assert!(v.tex_coords[1] >= 0.0 && v.tex_coords[1] <= 1.0);
        }
    }

    #[test]
    fn generate_vertices_multiple_glyphs() {
        let slot =
            GlyphSlot { x: 0, y: 0, width: 8, height: 10, page: 0, bearing_x: 0.0, bearing_y: 8.0 };
        let glyphs: Vec<_> = (0..5).map(|i| (slot, i as f32 * 10.0, 50.0)).collect();
        let verts =
            GlyphRenderer::generate_vertices(&glyphs, 256, 256, 800.0, 600.0, [1.0, 0.0, 0.0, 1.0]);

        assert_eq!(verts.len(), 30); // 5 glyphs * 6 vertices
    }

    #[test]
    fn vertex_size_is_correct() {
        assert_eq!(std::mem::size_of::<GlyphVertex>(), 32); // 2+2+4 floats * 4 bytes
    }

    fn with_gpu(f: impl FnOnce(&wgpu::Device, wgpu::TextureFormat)) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            backends: wgpu::Backends::PRIMARY,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: true,
            ..Default::default()
        }));
        let Ok(adapter) = adapter else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("test device"),
                ..Default::default()
            }))
            .expect("device creation failed");

        f(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn renderer_creation() {
        with_gpu(|device, format| {
            let renderer = GlyphRenderer::new(device, format);
            // Just verify it was created (pipeline, bind group layout exist)
            let _ = renderer.pipeline();
            let _ = renderer.bind_group_layout();
            let _ = renderer.sampler();
        });
    }

    #[test]
    fn glyph_atlas_insert_with_eviction_tracks_evicted_key() {
        let mut atlas = GlyphAtlas::new(256, 256, 3, 4);

        let k1 = GlyphKey { glyph_id: 1, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
        let k2 = GlyphKey { glyph_id: 2, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
        let k3 = GlyphKey { glyph_id: 3, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };
        let k4 = GlyphKey { glyph_id: 4, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };

        let (s1, e1) = atlas.insert_with_eviction(k1, 10, 10, 0.0, 0.0);
        assert!(s1.is_some());
        assert!(e1.is_none()); // cache not full yet

        let (s2, e2) = atlas.insert_with_eviction(k2, 10, 10, 0.0, 0.0);
        assert!(s2.is_some());
        assert!(e2.is_none());

        let (s3, e3) = atlas.insert_with_eviction(k3, 10, 10, 0.0, 0.0);
        assert!(s3.is_some());
        assert!(e3.is_none());

        // Now at capacity (3/3). Inserting k4 should evict k1 (LRU).
        let (s4, e4) = atlas.insert_with_eviction(k4, 10, 10, 0.0, 0.0);
        assert!(s4.is_some());
        assert!(e4.is_some());
        assert_eq!(e4.unwrap().glyph_id, 1); // k1 was LRU

        // k1 should be gone
        assert!(atlas.get(&k1).is_none());
    }

    #[test]
    fn glyph_atlas_insert_with_eviction_update_no_eviction() {
        let mut atlas = GlyphAtlas::new(256, 256, 3, 4);

        let k1 = GlyphKey { glyph_id: 1, font_id: 0, font_size: 14 * 64, subpixel_phase: 0 };

        // Insert first time
        let (s1, e1) = atlas.insert_with_eviction(k1, 10, 10, 0.0, 0.0);
        assert!(s1.is_some());
        assert!(e1.is_none());

        // Update same key — no eviction
        let (s2, e2) = atlas.insert_with_eviction(k1, 20, 20, 1.0, 1.0);
        assert!(s2.is_some());
        assert!(e2.is_none());
    }
}
