//! 产品矩形内的共享编辑器绘制。

use render::GlyphVertex;
use ui::plugin::PluginMessage;

use super::{EditorFrame, EditorRuntime, RenderError, RenderResources};
use crate::tab_session::TabSession;

const PLUGIN_CONTENT_HORIZONTAL_PADDING_LOGICAL: f32 = 24.0;
/// 顶部内边距刻意收窄：工具条本身已提供视觉间隔，过大留白会显得编辑区与工具条脱节。
const PLUGIN_CONTENT_TOP_PADDING_LOGICAL: f32 = 8.0;
const PLUGIN_CONTENT_BOTTOM_PADDING_LOGICAL: f32 = 24.0;

fn measure_preedit_advance_px(
    shaper: &mut shaping::Shaper,
    preedit_text: &str,
    font_size: f32,
) -> f32 {
    if preedit_text.is_empty() {
        return 0.0;
    }

    let previous_font_size = shaper.font_size();
    shaper.set_font_size(font_size);
    let advance = shaper
        .shape(preedit_text)
        .map(|shaped| shaped.clusters.iter().map(|cluster| cluster.advance.max(1.0)).sum())
        .unwrap_or(0.0);
    shaper.set_font_size(previous_font_size);
    advance
}

fn plain_text_preedit_origin(
    preedit_text: &str,
    cursor_visual_line: Option<usize>,
    cursor_x_px: f32,
    editor_top_px: f32,
    line_height_px: f32,
    sub_line_offset_px: f32,
) -> Option<(f32, f32)> {
    if preedit_text.is_empty() {
        return None;
    }

    let cursor_visual_line = cursor_visual_line?;
    let cursor_y_px =
        editor_top_px + cursor_visual_line as f32 * line_height_px + sub_line_offset_px;
    Some((cursor_x_px, cursor_y_px))
}

fn editor_viewport_dimensions(editor_height_px: f32, line_height_px: f32) -> (usize, f64) {
    let viewport_height = (editor_height_px / line_height_px).max(1.0);
    (viewport_height.floor() as usize, viewport_height as f64)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorSurfacePaint {
    #[default]
    Empty,
    Document {
        vertex_count: usize,
    },
}

impl EditorRuntime {
    pub fn paint_active_editor(
        &mut self,
        frame: &mut EditorFrame,
        resources: &mut RenderResources,
        editor_rect: ui::Rect,
    ) -> Result<EditorSurfacePaint, RenderError> {
        let Some(tab_id) = self.active_tab_id() else {
            return Ok(EditorSurfacePaint::Empty);
        };
        frame.paint_editor(editor_rect)?;
        if editor_rect.w == 0.0 || editor_rect.h == 0.0 {
            return Ok(EditorSurfacePaint::Document { vertex_count: 0 });
        }

        let settings = self.settings.clone();
        let theme = self.theme.clone();
        let dpi = self.render_session.scale_factor() as f32;
        let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
        let preedit = self.preedit();
        let screen = editor_screen(resources, editor_rect);
        let handles_own_rendering =
            self.tab_session(tab_id).is_some_and(|tab| tab.handles_own_rendering());
        let mut vertices = if handles_own_rendering {
            self.plain_text_preedit_advance_px = 0.0;
            paint_plugin_editor(
                self,
                tab_id,
                resources,
                editor_rect,
                screen,
                &theme,
                &metrics,
                settings.toc_max_depth,
                preedit,
            )
        } else {
            paint_text_editor(
                self,
                tab_id,
                resources,
                editor_rect,
                screen,
                &theme,
                &settings,
                &metrics,
            )
        };
        let vertex_count = vertices.len();
        frame.paint_editor_vertices(editor_rect, vertices.drain(..))?;
        Ok(EditorSurfacePaint::Document { vertex_count })
    }
}

fn paint_plugin_editor(
    runtime: &mut EditorRuntime,
    tab_id: appkit_core::workspace::types::TabId,
    resources: &mut RenderResources,
    editor_rect: ui::Rect,
    screen: ui::Screen,
    theme: &ui::Theme,
    metrics: &ui::settings::UiMetrics,
    toc_max_depth: u8,
    preedit: (String, Option<(usize, usize)>),
) -> Vec<GlyphVertex> {
    let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) else {
        return Vec::new();
    };
    let cursor_paint_enabled = runtime.active_cursor_paint_enabled();
    let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
        return Vec::new();
    };

    let preedit_active = !preedit.0.is_empty();
    synchronize_plugin_document(&mut tab, preedit);
    if preedit_active {
        tab.invalidate_cursor_visibility();
    }
    tab.send_message(PluginMessage::SetRenderSettings {
        font_size: metrics.font_size / metrics.dpi,
        line_height: metrics.line_height / metrics.dpi,
        toc_max_depth,
    });
    let cursor_visible = cursor_paint_enabled
        && (tab.cursor_blink_instant().elapsed().as_millis() / 500).is_multiple_of(2);
    tab.send_message(PluginMessage::SetCursorVisible(cursor_visible));

    let bounds = plugin_bounds(editor_rect, metrics.dpi, tab.is_canvas());
    let mut draw_list = if tab.is_canvas() {
        tab.prepare_canvas_plugin(theme, &mut text.shaper, metrics.dpi)
            .and_then(|canvas_metrics| {
                tab.prepare_canvas_viewport(canvas_metrics, bounds, metrics.dpi)
            })
            .map_or_else(ui::DrawList::new, |snapshot| {
                tab.render_canvas_plugin(&snapshot, theme, &mut text.shaper, metrics.dpi)
            })
    } else {
        render_plugin_with_visible_cursor(&mut tab, bounds, theme, &mut text.shaper, metrics.dpi)
    };
    let tab_view = TabSession::new(tab.id, tab.document, tab.runtime);
    if tab_view.has_selection() {
        draw_list.cmds.extend(tab_view.selection_highlights(theme.editor.selection).cmds);
    }
    if tab_view.search_state().is_active() && !tab_view.search_state().query.is_empty() {
        let search = tab_view.search_state();
        draw_list.cmds.extend(
            tab_view
                .search_highlights(
                    search.query.clone(),
                    search.options.match_case,
                    search.options.use_regex,
                    search.active_match_idx,
                    theme.palette.highlight,
                    theme.palette.inactive_highlight,
                )
                .cmds,
        );
    }
    crate::paint_backend::drain(draw_list, screen, Some(text), Some(gpu))
}

fn render_plugin_with_visible_cursor(
    tab: &mut crate::tab_session::TabSessionMut<'_>,
    bounds: ui::Rect,
    theme: &ui::Theme,
    shaper: &mut shaping::Shaper,
    dpi: f32,
) -> ui::DrawList {
    let mut draw_list = tab.render_plugin(bounds, theme, shaper, dpi);
    let cursor_offset = tab.document.cursor().offset;
    if cursor_offset == tab.last_cursor_offset() {
        return draw_list;
    }
    tab.set_last_cursor_offset(cursor_offset);

    let cursor_byte = cursor_offset.to_usize();
    let cursor_rect = TabSession::new(tab.id, &*tab.document, &*tab.runtime)
        .query_cursor_screen_rect(cursor_byte);
    let scroll_delta = cursor_rect.and_then(|(_, cursor_y, _, cursor_height)| {
        plugin_cursor_visibility_scroll_delta(cursor_y, cursor_height, bounds.h)
    });
    let Some(scroll_delta) = scroll_delta else {
        return draw_list;
    };
    if !tab.send_message(PluginMessage::Scroll { delta: scroll_delta, viewport_h: bounds.h }) {
        return draw_list;
    }

    draw_list = tab.render_plugin(bounds, theme, shaper, dpi);
    draw_list
}

fn plugin_cursor_visibility_scroll_delta(
    cursor_y: f32,
    cursor_height: f32,
    viewport_height: f32,
) -> Option<f32> {
    if !cursor_y.is_finite()
        || !cursor_height.is_finite()
        || !viewport_height.is_finite()
        || cursor_height < 0.0
        || viewport_height <= 0.0
    {
        return None;
    }
    if cursor_y < 0.0 {
        return Some(cursor_y);
    }

    let overflow_below = cursor_y + cursor_height - viewport_height;
    (overflow_below > 0.0).then_some(overflow_below)
}

fn synchronize_plugin_document(
    tab: &mut crate::tab_session::TabSessionMut<'_>,
    preedit: (String, Option<(usize, usize)>),
) {
    let generation = tab.document.tb().gap_buffer().generation();
    if tab.needs_source_update(generation) {
        tab.send_message(PluginMessage::UpdateSource {
            text: tab.document.full_text(),
            generation,
        });
    }
    if let Some((start, end)) = tab.document.selection_range().filter(|(start, end)| start < end) {
        tab.send_message(PluginMessage::SetSelAnchorByte(Some(start)));
        tab.send_message(PluginMessage::SetSelCursorByte(Some(end)));
    } else {
        tab.send_message(PluginMessage::SetSelAnchorByte(None));
        tab.send_message(PluginMessage::SetSelCursorByte(None));
    }
    tab.send_message(PluginMessage::SetCursorByte(tab.document.cursor_offset().to_usize()));
    tab.send_message(PluginMessage::SetPreedit { text: preedit.0, cursor: preedit.1 });
}

fn paint_text_editor(
    runtime: &mut EditorRuntime,
    tab_id: appkit_core::workspace::types::TabId,
    resources: &mut RenderResources,
    editor_rect: ui::Rect,
    screen: ui::Screen,
    theme: &ui::Theme,
    settings: &ui::settings::Settings,
    metrics: &ui::settings::UiMetrics,
) -> Vec<GlyphVertex> {
    runtime.plain_text_preedit_advance_px = 0.0;
    let (preedit_text, _) = runtime.preedit();
    let (visible_rows, viewport_height) =
        editor_viewport_dimensions(editor_rect.h, metrics.line_height);
    {
        let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
            return Vec::new();
        };
        tab.resize_and_refresh_presentation(visible_rows, viewport_height, metrics.line_height);
    }

    let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) else {
        return Vec::new();
    };
    let preedit_advance_px =
        measure_preedit_advance_px(&mut text.shaper, &preedit_text, metrics.font_size);
    runtime.plain_text_preedit_advance_px = preedit_advance_px;
    let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
        return Vec::new();
    };
    let line_count = tab.document.line_count();
    let gutter_width = settings.gutter_width(line_count) * metrics.dpi;
    let left_margin = editor_rect.x + metrics.content_left_margin.max(gutter_width);

    let context = ui::gutter::RenderContext {
        theme,
        screen_w: screen.w,
        screen_h: screen.h,
        left_margin,
        tab_bar_height: editor_rect.y,
        is_active_tab: true,
        gutter_width,
        preedit_advance_px,
        preedit_cursor_col: tab.document.cursor_column(),
    };
    let mut advance_cache = tab.take_advance_cache();
    let mut presentation = tab.take_presentation();
    let mut tree_dirty = false;
    let mut vertices = crate::render_pipeline::shape_visible_lines(
        metrics,
        settings.min_punctuation_width_ratio,
        &context,
        tab.document,
        &mut presentation,
        text,
        gpu,
        &mut advance_cache,
        &mut resources.frame_cache.cluster_pool,
        &mut resources.frame_cache.first_line,
        &mut resources.frame_cache.last_line,
        &mut tree_dirty,
        settings.word_wrap,
    );
    tab.restore_presentation(presentation);
    tab.restore_advance_cache(advance_cache);
    if tree_dirty {
        tab.derive_scroll_top(metrics.line_height);
    }
    let tab_view = TabSession::new(tab.id, tab.document, tab.runtime);
    vertices.extend(text_selection_vertices(
        &tab_view,
        metrics,
        screen,
        left_margin,
        theme,
        editor_rect.y,
    ));
    let preedit_origin = plain_text_preedit_origin(
        &preedit_text,
        tab_view.cursor_visual_line(),
        tab_view.cursor_pixel_x(),
        editor_rect.y,
        metrics.line_height,
        tab_view.sub_line_pixel_offset(metrics.line_height),
    );
    if let Some((preedit_x, preedit_y)) = preedit_origin {
        vertices.extend(crate::render_pipeline::preedit_text_vertices(
            metrics,
            &preedit_text,
            preedit_x,
            preedit_y,
            text,
            gpu,
            screen.w,
            screen.h,
            theme.editor.foreground,
        ));
    }
    vertices.extend(ui::decorations::cursor_vertices(
        theme,
        tab_view.cursor_visual_line(),
        editor_rect.y,
        tab_view.cursor_pixel_x() + preedit_advance_px,
        tab_view.cursor_blink_instant(),
        metrics,
        screen.w,
        screen.h,
        tab_view.sub_line_pixel_offset(metrics.line_height),
        None,
    ));
    let cursor_scroll_changed = tab.ensure_cursor_visual_row_visible(metrics.line_height);
    if cursor_scroll_changed {
        runtime.request_redraw();
    }
    vertices
}

fn text_selection_vertices(
    tab: &TabSession<'_>,
    metrics: &ui::settings::UiMetrics,
    screen: ui::Screen,
    left_margin: f32,
    theme: &ui::Theme,
    editor_top: f32,
) -> Vec<GlyphVertex> {
    let max_doc_line = tab.advance_cache().iter().map(|entry| entry.doc_line).max().unwrap_or(0);
    let mut line_offsets = vec![0usize; max_doc_line + 1];
    for entry in tab.advance_cache() {
        line_offsets[entry.doc_line] = tab.document.line_byte_offset(entry.doc_line).unwrap_or(0);
    }
    ui::decorations::selection_vertices(
        tab.document.selection_range(),
        tab.advance_cache(),
        metrics,
        screen.w,
        screen.h,
        left_margin,
        theme,
        editor_top,
        tab.sub_line_pixel_offset(metrics.line_height),
        &line_offsets,
    )
}

fn editor_screen(resources: &RenderResources, editor_rect: ui::Rect) -> ui::Screen {
    resources.gpu.as_ref().map_or_else(
        || ui::Screen::new(editor_rect.right().max(1.0), editor_rect.bottom().max(1.0)),
        |gpu| ui::Screen::new(gpu.ctx.config.width as f32, gpu.ctx.config.height as f32),
    )
}

pub(super) fn plugin_bounds(editor_rect: ui::Rect, dpi: f32, is_canvas: bool) -> ui::Rect {
    if is_canvas {
        return editor_rect;
    }
    let horizontal_padding = PLUGIN_CONTENT_HORIZONTAL_PADDING_LOGICAL * dpi;
    let top_padding = PLUGIN_CONTENT_TOP_PADDING_LOGICAL * dpi;
    let bottom_padding = PLUGIN_CONTENT_BOTTOM_PADDING_LOGICAL * dpi;
    ui::Rect::new(
        editor_rect.x + horizontal_padding,
        editor_rect.y + top_padding,
        (editor_rect.w - horizontal_padding * 2.0).max(1.0),
        (editor_rect.h - top_padding - bottom_padding).max(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        editor_viewport_dimensions, plain_text_preedit_origin,
        plugin_cursor_visibility_scroll_delta,
    };

    #[test]
    fn partial_bottom_row_is_not_counted_as_fully_visible() {
        const LINE_HEIGHT_PX: f32 = 20.0;
        const EDITOR_HEIGHT_PX: f32 = LINE_HEIGHT_PX * 10.5;

        let (visible_rows, viewport_height) =
            editor_viewport_dimensions(EDITOR_HEIGHT_PX, LINE_HEIGHT_PX);

        assert_eq!(visible_rows, 10);
        assert_eq!(viewport_height, 10.5);
    }

    #[test]
    fn plugin_cursor_below_the_short_viewport_requests_minimal_scroll() {
        const VIEWPORT_HEIGHT_PX: f32 = 90.0;
        const CURSOR_TOP_PX: f32 = 82.0;
        const CURSOR_HEIGHT_PX: f32 = 30.0;

        assert_eq!(
            plugin_cursor_visibility_scroll_delta(
                CURSOR_TOP_PX,
                CURSOR_HEIGHT_PX,
                VIEWPORT_HEIGHT_PX,
            ),
            Some(22.0),
        );
    }

    #[test]
    fn visible_plugin_cursor_does_not_request_scroll() {
        assert_eq!(plugin_cursor_visibility_scroll_delta(24.0, 30.0, 90.0), None);
    }

    #[test]
    fn plugin_cursor_above_the_viewport_requests_minimal_scroll() {
        assert_eq!(plugin_cursor_visibility_scroll_delta(-12.0, 30.0, 90.0), Some(-12.0));
    }

    #[test]
    fn non_empty_plain_text_preedit_starts_at_the_painted_document_caret() {
        const EDITOR_TOP_PX: f32 = 120.0;
        const CURSOR_X_PX: f32 = 248.0;
        const LINE_HEIGHT_PX: f32 = 24.0;
        const SUB_LINE_OFFSET_PX: f32 = -6.0;

        let origin = plain_text_preedit_origin(
            "拼音",
            Some(3),
            CURSOR_X_PX,
            EDITOR_TOP_PX,
            LINE_HEIGHT_PX,
            SUB_LINE_OFFSET_PX,
        )
        .expect("active preedit on a visible caret must produce a paint origin");

        assert_eq!(origin, (CURSOR_X_PX, 186.0));
        assert_eq!(
            plain_text_preedit_origin(
                "",
                Some(3),
                CURSOR_X_PX,
                EDITOR_TOP_PX,
                LINE_HEIGHT_PX,
                SUB_LINE_OFFSET_PX,
            ),
            None
        );
    }
}
