//! Markdown rendering for edit+.

pub mod augmenter;
pub mod builder;
pub mod commands;
pub mod edit;
#[cfg(feature = "rich-markdown")]
pub mod embedded;
#[cfg(not(feature = "rich-markdown"))]
#[path = "embedded_lite.rs"]
pub mod embedded;
pub mod grapheme_map;
pub mod layout;
pub mod parser;
pub mod paste;
pub(crate) mod projection;
pub mod render;
pub mod search;
pub mod selection;
pub mod style;
pub mod typography;
pub mod view;

pub mod mindmap_view;
pub mod mmf;

use ui::core::paint::DrawList;

/// Clamp a byte index to a valid char boundary in `s` (snap left).
/// In debug builds, fires an assertion so invalid offsets are caught early;
/// in release builds, silently falls back to the nearest safe boundary.
#[inline]
pub(crate) fn safe_byte_idx(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        s.len()
    } else {
        debug_assert!(
            s.is_char_boundary(idx),
            "byte index {idx} is not a char boundary in string of len {}",
            s.len()
        );
        s.floor_char_boundary(idx)
    }
}

/// Render markdown source text into a DrawList.
///
/// This is the main public API — calls the full pipeline:
/// parse → build → layout → render.
pub fn render_markdown(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
) -> DrawList {
    let mut shaper = shaping::Shaper::new().ok();
    render_markdown_with_shaper(src, style, viewport_w, viewport_h, scroll_y, shaper.as_mut())
}

/// Render with optional shaper for precise text measurement.
pub fn render_markdown_with_shaper(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    shaper: Option<&mut shaping::Shaper>,
) -> DrawList {
    render_markdown_with_offset(src, style, viewport_w, viewport_h, scroll_y, shaper, 0.0, 0.0)
}

/// Render with pixel offset for positioning inside editor content area.
#[allow(clippy::too_many_arguments)]
pub fn render_markdown_with_offset(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    shaper: Option<&mut shaping::Shaper>,
    offset_x: f32,
    offset_y: f32,
) -> DrawList {
    render_markdown_with_highlighter(
        src, style, viewport_w, viewport_h, scroll_y, shaper, offset_x, offset_y, None,
    )
}

/// Render with optional syntax highlighting for code blocks.
#[allow(clippy::too_many_arguments)]
pub fn render_markdown_with_highlighter(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
    offset_x: f32,
    offset_y: f32,
    highlighter: Option<&dyn builder::CodeHighlighter>,
) -> DrawList {
    let parsed = parser::parse_markdown(src);
    let doc = builder::MarkdownDoc::build(&parsed, style);
    let string_doc = core::document::StringDocView::new(src);
    let layout = layout::layout_doc_with_shaper_for_rendering(
        &doc.blocks,
        style,
        viewport_w,
        shaper.as_deref_mut(),
        highlighter,
        &string_doc,
    );
    let mut dl = DrawList::new();
    render::render_layout_with_offset(
        &layout,
        style,
        &mut dl,
        scroll_y,
        viewport_h,
        offset_x,
        offset_y,
        shaper,
        &[],
    );
    dl
}

/// Get the total content height for a markdown document (for scrollbar calculations).
pub fn markdown_content_height(src: &str, style: &style::MarkdownStyle, viewport_w: f32) -> f32 {
    let parsed = parser::parse_markdown(src);
    let doc = builder::MarkdownDoc::build(&parsed, style);
    let string_doc = core::document::StringDocView::new(src);
    let laid_out = layout::layout_doc(&doc.blocks, style, viewport_w, &string_doc);
    laid_out.total_height
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::style::MarkdownStyle;

    /// Default dark-theme style for testing (matches ui::theme::test_theme() + 15px/24px).
    pub fn default_style() -> MarkdownStyle {
        let theme = ui::theme::test_theme();
        MarkdownStyle::from_theme(&theme, 15.0, 24.0)
    }
}

#[cfg(test)]
mod lib_tests;
