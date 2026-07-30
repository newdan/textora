//! Scroll handling: mouse wheel, cursor movement, page up/down, visual-line selection.
//! Methods on `impl App`, extracted from app.rs.

use crate::app::App;
use crate::app_effect::AppEffect;
use crate::canvas_viewport::CanvasViewportAction;
use crate::cursor_motion::CursorContext;
use ui::canvas::CanvasPoint;
use winit::event::MouseScrollDelta;

const CANVAS_LINE_SCROLL_DISTANCE_PX: f32 = 40.0;
const MIN_CANVAS_WHEEL_ZOOM_FACTOR: f32 = 0.01;

fn canvas_pan_delta(delta: MouseScrollDelta, shift_pressed: bool) -> CanvasPoint {
    let pan_delta = match delta {
        MouseScrollDelta::LineDelta(x, y) => CanvasPoint::new(
            -x * CANVAS_LINE_SCROLL_DISTANCE_PX,
            -y * CANVAS_LINE_SCROLL_DISTANCE_PX,
        ),
        MouseScrollDelta::PixelDelta(position) => {
            CanvasPoint::new(-(position.x as f32), -(position.y as f32))
        }
    };

    if shift_pressed && pan_delta.x == 0.0 { CanvasPoint::new(pan_delta.y, 0.0) } else { pan_delta }
}

fn canvas_wheel_zoom_factor(delta: MouseScrollDelta) -> Option<f32> {
    let delta = match delta {
        MouseScrollDelta::LineDelta(_, y) => y as f64,
        MouseScrollDelta::PixelDelta(position) => position.y,
    };
    if !delta.is_finite() {
        return None;
    }

    let factor = 1.0 + delta as f32;
    if !factor.is_finite() {
        return None;
    }

    Some(factor.max(MIN_CANVAS_WHEEL_ZOOM_FACTOR))
}

impl App {
    pub(crate) fn apply_canvas_viewport_action(
        &mut self,
        action: CanvasViewportAction,
    ) -> AppEffect {
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };
        if !tab.is_canvas() || !tab.has_canvas_viewport_snapshot() {
            return AppEffect::NONE;
        }

        tab.apply_canvas_viewport_action(action);
        AppEffect::REDRAW
    }

    /// Move cursor to an adjacent visual line, preserving horizontal position (sticky_x).
    /// Used for up/down arrow keys when word wrap creates multiple visual lines per doc line.
    /// Only updates DocumentView cursor_offset; shape_visible_lines handles visual position.
    pub(crate) fn move_cursor_visual(&mut self, delta: isize) -> AppEffect {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let line_height = metrics.line_height;
        let Some(tab_id) = self.active_tab_id() else {
            return AppEffect::NONE;
        };
        let runtime_frame_cache = self.editor_runtime.frame_cache_snapshot();
        let first_line = runtime_frame_cache.first_line;
        let last_line = runtime_frame_cache.last_line;
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return AppEffect::NONE;
        };
        let display_map = tab.display_map_clone();
        let vl_before = tab.cursor_visual_line();
        let doc_line_before = tab.document.cursor_line();
        let advance_cache = tab.take_advance_cache();
        let ctx = CursorContext {
            display_map: &display_map,
            cursor_visual_line: tab.cursor_visual_line(),
            advance_cache: &advance_cache,
            first_line: &first_line,
            last_line: &last_line,
            visible_rows: tab.visible_rows(),
            first_visible_row: tab.display().viewport.first_visible_row(),
            scroll_top: tab.scroll_top(),
            sticky_x: tab.cursor_render_state().sticky_x,
            dpi_scale: dpi,
        };
        tab.document.cursor_mut().selection_anchor = None;
        tab.move_cursor_visual(delta, ctx);
        tab.restore_advance_cache(advance_cache);
        tab.ensure_cursor_visible(line_height);
        // Long wrapped line: ensure_cursor_visible only checks doc-line
        // boundaries, so cursor may move off-screen within the same doc
        // line. Force-scroll when cursor was at viewport bottom.
        if delta > 0
            && doc_line_before == tab.document.cursor_line()
            && vl_before.is_some_and(|vl| vl + 1 >= tab.visible_rows())
        {
            tab.display_mut().viewport.scroll_pixels(line_height, &display_map, line_height);
            tab.refresh_scroll_metrics(line_height);
        }
        AppEffect::REDRAW
    }

    pub(crate) fn page_up(&mut self) -> AppEffect {
        let line_height = self.ui_metrics().line_height;
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };
        tab.page_up(line_height);
        AppEffect::REDRAW
    }

    pub(crate) fn page_down(&mut self) -> AppEffect {
        let line_height = self.ui_metrics().line_height;
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };
        tab.page_down(line_height);
        AppEffect::REDRAW
    }

    /// Scroll the active non-editing plugin by a navigation command.
    /// Used for PageUp/PageDown/Arrow keys/Home/End in preview/novel mode.
    pub(crate) fn plugin_scroll_by_command(
        &mut self,
        cmd: &crate::input::EditCommand,
    ) -> AppEffect {
        use crate::input::EditCommand;

        let metrics = self.ui_metrics();
        let line_height = metrics.line_height;
        let viewport_h = self.plugin_viewport_h();
        let handles_own_rendering = self.active_handles_own_rendering();

        // Dispatch scroll command to plugin and collect resulting scroll state.
        // The inner scope ensures the mutable borrow of workspace ends before
        // we touch self.ui_shell for the scrollbar update.
        let scroll_result: Option<(f32, f32)> = {
            let Some(mut tab) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            // Only plugins that render their own viewport get plugin scroll.
            // Standard editor (allows_editing=true, handles_own_rendering=false)
            // falls through to normal DocumentView scrolling below.
            if !handles_own_rendering {
                return AppEffect::NONE;
            }

            let scroll_msg = match cmd {
                EditCommand::PageUp => {
                    Some(ui::plugin::PluginMessage::Scroll { delta: -viewport_h, viewport_h })
                }
                EditCommand::PageDown => {
                    Some(ui::plugin::PluginMessage::Scroll { delta: viewport_h, viewport_h })
                }
                EditCommand::MoveUp => Some(ui::plugin::PluginMessage::Scroll {
                    delta: -line_height * 3.0,
                    viewport_h,
                }),
                EditCommand::MoveDown => {
                    Some(ui::plugin::PluginMessage::Scroll { delta: line_height * 3.0, viewport_h })
                }
                EditCommand::MoveToDocStart => {
                    tab.send_message(ui::plugin::PluginMessage::SetScrollY(0.0));
                    None
                }
                EditCommand::MoveToDocEnd => {
                    tab.send_message(ui::plugin::PluginMessage::SetScrollRatio(1.0));
                    None
                }
                _ => return AppEffect::NONE,
            };

            let consumed = if let Some(msg) = scroll_msg { tab.send_message(msg) } else { true };

            if consumed {
                let content_h = tab.content_height();
                let scroll_y = tab.scroll_y();
                Some((content_h, scroll_y))
            } else {
                None
            }
        };

        if let Some((content_h, scroll_y)) = scroll_result {
            self.sync_plugin_scrollbar(content_h, scroll_y, line_height, viewport_h);
            return AppEffect::REDRAW;
        }
        AppEffect::NONE
    }

    /// Scroll the active non-editing plugin by pixel delta.
    /// Shared by mouse wheel and keyboard scroll paths.
    pub(crate) fn plugin_scroll_by_pixels(&mut self, delta: f32) -> AppEffect {
        let metrics = self.ui_metrics();
        let line_height = metrics.line_height;
        let viewport_h = self.plugin_viewport_h();
        let handles_own_rendering = self.active_handles_own_rendering();

        let scroll_result: Option<(f32, f32)> = {
            let Some(mut tab) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            // Only plugins that render their own viewport get plugin scroll.
            // Standard editor (allows_editing=true, handles_own_rendering=false)
            // falls through to normal DocumentView scrolling below.
            if !handles_own_rendering {
                return AppEffect::NONE;
            }
            let consumed =
                tab.send_message(ui::plugin::PluginMessage::Scroll { delta, viewport_h });
            if consumed {
                let content_h = tab.content_height();
                let scroll_y = tab.scroll_y();
                Some((content_h, scroll_y))
            } else {
                None
            }
        };

        if let Some((content_h, scroll_y)) = scroll_result {
            self.sync_plugin_scrollbar(content_h, scroll_y, line_height, viewport_h);
            return AppEffect::REDRAW;
        }
        AppEffect::NONE
    }

    /// Effective viewport height for plugin content (editor area minus top padding).
    pub(crate) fn plugin_viewport_h(&self) -> f32 {
        let dpi = self.ui_metrics().dpi;
        let preview_top_pad = 16.0 * dpi;
        (self.ui_shell.editor_rect().h - preview_top_pad).max(100.0)
    }

    /// Synchronize scrollbar with plugin scroll position.
    pub(crate) fn sync_plugin_scrollbar(
        &mut self,
        content_h: f32,
        scroll_y: f32,
        line_height: f32,
        viewport_h: f32,
    ) {
        let total_rows = (content_h / line_height).ceil() as usize;
        let scroll_rows = scroll_y / line_height;
        self.ui_shell.set_scrollbar_input(
            (viewport_h / line_height) as f64,
            total_rows,
            scroll_rows as f64,
        );
    }

    pub(crate) fn extend_selection_visual(&mut self, delta: isize) -> AppEffect {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let line_height = metrics.line_height;
        let Some(tab_id) = self.active_tab_id() else {
            return AppEffect::NONE;
        };
        let runtime_frame_cache = self.editor_runtime.frame_cache_snapshot();
        let first_line = runtime_frame_cache.first_line;
        let last_line = runtime_frame_cache.last_line;
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return AppEffect::NONE;
        };
        let display_map = tab.display_map_clone();
        let vl_before = tab.cursor_visual_line();
        let doc_line_before = tab.document.cursor_line();
        let advance_cache = tab.take_advance_cache();
        let ctx = CursorContext {
            display_map: &display_map,
            cursor_visual_line: tab.cursor_visual_line(),
            advance_cache: &advance_cache,
            first_line: &first_line,
            last_line: &last_line,
            visible_rows: tab.visible_rows(),
            first_visible_row: tab.display().viewport.first_visible_row(),
            scroll_top: tab.scroll_top(),
            sticky_x: tab.cursor_render_state().sticky_x,
            dpi_scale: dpi,
        };
        if tab.document.cursor().selection_anchor.is_none() {
            tab.document.cursor_mut().selection_anchor =
                Some(tab.document.cursor().offset.to_usize());
        }
        tab.move_cursor_visual(delta, ctx);
        tab.ensure_cursor_visible(line_height);
        tab.restore_advance_cache(advance_cache);
        // Long wrapped line: ensure_cursor_visible only checks doc-line
        // boundaries, so cursor may move off-screen within the same doc
        // line. Force-scroll when cursor was at viewport bottom.
        if delta > 0
            && doc_line_before == tab.document.cursor_line()
            && vl_before.is_some_and(|vl| vl + 1 >= tab.visible_rows())
        {
            tab.display_mut().viewport.scroll_pixels(line_height, &display_map, line_height);
            tab.refresh_scroll_metrics(line_height);
        }
        AppEffect::REDRAW
    }

    /// Handle mouse wheel scroll events.
    /// PixelDelta scrolls by exact visual line fractions for smooth sub-line scrolling.
    pub(crate) fn handle_scroll(&mut self, delta: MouseScrollDelta) -> AppEffect {
        // Extract metrics before mutable borrows of workspace.
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let line_height = metrics.line_height;
        let view_mode = self.settings.view_mode;
        let toc_width = metrics.toc_width;

        // If the mouse is over the tab bar, scroll tabs horizontally via navigator
        let tbh = self.current_tab_bar_height();
        if tbh > 0.0 && (self.mouse.pos.1 as f32) < tbh {
            let dx: f32 = match delta {
                MouseScrollDelta::LineDelta(x, y) => {
                    if x != 0.0 {
                        x * 50.0
                    } else {
                        y * -50.0
                    }
                }
                MouseScrollDelta::PixelDelta(pos) => {
                    let px = pos.x as f32;
                    if px != 0.0 { px } else { -(pos.y as f32) }
                }
            };
            self.ui_shell.tab_bar_scroll_by(dx);
            return AppEffect::REDRAW;
        }
        // Sidebar: scroll sidebar file list when mouse is over sidebar area
        if matches!(view_mode, ui::view_mode::ViewMode::Sidebar)
            && self.editor_runtime.surface_size().is_some()
        {
            let sidebar_w = self.ui_shell.sidebar_current_width();
            if sidebar_w > 0.0 && (self.mouse.pos.0 as f32) < sidebar_w {
                let dy: f32 = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * -40.0,
                    MouseScrollDelta::PixelDelta(pos) => -(pos.y as f32),
                };
                let total = self.editor_tab_count();
                self.ui_shell.sidebar_on_scroll(dy, total);
                return AppEffect::REDRAW;
            }
        }

        // TOC panel: scroll TOC when mouse is over the TOC area
        {
            let toc_vis = self.active_toc_visible();
            if toc_vis {
                let sidebar_w = if matches!(view_mode, ui::view_mode::ViewMode::Sidebar) {
                    self.ui_shell.sidebar_current_width()
                } else {
                    0.0
                };
                let toc_w = toc_width; // toc_width is already physical pixels
                let mx = self.mouse.pos.0 as f32;
                if mx >= sidebar_w && mx < sidebar_w + toc_w {
                    let dy: f32 = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y * -40.0,
                        MouseScrollDelta::PixelDelta(pos) => -(pos.y as f32),
                    };
                    let content_h = self.ui_shell.toc_content_height(dpi);
                    let viewport_h = self.ui_shell.editor_rect().h;
                    self.ui_shell.toc_on_scroll(dy, viewport_h, content_h, dpi);
                    return AppEffect::REDRAW;
                }
            }
        }

        // Canvas: wheel input controls the two-dimensional viewport after chrome routing and
        // before generic plugin preview scrolling.
        if self.active_is_canvas() {
            let input_modifiers = self.editor_runtime.input_modifiers();
            if input_modifiers.super_key() || input_modifiers.control_key() {
                let screen_anchor =
                    CanvasPoint::new(self.mouse.pos.0 as f32, self.mouse.pos.1 as f32);
                let Some(factor) = canvas_wheel_zoom_factor(delta) else {
                    return AppEffect::NONE;
                };
                return self.apply_canvas_viewport_action(CanvasViewportAction::ZoomBy {
                    factor,
                    screen_anchor,
                });
            }

            return self.apply_canvas_viewport_action(CanvasViewportAction::PanBy(
                canvas_pan_delta(delta, input_modifiers.shift_key()),
            ));
        }

        // Non-editing plugin: scroll preview content
        {
            let dy: f32 = match delta {
                MouseScrollDelta::LineDelta(_, y) => y * -60.0,
                MouseScrollDelta::PixelDelta(pos) => -(pos.y as f32),
            };
            let effect = self.plugin_scroll_by_pixels(dy);
            if effect != AppEffect::NONE {
                return effect;
            }
        }

        match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                let pixels = -y * 3.0 * line_height;
                tab.scroll_viewport_by_pixels(pixels, line_height);
            }
            MouseScrollDelta::PixelDelta(pos) => {
                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                tab.scroll_viewport_by_pixels(-pos.y as f32, line_height);
            }
        }
        self.last_scroll_time = std::time::Instant::now();
        AppEffect::REDRAW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;
    use crate::snap_tree::DisplayLineEntry;

    use winit::dpi::PhysicalPosition;
    use winit::keyboard::ModifiersState;

    #[test]
    fn pixel_scroll_uses_instance_line_height() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app.settings.line_height = 36.0;
        app.update_scale_factor(2.0);
        app.mouse.pos = (500.0, 400.0);

        let dv = DocumentView::new((0..100).map(|i| format!("line {i}")).collect(), 10, 10.0);
        app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
        app.active_tab_session_mut()
            .unwrap()
            .display_mut()
            .display_map
            .set_entries((0..100).map(|i| DisplayLineEntry::placeholder(i * 8, 8, 0, 1)).collect());

        // line_height=36.0 logical * dpi 2.0 = 72.0 physical px per line.
        // Scroll -72.0 physical px = -1.0 visual lines.
        app.needs_redraw = false;
        let effect =
            app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -72.0)));

        let scroll_top = app.active_tab_session().unwrap().scroll_top();
        assert!((scroll_top - 1.0).abs() < 0.01, "scroll_top={scroll_top}");
        assert_eq!(effect, AppEffect::REDRAW);
        assert!(!app.needs_redraw);
    }

    #[test]
    fn line_scroll_uses_app_metrics_line_height() {
        let mut app = App::new(None);
        let dv = DocumentView::new((0..100).map(|i| format!("line {i}")).collect(), 80, 10.0);
        app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
        app.switch_workspace_for_test(0);
        app.active_tab_session_mut().unwrap().display_mut().display_map.set_entries(
            (0..100)
                .map(|i| crate::snap_tree::DisplayLineEntry::placeholder(i, 10, 0, 1))
                .collect(),
        );
        app.update_scale_factor(2.0);
        app.mouse.pos = (400.0, 300.0);
        let line_height = app.ui_metrics().line_height;
        app.handle_scroll(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
            0.0,
            -(line_height as f64),
        )));
        let session = app.active_tab_session().unwrap();
        let viewport = &session.display().viewport;
        assert!((viewport.scroll_top - 1.0).abs() < 0.01);
    }

    #[test]
    fn page_down_returns_redraw_without_applying() {
        let mut app = App::new(None);
        let dv =
            DocumentView::new((0..100).map(|index| format!("line {index}")).collect(), 20, 200.0);
        app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
        app.switch_workspace_for_test(0);
        app.needs_redraw = false;

        let effect = app.page_down();

        assert_eq!(effect, AppEffect::REDRAW);
        assert!(!app.needs_redraw);
    }

    // ── Non-editing plugin (reading mode) scroll tests ──

    /// Minimal non-editing plugin that tracks scroll position.
    struct ReadingPlugin {
        scroll_y: f32,
        content_height: f32,
    }

    impl ReadingPlugin {
        fn new(content_height: f32) -> Self {
            Self { scroll_y: 0.0, content_height }
        }
    }

    impl ui::plugin::ViewPlugin for ReadingPlugin {
        fn name(&self) -> &str {
            "reading_test"
        }
        fn allows_editing(&self) -> bool {
            false
        }
        fn shows_cursor(&self) -> bool {
            false
        }
        fn shows_gutter(&self) -> bool {
            false
        }
        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
        fn handle_message(
            &mut self,
            msg: ui::plugin::PluginMessage,
            _doc: &mut dyn core::document::DocViewMut,
        ) -> bool {
            match msg {
                ui::plugin::PluginMessage::Scroll { delta, viewport_h } => {
                    let max = (self.content_height - viewport_h).max(0.0);
                    self.scroll_y = (self.scroll_y + delta).clamp(0.0, max);
                    true
                }
                ui::plugin::PluginMessage::SetScrollY(y) => {
                    self.scroll_y = y.clamp(0.0, self.content_height);
                    true
                }
                ui::plugin::PluginMessage::SetScrollRatio(r) => {
                    self.scroll_y = (r * self.content_height).clamp(0.0, self.content_height);
                    true
                }
                _ => false,
            }
        }
        fn query(
            &self,
            q: ui::plugin::PluginQuery,
            _doc: &dyn core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            match q {
                ui::plugin::PluginQuery::ScrollY => {
                    ui::plugin::PluginResponse::Float(self.scroll_y)
                }
                ui::plugin::PluginQuery::ContentHeight => {
                    ui::plugin::PluginResponse::Float(self.content_height)
                }
                _ => ui::plugin::PluginResponse::None,
            }
        }
    }

    fn make_reading_app(content_height: f32) -> App {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app.update_scale_factor(1.0);
        let dv = DocumentView::new(vec!["hello".to_string()], 80, 10.0);
        app.push_entry_for_test(dv, Box::new(ReadingPlugin::new(content_height)));
        app.switch_workspace_for_test(0);
        app
    }

    #[test]
    fn plugin_scroll_routes_to_the_active_store_runtime() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app.update_scale_factor(1.0);
        let document = DocumentView::new(vec!["hello".to_string()], 80, 10.0);
        let tab_id = app.push_entry_for_test(document, Box::new(EditorPlugin::new()));
        app.switch_workspace_for_test(0);
        app.tab_runtime_mut(tab_id).expect("test tab runtime must exist").plugin =
            Box::new(ReadingPlugin::new(5000.0));

        let effect = app.plugin_scroll_by_pixels(200.0);

        assert_eq!(effect, AppEffect::REDRAW);
        assert_eq!(app.active_tab_session().expect("active session").scroll_y(), 200.0);
    }

    #[test]
    fn plugin_page_down_scrolls_by_viewport() {
        let mut app = make_reading_app(5000.0);
        let effect = app.plugin_scroll_by_command(&crate::input::EditCommand::PageDown);
        assert_eq!(effect, AppEffect::REDRAW);
        let tab = app.active_tab_session().unwrap();
        match tab.query(ui::plugin::PluginQuery::ScrollY) {
            ui::plugin::PluginResponse::Float(y) => {
                assert!(y > 0.0, "scroll_y should advance on PageDown, got {y}");
            }
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn plugin_page_up_scrolls_back() {
        let mut app = make_reading_app(5000.0);
        // Scroll down first.
        app.plugin_scroll_by_command(&crate::input::EditCommand::PageDown);
        let y_after_down = app.active_tab_session().unwrap().scroll_y();
        assert!(y_after_down > 0.0);
        // Scroll back up.
        app.plugin_scroll_by_command(&crate::input::EditCommand::PageUp);
        let y_after_up = app.active_tab_session().unwrap().scroll_y();
        assert!(y_after_up < y_after_down, "PageUp should reduce scroll_y");
    }

    #[test]
    fn plugin_move_down_scrolls_small_amount() {
        let mut app = make_reading_app(5000.0);
        app.plugin_scroll_by_command(&crate::input::EditCommand::MoveDown);
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y > 0.0 && y < 100.0, "MoveDown should scroll a small amount, got {y}");
    }

    #[test]
    fn plugin_move_to_doc_start_resets_scroll() {
        let mut app = make_reading_app(5000.0);
        // Scroll down first.
        app.plugin_scroll_by_command(&crate::input::EditCommand::PageDown);
        app.plugin_scroll_by_command(&crate::input::EditCommand::PageDown);
        // Go to start.
        app.plugin_scroll_by_command(&crate::input::EditCommand::MoveToDocStart);
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y.abs() < 1.0, "MoveToDocStart should reset scroll_y to 0, got {y}");
    }

    #[test]
    fn plugin_move_to_doc_end_scrolls_to_bottom() {
        let mut app = make_reading_app(5000.0);
        app.plugin_scroll_by_command(&crate::input::EditCommand::MoveToDocEnd);
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y > 0.0, "MoveToDocEnd should scroll near bottom, got {y}");
    }

    #[test]
    fn plugin_scroll_by_pixels_advances_scroll() {
        let mut app = make_reading_app(5000.0);
        let effect = app.plugin_scroll_by_pixels(200.0);
        assert_eq!(effect, AppEffect::REDRAW);
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!((y - 200.0).abs() < 1.0, "scroll_y should be ~200, got {y}");
    }

    #[test]
    fn plugin_scroll_clamps_at_content_end() {
        let mut app = make_reading_app(500.0);
        // Try to scroll way past the end.
        app.plugin_scroll_by_pixels(9999.0);
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y <= 500.0, "scroll_y should clamp at content_height, got {y}");
    }

    #[test]
    fn plugin_scroll_noop_for_standard_editor() {
        let mut app = App::new(None);
        app.update_scale_factor(1.0);
        let dv = DocumentView::new(vec!["hello".to_string()], 80, 10.0);
        app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
        app.switch_workspace_for_test(0);
        // Standard editor: allows_editing=true, handles_own_rendering=false.
        // Plugin scroll is a no-op; scrolling falls through to DocumentView.
        let effect = app.plugin_scroll_by_command(&crate::input::EditCommand::PageDown);
        assert_eq!(effect, AppEffect::NONE);
    }

    // ── WYSIWYG plugin (allows_editing=true, handles_own_rendering=true) ──

    /// Minimal WYSIWYG plugin stub: allows_editing=true, handles_own_rendering=true,
    /// handles_own_rendering=true (derived). Tracks scroll position.
    struct WysiwygPlugin {
        scroll_y: f32,
        content_height: f32,
    }

    impl WysiwygPlugin {
        fn new(content_height: f32) -> Self {
            Self { scroll_y: 0.0, content_height }
        }
    }

    impl ui::plugin::ViewPlugin for WysiwygPlugin {
        fn handles_own_rendering(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "wysiwyg_test"
        }
        fn allows_editing(&self) -> bool {
            true
        }
        fn shows_cursor(&self) -> bool {
            true
        }
        fn shows_gutter(&self) -> bool {
            false
        }
        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
        fn handle_message(
            &mut self,
            msg: ui::plugin::PluginMessage,
            _doc: &mut dyn core::document::DocViewMut,
        ) -> bool {
            match msg {
                ui::plugin::PluginMessage::Scroll { delta, viewport_h } => {
                    let max = (self.content_height - viewport_h).max(0.0);
                    self.scroll_y = (self.scroll_y + delta).clamp(0.0, max);
                    true
                }
                ui::plugin::PluginMessage::SetScrollY(y) => {
                    self.scroll_y = y.clamp(0.0, self.content_height);
                    true
                }
                _ => false,
            }
        }
        fn query(
            &self,
            q: ui::plugin::PluginQuery,
            _doc: &dyn core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            match q {
                ui::plugin::PluginQuery::ScrollY => {
                    ui::plugin::PluginResponse::Float(self.scroll_y)
                }
                ui::plugin::PluginQuery::ContentHeight => {
                    ui::plugin::PluginResponse::Float(self.content_height)
                }
                _ => ui::plugin::PluginResponse::None,
            }
        }
    }

    fn make_wysiwyg_app(content_height: f32) -> App {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app.update_scale_factor(1.0);
        let dv = DocumentView::new(vec!["hello **world**".to_string()], 80, 10.0);
        app.push_entry_for_test(dv, Box::new(WysiwygPlugin::new(content_height)));
        app.switch_workspace_for_test(0);
        app
    }

    #[test]
    fn wysiwyg_plugin_receives_scroll_by_pixels() {
        let mut app = make_wysiwyg_app(5000.0);
        let scroll_top_before =
            app.active_tab_session().unwrap().runtime.presentation.display.viewport.scroll_top;
        let effect = app.plugin_scroll_by_pixels(200.0);
        assert_eq!(effect, AppEffect::REDRAW);
        // Plugin ScrollY increased.
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!((y - 200.0).abs() < 1.0, "scroll_y should be ~200, got {y}");
        // DocumentView scroll_top unchanged.
        let scroll_top_after =
            app.active_tab_session().unwrap().runtime.presentation.display.viewport.scroll_top;
        assert!(
            (scroll_top_after - scroll_top_before).abs() < 0.01,
            "DocumentView scroll_top should not change for WYSIWYG scroll"
        );
    }

    #[test]
    fn wysiwyg_plugin_receives_scroll_by_command_page_down() {
        let mut app = make_wysiwyg_app(5000.0);
        let effect = app.plugin_scroll_by_command(&crate::input::EditCommand::PageDown);
        assert_eq!(effect, AppEffect::REDRAW);
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y > 0.0, "scroll_y should advance on PageDown, got {y}");
    }

    #[test]
    fn wysiwyg_scroll_viewport_by_routes_to_plugin() {
        let mut app = make_wysiwyg_app(5000.0);
        let scroll_top_before =
            app.active_tab_session().unwrap().runtime.presentation.display.viewport.scroll_top;
        // ScrollViewportBy is the action produced by scrollbar PageUp/PageDown.
        // For handles_own_rendering() plugins, it should scroll the plugin viewport.
        let effect = app.dispatch_viewport_action(
            crate::dispatch::viewport::ViewportDispatchAction::ScrollViewportBy(1.0),
        );
        assert_eq!(effect, AppEffect::REDRAW);
        // Plugin ScrollY increased.
        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y > 0.0, "plugin ScrollY should increase on ScrollViewportBy, got {y}");
        // DocumentView scroll_top unchanged.
        let scroll_top_after =
            app.active_tab_session().unwrap().runtime.presentation.display.viewport.scroll_top;
        assert!(
            (scroll_top_after - scroll_top_before).abs() < 0.01,
            "DocumentView scroll_top should not change for WYSIWYG ScrollViewportBy"
        );
    }

    #[test]
    fn wysiwyg_handle_scroll_wheel_routes_to_plugin_scroll() {
        let mut app = make_wysiwyg_app(5000.0);
        app.mouse.pos = (400.0, 300.0); // Set mouse away from tab bar / sidebar
        let scroll_top_before =
            app.active_tab_session().unwrap().runtime.presentation.display.viewport.scroll_top;

        // PixelDelta with negative y (scroll down) → dy = 200.0 for plugin
        let effect =
            app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -200.0)));
        assert_eq!(effect, AppEffect::REDRAW);

        let y = app.active_tab_session().unwrap().scroll_y();
        assert!(y > 0.0, "WYSIWYG plugin ScrollY should increase on wheel scroll, got {y}");

        let scroll_top_after =
            app.active_tab_session().unwrap().runtime.presentation.display.viewport.scroll_top;
        assert!(
            (scroll_top_after - scroll_top_before).abs() < 0.01,
            "DocumentView scroll_top should not change for WYSIWYG wheel scroll"
        );
    }

    struct CanvasScrollPlugin;

    impl ui::plugin::ViewPlugin for CanvasScrollPlugin {
        fn name(&self) -> &str {
            "canvas_scroll"
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn is_canvas(&self) -> bool {
            true
        }
    }

    fn app_with_prepared_canvas_viewport() -> App {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app.mouse.pos = (400.0, 300.0);

        let document = DocumentView::new(vec!["canvas".to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(CanvasScrollPlugin));
        app.switch_workspace_for_test(0);

        let tab = app.active_tab_session_mut().expect("test canvas tab must be active");
        let snapshot = tab.runtime.canvas_viewport.prepare(
            ui::plugin::CanvasContentMetrics {
                content_bounds: ui::core::geom::Rect::new(0.0, 0.0, 5_000.0, 5_000.0),
                focus_anchor: None,
            },
            ui::core::geom::Rect::new(0.0, 0.0, 1_000.0, 800.0),
            ui::canvas::CanvasViewportConfig::for_dpi(1.0),
        );
        assert!(snapshot.is_some(), "test canvas viewport must prepare a snapshot");
        app
    }

    #[test]
    fn canvas_pixel_scroll_follows_natural_touchpad_on_both_axes() {
        let mut app = app_with_prepared_canvas_viewport();
        app.apply_canvas_viewport_action(CanvasViewportAction::PanBy(
            ui::canvas::CanvasPoint::new(200.0, 200.0),
        ));
        let before = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");

        assert_eq!(
            app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(36.0, -72.0))),
            AppEffect::REDRAW
        );

        let after = app
            .active_tab_session()
            .expect("test canvas tab must remain active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("canvas viewport snapshot must remain available");
        assert!((after.scroll.x - (before.scroll.x - 36.0)).abs() < 0.001);
        assert!((after.scroll.y - (before.scroll.y + 72.0)).abs() < 0.001);
    }

    #[test]
    fn canvas_command_scroll_zooms_at_mouse_anchor_without_panning() {
        let mut app = app_with_prepared_canvas_viewport();
        let anchor = ui::canvas::CanvasPoint::new(app.mouse.pos.0 as f32, app.mouse.pos.1 as f32);
        let before = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        let content_at_anchor = before.screen_to_content(anchor);
        app.editor_runtime.set_input_modifiers(ModifiersState::SUPER);

        assert_eq!(
            app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(36.0, 0.25))),
            AppEffect::REDRAW
        );

        let after = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        assert!(after.zoom > before.zoom, "command scroll should zoom the canvas");
        let content_after_zoom = after.screen_to_content(anchor);
        assert!((content_after_zoom.x - content_at_anchor.x).abs() < 0.001);
        assert!((content_after_zoom.y - content_at_anchor.y).abs() < 0.001);
    }

    #[test]
    fn canvas_control_scroll_zooms_at_mouse_anchor_without_panning() {
        let mut app = app_with_prepared_canvas_viewport();
        let anchor = ui::canvas::CanvasPoint::new(app.mouse.pos.0 as f32, app.mouse.pos.1 as f32);
        let before = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        let content_at_anchor = before.screen_to_content(anchor);
        app.editor_runtime.set_input_modifiers(ModifiersState::CONTROL);

        assert_eq!(
            app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(36.0, 0.25))),
            AppEffect::REDRAW
        );

        let after = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        assert!(after.zoom > before.zoom, "control scroll should zoom the canvas");
        let content_after_zoom = after.screen_to_content(anchor);
        assert!((content_after_zoom.x - content_at_anchor.x).abs() < 0.001);
        assert!((content_after_zoom.y - content_at_anchor.y).abs() < 0.001);
    }

    #[test]
    fn canvas_shift_scroll_converts_vertical_line_delta_to_horizontal_pan() {
        let mut app = app_with_prepared_canvas_viewport();
        app.apply_canvas_viewport_action(crate::canvas_viewport::CanvasViewportAction::PanBy(
            ui::canvas::CanvasPoint::new(100.0, 100.0),
        ));
        let before = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        app.editor_runtime.set_input_modifiers(ModifiersState::SHIFT);

        assert_eq!(app.handle_scroll(MouseScrollDelta::LineDelta(0.0, 1.0)), AppEffect::REDRAW);

        let after = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        assert!(
            (after.scroll.x - (before.scroll.x - CANVAS_LINE_SCROLL_DISTANCE_PX)).abs() < 0.001
        );
        assert!((after.scroll.y - before.scroll.y).abs() < 0.001);
    }
}
