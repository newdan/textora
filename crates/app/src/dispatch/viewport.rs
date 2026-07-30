//! Viewport dispatch: scrollbar, scroll-by, update-scroll-top, heading jump.
//! Methods on `impl App`, extracted from app_dispatch.rs.

use crate::app::App;
use crate::app_effect::AppEffect;

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
                    tab.send_message(ui::plugin::PluginMessage::Scroll {
                        delta: pixel_scroll - scroll_y,
                        viewport_h: viewport_height,
                    });
                    return if changed { AppEffect::REDRAW } else { AppEffect::NONE };
                }
                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                tab.set_scroll_top_rows(scroll_top, line_height);
                self.last_scroll_time = std::time::Instant::now();
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
                        tab.send_message(ui::plugin::PluginMessage::Scroll { delta, viewport_h });
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
                    tab.send_message(ui::plugin::PluginMessage::ScrollToHeading(index));
                    AppEffect::REDRAW
                } else {
                    AppEffect::NONE
                }
            }
        }
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
