//! Unified text fragment type for widget text rendering.
//!
//! Widgets produce `Vec<TextFragment>` from their `text_positions()` methods.
//! The app layer uses a single `render_text_fragments()` function to shape
//! and produce glyph vertices, replacing per-widget duplicated text rendering.

/// A pre-positioned text label ready for shaping.
/// Coordinates are in **pixel space** (origin top-left).
#[derive(Debug, Clone)]
pub struct TextFragment {
    pub text: String,
    pub x_px: f32,
    pub y_px: f32,
    pub color: [f32; 4],
    pub font_size: f32,
    /// Optional maximum pixel width; text exceeding this is clipped.
    pub max_width_px: Option<f32>,
}
