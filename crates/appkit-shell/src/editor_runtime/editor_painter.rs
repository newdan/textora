//! 产品矩形内的共享编辑器绘制。

use render::GlyphVertex;
use ui::plugin::PluginMessage;

use super::{EditorFrame, EditorRuntime, RenderError, RenderResources};
use crate::tab_session::TabSession;

const PLUGIN_CONTENT_PADDING_LOGICAL: f32 = 24.0;

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
    let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
        return Vec::new();
    };

    synchronize_plugin_document(&mut tab, preedit);
    tab.send_message(PluginMessage::SetRenderSettings {
        font_size: metrics.font_size / metrics.dpi,
        line_height: metrics.line_height / metrics.dpi,
        toc_max_depth,
    });
    let cursor_visible = (tab.cursor_blink_instant().elapsed().as_millis() / 500).is_multiple_of(2);
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
        tab.render_plugin(bounds, theme, &mut text.shaper, metrics.dpi)
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
    let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) else {
        return Vec::new();
    };
    let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
        return Vec::new();
    };
    let line_count = tab.document.line_count();
    let gutter_width = settings.gutter_width(line_count) * metrics.dpi;
    let left_margin = editor_rect.x + metrics.content_left_margin.max(gutter_width);
    let visible_rows = (editor_rect.h / metrics.line_height).ceil().max(1.0) as usize;
    tab.resize_presentation(visible_rows, editor_rect.h as f64);

    let context = ui::gutter::RenderContext {
        theme,
        screen_w: screen.w,
        screen_h: screen.h,
        left_margin,
        tab_bar_height: editor_rect.y,
        is_active_tab: true,
        gutter_width,
        preedit_advance_px: 0.0,
        preedit_cursor_col: 0,
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
    vertices.extend(ui::decorations::cursor_vertices(
        theme,
        tab_view.cursor_visual_line(),
        editor_rect.y,
        tab_view.cursor_pixel_x(),
        tab_view.cursor_blink_instant(),
        metrics,
        screen.w,
        screen.h,
        tab_view.sub_line_pixel_offset(metrics.line_height),
        None,
    ));
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

fn plugin_bounds(editor_rect: ui::Rect, dpi: f32, is_canvas: bool) -> ui::Rect {
    if is_canvas {
        return editor_rect;
    }
    let padding = PLUGIN_CONTENT_PADDING_LOGICAL * dpi;
    ui::Rect::new(
        editor_rect.x + padding,
        editor_rect.y + padding,
        (editor_rect.w - padding * 2.0).max(1.0),
        (editor_rect.h - padding * 2.0).max(1.0),
    )
}
