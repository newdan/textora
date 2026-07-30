use crate::theme::Theme;
use render::{GlyphKey, GlyphVertex};
use shaping;

/// Atlas texture size (must match the render state).
pub const ATLAS_SIZE: u32 = 4096;

/// Rendering context passed to widget render functions.
pub struct RenderContext<'a> {
    pub theme: &'a Theme,
    pub screen_w: f32,
    pub screen_h: f32,
    pub left_margin: f32,
    pub tab_bar_height: f32,
    pub is_active_tab: bool,
    pub gutter_width: f32,
    /// IME preedit: total pixel advance of composing text (0 when inactive).
    pub preedit_advance_px: f32,
    /// IME preedit: cursor byte column — clusters at/after this offset are shifted.
    pub preedit_cursor_col: usize,
}

/// Format a 1-based line number into a stack buffer (no allocation).
#[inline]
pub fn format_line_num(n: usize) -> ([u8; 8], usize) {
    let mut buf = [0u8; 8];
    let mut i = 8;
    let mut m = n;
    if m == 0 {
        buf[7] = b'0';
        return (buf, 1);
    }
    while m > 0 {
        i -= 1;
        buf[i] = b'0' + (m % 10) as u8;
        m /= 10;
    }
    (buf, 8 - i)
}

/// Get a &str from format_line_num result.
#[inline]
pub fn line_num_str(buf: &([u8; 8], usize)) -> &str {
    std::str::from_utf8(&buf.0[8 - buf.1..]).unwrap()
}

/// Generate line number vertices for a single line in the gutter.
pub fn generate_line_number_vertices(
    ctx: &RenderContext,
    shaped: &shaping::ShapedRun,
    atlas: &mut render::GlyphAtlas,
    atlas_texture: &wgpu::Texture,
    queue: &wgpu::Queue,
    shaper: &mut shaping::Shaper,
    font_size: f32,
    line_y: f32,
    is_active: bool,
    settings_gutter_padding: f32,
    settings_line_height: f32,
) -> Vec<GlyphVertex> {
    let text_color =
        if is_active { ctx.theme.editor.foreground } else { ctx.theme.editor.line_number };

    let letter_spacing = -0.12 * font_size;
    let raw_total_advance: f32 = shaped.clusters.iter().map(|c| c.advance.max(1.0)).sum();
    let n = shaped.clusters.len();
    let total_spacing = if n > 1 { letter_spacing * (n - 1) as f32 } else { 0.0 };
    let adjusted_total_width = raw_total_advance + total_spacing;

    let start_x = ctx.left_margin - settings_gutter_padding - adjusted_total_width;
    let y_base = line_y + settings_line_height * 0.8 + ctx.tab_bar_height;

    let mut vertices = Vec::new();
    let mut x_cursor = start_x;

    for cluster in &shaped.clusters {
        let glyph_id = cluster.glyph_id as u16;
        let font_id = cluster.font_id;
        let font_id_usize = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            font_id.hash(&mut h);
            h.finish() as usize
        };
        let advance = cluster.advance.max(1.0);
        let key = GlyphKey {
            glyph_id: cluster.glyph_id,
            font_id: font_id_usize,
            font_size: (font_size * 64.0) as u32,
            subpixel_phase: 0,
        };

        let slot = if let Some(cached) = atlas.get(&key) {
            *cached
        } else if let Some(bitmap) =
            shaper.rasterize_glyph(font_id, glyph_id, font_size, (0.0, 0.0))
        {
            if bitmap.width > 0 && bitmap.height > 0 {
                if let Some(s) = atlas.insert(
                    key,
                    bitmap.width,
                    bitmap.height,
                    bitmap.left as f32,
                    bitmap.top as f32,
                ) {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: atlas_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x: s.x, y: s.y, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &bitmap.data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(bitmap.width),
                            rows_per_image: Some(bitmap.height),
                        },
                        wgpu::Extent3d {
                            width: bitmap.width,
                            height: bitmap.height,
                            depth_or_array_layers: 1,
                        },
                    );
                    s
                } else {
                    x_cursor += advance;
                    continue;
                }
            } else {
                x_cursor += advance;
                continue;
            }
        } else {
            x_cursor += advance;
            continue;
        };

        let px = (x_cursor + slot.bearing_x).round();
        let py = (y_base - slot.bearing_y).round();
        let left = px / ctx.screen_w * 2.0 - 1.0;
        let right = (px + slot.width as f32) / ctx.screen_w * 2.0 - 1.0;
        let top = 1.0 - py / ctx.screen_h * 2.0;
        let bottom = 1.0 - (py + slot.height as f32) / ctx.screen_h * 2.0;
        let uv_left = slot.x as f32 / ATLAS_SIZE as f32;
        let uv_top = slot.y as f32 / ATLAS_SIZE as f32;
        let uv_right = (slot.x + slot.width) as f32 / ATLAS_SIZE as f32;
        let uv_bottom = (slot.y + slot.height) as f32 / ATLAS_SIZE as f32;

        let color = text_color;
        vertices.push(GlyphVertex { position: [left, top], tex_coords: [uv_left, uv_top], color });
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

        x_cursor += advance + letter_spacing;
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_num_zero() {
        let (buf, len) = format_line_num(0);
        assert_eq!(len, 1);
        assert_eq!(line_num_str(&(buf, len)), "0");
    }

    #[test]
    fn format_line_num_single_digit() {
        let (buf, len) = format_line_num(7);
        assert_eq!(len, 1);
        assert_eq!(line_num_str(&(buf, len)), "7");
    }

    #[test]
    fn format_line_num_double_digit() {
        let (buf, len) = format_line_num(42);
        assert_eq!(len, 2);
        assert_eq!(line_num_str(&(buf, len)), "42");
    }

    #[test]
    fn format_line_num_triple_digit() {
        let (buf, len) = format_line_num(999);
        assert_eq!(len, 3);
        assert_eq!(line_num_str(&(buf, len)), "999");
    }

    #[test]
    fn format_line_num_large() {
        let (buf, len) = format_line_num(12345678);
        assert_eq!(len, 8);
        assert_eq!(line_num_str(&(buf, len)), "12345678");
    }

    #[test]
    fn format_line_num_boundary_10() {
        assert_eq!(line_num_str(&format_line_num(9)), "9");
        assert_eq!(line_num_str(&format_line_num(10)), "10");
    }

    #[test]
    fn format_line_num_boundary_100() {
        assert_eq!(line_num_str(&format_line_num(99)), "99");
        assert_eq!(line_num_str(&format_line_num(100)), "100");
    }
}
