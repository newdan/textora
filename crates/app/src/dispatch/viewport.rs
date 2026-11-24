//! Routes scroll and zoom actions and computes editor viewport bounds.

use crate::app::App;
use crate::app_effect::AppEffect;
use appkit_shell::canvas_viewport::CanvasViewportAction;

const PREVIEW_TOP_PAD_LOGICAL: f32 = 16.0;
const PREVIEW_HORIZONTAL_PAD_LOGICAL: f32 = 20.0;
const MAX_READING_WIDTH_LOGICAL: f32 = 800.0;
pub(super) const MIN_PLUGIN_VIEWPORT_PX: f32 = 100.0;
const MIN_CANVAS_PINCH_ZOOM_FACTOR: f32 = 0.01;

fn take_top(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.h);
    rect.y += clamped;
    rect.h = (rect.h - clamped).max(0.0);
}

fn take_bottom(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.h);
    rect.h = (rect.h - clamped).max(0.0);
}

fn take_left(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.w);
    rect.x += clamped;
    rect.w = (rect.w - clamped).max(0.0);
}

fn take_right(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.w);
    rect.w = (rect.w - clamped).max(0.0);
}

pub(super) fn projected_editor_rect(
    screen_w: f32,
    screen_h: f32,
    inputs: &crate::ui_shell::ShellInputs,
    inspector_panel_thickness: f32,
) -> ui::core::geom::Rect {
    let mut rect = ui::core::geom::Rect::new(0.0, 0.0, screen_w, screen_h);

    if !inputs.sidebar_visible {
        take_top(&mut rect, ui::title_bar::title_bar_height(inputs.metrics.dpi));
    }

    if inputs.tabs_visible {
        take_top(&mut rect, inputs.tabs_thickness);
    }
    if inputs.sidebar_visible && inputs.sidebar_thickness > 0.0 {
        take_left(&mut rect, inputs.sidebar_thickness);
    }
    if inputs.sidebar_visible {
        take_top(&mut rect, ui::title_bar::title_bar_height(inputs.metrics.dpi));
    }
    if inputs.toc_visible {
        take_left(&mut rect, inputs.toc_thickness);
    }
    if inputs.search_visible {
        take_top(&mut rect, inputs.search_thickness);
    }
    take_bottom(&mut rect, inputs.status_thickness);
    take_right(&mut rect, inspector_panel_thickness);
    take_right(&mut rect, inputs.scrollbar_thickness);

    rect
}

pub(crate) enum ViewportDispatchAction {
    Scrollbar(ui::scrollbar::ScrollbarAction),
    UpdateScrollTop(f64),
    ScrollViewportBy(f64),
    JumpToHeading(usize),
}

impl App {
    pub(crate) fn dispatch_wheel_scroll(
        &mut self,
        delta: winit::event::MouseScrollDelta,
    ) -> AppEffect {
        self.handle_scroll(delta)
    }

    pub(crate) fn dispatch_viewport_action(&mut self, action: ViewportDispatchAction) -> AppEffect {
        match action {
            ViewportDispatchAction::Scrollbar(ui::scrollbar::ScrollbarAction::StartDrag) => {
                AppEffect::REDRAW
            }
            ViewportDispatchAction::Scrollbar(_) => AppEffect::NONE,
            ViewportDispatchAction::UpdateScrollTop(scroll_top) => {
                let line_height = self.ui_metrics().line_height;
                let viewport_height = self.ui_shell.editor_rect().h;
                let handles_own_rendering = self.active_handles_own_rendering();
                if handles_own_rendering && let Some(mut tab) = self.active_tab_session_mut() {
                    let content_h = tab.content_height();
                    let scroll_y = tab.scroll_y();
                    let max_scroll = (content_h - viewport_height).max(0.0);
                    let pixel_scroll = (scroll_top as f32 * line_height).clamp(0.0, max_scroll);
                    let changed = (scroll_y - pixel_scroll).abs() > 0.5;
                    tab.scroll_plugin(pixel_scroll - scroll_y, viewport_height);
                    return if changed { AppEffect::REDRAW } else { AppEffect::NONE };
                }
                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                tab.set_scroll_top_rows(scroll_top, line_height);
                AppEffect::RESHAPE
            }
            ViewportDispatchAction::ScrollViewportBy(amount) => {
                let line_height = self.ui_metrics().line_height;

                // Plugins that handle their own rendering: delegate scroll to
                // the plugin and sync the scrollbar. The amount is a sign-only
                // indicator (+1.0 / -1.0); magnitude comes from plugin_viewport_h.
                if self.active_handles_own_rendering() {
                    let viewport_h = self.plugin_viewport_h();
                    let delta = if amount > 0.0 { viewport_h } else { -viewport_h };
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        tab.scroll_plugin(delta, viewport_h);
                    }
                    // Sync scrollbar from plugin state.
                    let (content_h, scroll_y) = self
                        .active_tab_session()
                        .map(|tab| (tab.content_height(), tab.scroll_y()))
                        .unwrap_or((0.0, 0.0));
                    self.sync_plugin_scrollbar(content_h, scroll_y, line_height, viewport_h);
                    return AppEffect::REDRAW;
                }

                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                tab.scroll_viewport_by_pages(amount, line_height);
                AppEffect::REDRAW
            }
            ViewportDispatchAction::JumpToHeading(index) => {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    tab.scroll_to_heading(index);
                    AppEffect::REDRAW
                } else {
                    AppEffect::NONE
                }
            }
        }
    }

    pub(super) fn dispatch_canvas_scrollbar_action(
        &mut self,
        axis: ui::canvas::CanvasAxis,
        action: ui::scrollbar::ScrollbarAction,
    ) -> AppEffect {
        use ui::scrollbar::ScrollbarAction;

        let viewport_action = match action {
            ScrollbarAction::DragTo(position) => {
                CanvasViewportAction::SetAxisPosition { axis, position: position as f32 }
            }
            ScrollbarAction::PageUp => CanvasViewportAction::Page { axis, direction: -1.0 },
            ScrollbarAction::PageDown => CanvasViewportAction::Page { axis, direction: 1.0 },
            ScrollbarAction::StartDrag
            | ScrollbarAction::EndDrag
            | ScrollbarAction::HoverChanged(_) => return self.canvas_viewport_redraw_effect(),
        };

        self.apply_canvas_viewport_action(viewport_action)
    }

    pub(super) fn dispatch_canvas_pinch(
        &mut self,
        delta: f64,
        screen_anchor: ui::canvas::CanvasPoint,
    ) -> AppEffect {
        if !delta.is_finite() {
            return AppEffect::NONE;
        }

        let factor = 1.0 + delta as f32;
        if !factor.is_finite() {
            return AppEffect::NONE;
        }

        self.apply_canvas_viewport_action(CanvasViewportAction::ZoomBy {
            factor: factor.max(MIN_CANVAS_PINCH_ZOOM_FACTOR),
            screen_anchor,
        })
    }

    fn canvas_viewport_redraw_effect(&self) -> AppEffect {
        if self.active_is_canvas() && self.active_has_canvas_viewport_snapshot() {
            AppEffect::REDRAW
        } else {
            AppEffect::NONE
        }
    }

    /// Compute the plugin render bounds (the same Rect + viewport dimensions
    /// used by the main render pass). Used for the WYSIWYG two-phase hit-test
    /// inter-phase layout refresh mini-render.
    pub(crate) fn plugin_render_bounds(&self) -> ui::core::geom::Rect {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let editor_r = self.plugin_editor_rect();
        let toc_off = self.toc_left_offset();
        let content_top = self.content_top_offset();

        let is_canvas = self.active_is_canvas();
        if is_canvas {
            let offset_x = editor_r.x;
            let offset_y = content_top;
            let viewport_w = editor_r.w;
            let viewport_h = (editor_r.h - (content_top - editor_r.y)).max(MIN_PLUGIN_VIEWPORT_PX);
            return ui::core::geom::Rect::new(offset_x, offset_y, viewport_w, viewport_h);
        }

        let preview_top_pad = PREVIEW_TOP_PAD_LOGICAL * dpi;
        let preview_pad = PREVIEW_HORIZONTAL_PAD_LOGICAL * dpi;
        let reading_base_x = toc_off + preview_pad;
        let physical_w = (editor_r.w - reading_base_x - preview_pad).max(MIN_PLUGIN_VIEWPORT_PX);
        let max_reading_width = MAX_READING_WIDTH_LOGICAL * dpi;
        let viewport_w = physical_w.min(max_reading_width);
        let viewport_h = (editor_r.h - preview_top_pad).max(MIN_PLUGIN_VIEWPORT_PX);
        let offset_x = editor_r.x + reading_base_x + (physical_w - viewport_w) * 0.5;
        let offset_y = content_top + preview_top_pad;

        ui::core::geom::Rect::new(offset_x, offset_y, viewport_w, viewport_h)
    }

    fn plugin_editor_rect(&self) -> ui::core::geom::Rect {
        let cached = self.ui_shell.editor_rect();
        if cached.w > 0.0 && cached.h > 0.0 && !self.ui_shell.dock_is_dirty() {
            return cached;
        }

        let inputs = self.build_shell_inputs();
        projected_editor_rect(
            self.screen_width(),
            self.screen_height(),
            &inputs,
            self.ui_shell.inspector_panel_thickness(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_without_active_view_returns_none() {
        let mut app = App::new(None);
        let effect = app.dispatch_viewport_action(ViewportDispatchAction::ScrollViewportBy(1.0));
        assert_eq!(effect, AppEffect::NONE);
    }

    #[test]
    fn scrollbar_start_drag_requests_redraw() {
        let mut app = App::new(None);
        let effect = app.dispatch_viewport_action(ViewportDispatchAction::Scrollbar(
            ui::scrollbar::ScrollbarAction::StartDrag,
        ));
        assert_eq!(effect, AppEffect::REDRAW);
    }
}
