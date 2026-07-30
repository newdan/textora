//! Unified glyph resolution: atlas lookup → rasterize → upload → return slot.
//!
//! Extracted from duplicated logic in `render_pipeline.rs` and `paint_backend.rs`.

use render::{GlyphAtlas, GlyphKey, GlyphSlot};
use shaping::FontId;
use shaping::Shaper;

/// Look up a glyph in the atlas, or rasterize + upload it on cache miss.
///
/// Returns `Some(GlyphSlot)` with position in the atlas texture, or `None` if
/// rasterization failed, the glyph bitmap was empty, or atlas insertion failed.
pub fn resolve_glyph(
    font_id: FontId,
    glyph_id: u16,
    font_size: f32,
    subpixel_phase: u8,
    shaper: &mut Shaper,
    atlas: &mut GlyphAtlas,
    atlas_texture: &wgpu::Texture,
    queue: &wgpu::Queue,
) -> Option<GlyphSlot> {
    // Font IDs from cosmic-text are platform handles (e.g. pointer-sized).
    // Hash them into a stable usize for the atlas key.
    let font_id_usize = {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        font_id.hash(&mut h);
        h.finish() as usize
    };
    let key = GlyphKey {
        glyph_id: glyph_id as u32,
        font_id: font_id_usize,
        font_size: (font_size * 64.0) as u32,
        subpixel_phase,
    };

    // Cache hit
    if let Some(cached) = atlas.get(&key) {
        return Some(*cached);
    }

    // Cache miss: rasterize
    let subpixel_x = subpixel_phase as f32 * 0.25;
    let bitmap = shaper.rasterize_glyph(font_id, glyph_id, font_size, (subpixel_x, 0.0))?;
    if bitmap.width == 0 || bitmap.height == 0 {
        return None;
    }

    // Insert into atlas
    let slot =
        atlas.insert(key, bitmap.width, bitmap.height, bitmap.left as f32, bitmap.top as f32)?;

    // Upload bitmap data to GPU texture
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: atlas_texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: slot.x, y: slot.y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &bitmap.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bitmap.width),
            rows_per_image: Some(bitmap.height),
        },
        wgpu::Extent3d { width: bitmap.width, height: bitmap.height, depth_or_array_layers: 1 },
    );

    Some(slot)
}
