//! Rendering for the Markdown editing view.

use super::*;
use std::ops::Range;

pub(super) fn render_engine(
    engine: &mut BlockViewLayout,
    source: &MarkdownSource<'_>,
    composing_range: Option<Range<usize>>,
    bounds: ui::Rect,
    theme: &Theme,
    shaper: &mut shaping::Shaper,
    dpi_scale: f32,
) -> DrawList {
    let settings = MarkdownRenderSettings {
        font_family: &engine.base_font_family,
        font_size: engine.base_font_size * dpi_scale,
        line_height: engine.base_line_height * dpi_scale,
        toc_max_depth: engine.toc_max_depth,
        markdown_first_line_indent: engine.markdown_first_line_indent,
        text_spacing_mode: engine.text_spacing_mode,
    };
    let style = settings.style_at_dpi(theme, dpi_scale);
    engine.toc_max_depth = settings.toc_max_depth;
    // A changed width forces new geometry. The old layout will not reconcile its
    // blocks at a different width, so its exact-source editable tree can move.
    let reuse_structure = if composing_range.is_none() && bounds.w != engine.cached_viewport_w {
        engine.lazy.as_mut().and_then(|layout| layout.take_editor_structure_for_reflow(source))
    } else {
        None
    };
    let string_doc = textora_core::document::StringDocView::new(source);
    let (mut dl, _) = engine.render(
        theme,
        bounds.w,
        bounds.h,
        bounds.x,
        bounds.y,
        &style,
        |_| {
            reuse_structure.unwrap_or_else(|| {
                let document = composing_range.as_ref().map_or_else(
                    || source.layout_document(),
                    |range| preedit::build_composition_document(source, range),
                );
                document.with_editable_paragraphs(source)
            })
        },
        Some(shaper),
        &string_doc,
        true,  // editing keeps whole-document flat lines for selection/navigation.
        false, // precise shaping/highlighting stays viewport-driven for responsiveness.
    );
    draw_empty_heading_placeholder(engine, &mut dl, bounds, &style, shaper);
    preedit::draw_standalone_preedit(engine, &mut dl, bounds, theme, shaper);
    draw_editor_cursor(engine, &mut dl, bounds, theme, dpi_scale);
    dl
}

fn draw_editor_cursor(
    engine: &BlockViewLayout,
    commands: &mut DrawList,
    bounds: ui::Rect,
    theme: &Theme,
    dpi_scale: f32,
) {
    // Draw cursor at the WYSIWYG position (only when blink phase is visible)
    if engine.cursor_visible
        && let Some((cx, cy, cw, ch)) =
            engine.visual_cursor_screen_pos().or_else(|| engine.cursor_screen_pos())
    {
        let cursor_x = bounds.x + cx;
        let cursor_y = bounds.y + cy;
        let visual_cw = cw * dpi_scale;
        let cursor_rect =
            ui::core::geom::Rect::new(cursor_x - visual_cw * 0.5, cursor_y, visual_cw, ch);
        commands.fill(cursor_rect, theme.editor.cursor);
    }
}

fn empty_heading_level(blocks: &[crate::builder::BlockNode], cursor: usize) -> Option<u8> {
    for block in blocks {
        if block.block_range.start > cursor || block.block_range.end < cursor {
            continue;
        }
        if let crate::builder::BlockKind::Heading { level } = block.kind
            && block.text_lines.iter().all(String::is_empty)
        {
            return Some(level);
        }
        if let Some(level) = empty_heading_level(&block.children, cursor) {
            return Some(level);
        }
    }
    None
}

fn empty_heading_placeholder_target(
    engine: &BlockViewLayout,
) -> Option<(u8, &crate::layout::FlatLine)> {
    let context = engine.edit_ctx.as_ref()?;
    if context.preedit_text.as_ref().is_some_and(|text| !text.is_empty())
        || engine.byte_selection_range().is_some()
    {
        return None;
    }
    let layout = engine.lazy.as_ref()?;
    let level = empty_heading_level(layout.source.blocks(), context.cursor_byte)?;
    let line = engine.flat_lines().iter().find(|line| {
        line.text.is_empty()
            && line.source_projection.as_ref().is_some_and(|projection| {
                projection
                    .boundaries
                    .first()
                    .is_some_and(|anchor| anchor.byte.get() == context.cursor_byte)
            })
    })?;
    Some((level, line))
}

fn draw_empty_heading_placeholder(
    engine: &BlockViewLayout,
    commands: &mut DrawList,
    bounds: ui::Rect,
    style: &MarkdownStyle,
    shaper: &mut shaping::Shaper,
) {
    const HEADING_PLACEHOLDERS: [&str; 6] =
        ["一级标题", "二级标题", "三级标题", "四级标题", "五级标题", "六级标题"];
    const PLACEHOLDER_FADE_RATIO: f32 = 0.55;
    let Some((level, line)) = empty_heading_placeholder_target(engine) else { return };
    commands.clip(bounds, |commands| {
        commands.text_shaped_with_font(
            bounds.x + line.rect.x,
            bounds.y + line.rect.y - engine.scroll_y + line.font_size,
            line.font_size,
            crate::style::blend_toward_bg(
                style.heading_color,
                style.background_color,
                PLACEHOLDER_FADE_RATIO,
            ),
            HEADING_PLACEHOLDERS[usize::from(level - 1)],
            style.body_font_family.first().cloned(),
            shaping::Weight::SEMIBOLD,
            shaping::Style::Normal,
            false,
            shaper,
        );
    });
}
