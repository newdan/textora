//! TabBarWidget — Phase 6: 真正 widget 实现。
//! 持有 TabBarState，paint 走 to_drawlist，事件走 on_event。

use super::layout::max_tab_scroll;
use super::{TabBarAction, TabBarCtx, TabBarInput, TabBarState, TabHit, tab_bar_height};
use crate::core::geom::Rect;
use crate::core::paint::DrawList;
use crate::core::text_util::estimate_text_width_px;
use crate::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget, WidgetAction};
use crate::widgets::tooltip::TooltipHint;
use std::any::Any;
use winit::window::CursorIcon;

/// Thin widget wrapper around TabBarState.
pub struct TabBarWidget {
    rect: Rect,
    state: TabBarState,
    active_index: usize,
    input: Option<TabBarWidgetInput>,
}

/// Owned input data for `TabBarWidget`, injected each frame by the app layer.
#[derive(Debug, Clone)]
pub struct TabBarWidgetInput {
    pub tabs: Vec<super::TabInfo>,
    pub active_index: Option<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_size_px: (f32, f32),
    pub hovered_index: Option<usize>,
    pub scroll_offset_px: f32,
    pub metrics: crate::settings::UiMetrics,
}

impl Default for TabBarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TabBarWidget {
    pub fn new() -> Self {
        Self { rect: Rect::ZERO, state: TabBarState::new(), active_index: 0, input: None }
    }

    /// Single entry point: inject owned input data and re-layout.
    pub fn set_input(&mut self, input: TabBarWidgetInput, shaper: Option<&mut shaping::Shaper>) {
        self.active_index = input.active_index.unwrap_or(0);
        self.state.set_hovered_index(input.hovered_index);
        self.state.set_scroll_offset(input.scroll_offset_px);
        let borrowed = TabBarInput {
            tabs: &input.tabs,
            active_index: input.active_index,
            back_enabled: input.back_enabled,
            forward_enabled: input.forward_enabled,
            screen_w: input.screen_size_px.0,
            screen_h: input.screen_size_px.1,
        };
        self.state.update_layout(&borrowed, shaper, input.metrics.dpi);

        // Autoscroll: keep active tab visible
        if let Some(active_idx) = input.active_index {
            let current = self.state.scroll_offset();
            if let Some((target, _)) = self.autoscroll_target(active_idx, current) {
                self.state.set_scroll_target(target);
            }
        }

        self.input = Some(input);
    }

    /// Expose state for event handling (mouse move → hover update).
    pub fn state(&self) -> &TabBarState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut TabBarState {
        &mut self.state
    }

    /// Check if the active tab is visible and return `(target_scroll, max_scroll)` if not.
    /// Uses the layout from the most recent `set_input` call.
    pub fn autoscroll_target(
        &self,
        active_index: usize,
        current_scroll: f32,
    ) -> Option<(f32, f32)> {
        let layout = self.state.current_layout()?;
        let active_tab = layout.tabs.iter().find(|t| t.index == active_index)?;
        let left_px = active_tab.rect_px.x;
        let right_px = active_tab.rect_px.x + active_tab.rect_px.w;
        let is_visible = left_px >= layout.clip_left_px && right_px <= layout.clip_right_px;
        if is_visible {
            return None;
        }
        let tab_center_px = (left_px + right_px) * 0.5;
        let viewport_center_px = (layout.clip_left_px + layout.clip_right_px) * 0.5;
        let delta_px = tab_center_px - viewport_center_px;
        Some(((current_scroll + delta_px).clamp(0.0, layout.max_scroll), layout.max_scroll))
    }

    pub fn scroll_by(&mut self, delta: f32) {
        self.state.scroll_by(delta);
    }

    pub fn scroll_target(&self) -> f32 {
        self.state.scroll_target()
    }
}

impl Widget for TabBarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let mut dl = DrawList::new();
        self.state.to_drawlist(
            self.active_index,
            ctx.theme,
            ctx.dpi,
            &mut dl,
            ctx.shaper.as_deref_mut(),
        );
        ctx.list.cmds.extend(dl.cmds);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            Event::MouseDown { px, py, button } => {
                self.state.on_click_px(*px, *py, *button).map(WidgetAction::TabBar)
            }
            Event::MouseMove { px, py } => {
                // Guard: only set cursor_hint when mouse is inside widget rect
                let inside = self.rect.contains(*px, *py);
                self.state.on_mouse_move_px(*px, *py);
                let hit = self.state.hit_test_px(*px, *py);
                if inside {
                    ctx.cursor_hint = Some(match &hit {
                        Some(TabHit::Tab(_))
                        | Some(TabHit::Close(_))
                        | Some(TabHit::NewTab)
                        | Some(TabHit::ScrollLeft)
                        | Some(TabHit::ScrollRight)
                        | Some(TabHit::Dropdown) => CursorIcon::Pointer,
                        _ => CursorIcon::Default,
                    });
                }
                Some(WidgetAction::TabBar(match hit {
                    Some(TabHit::Tab(idx)) => TabBarAction::HoverTab(Some(idx)),
                    _ => TabBarAction::HoverTab(None),
                }))
            }
            Event::Wheel { dx, .. } => {
                if let Some(ref input) = self.input {
                    let dpi = input.metrics.dpi;
                    let ctx_tab_bar = TabBarCtx {
                        screen_w: input.screen_size_px.0,
                        screen_h: input.screen_size_px.1,
                        dpi,
                    };
                    let max =
                        max_tab_scroll(input.tabs.len(), &ctx_tab_bar, tab_bar_height(ctx.dpi));
                    let old = self.state.scroll_offset();
                    let new = (old + dx * -40.0).clamp(0.0, max);
                    self.state.set_scroll_offset(new);
                    let input_ref = TabBarInput {
                        tabs: &input.tabs,
                        active_index: input.active_index,
                        back_enabled: input.back_enabled,
                        forward_enabled: input.forward_enabled,
                        screen_w: input.screen_size_px.0,
                        screen_h: input.screen_size_px.1,
                    };
                    self.state.update_layout(&input_ref, None, dpi);
                }
                Some(WidgetAction::TabBar(TabBarAction::ScrollRight))
            }
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        let layout = self.state.current_layout()?;
        let dpi = self.input.as_ref().map(|i| i.metrics.dpi).unwrap_or(1.0);
        let font_size = crate::constants::TITLE_FONT_SIZE * dpi;

        if let Some(TabHit::Tab(idx)) = self.state.hit_test_px(px, py) {
            let tab = layout.tabs.iter().find(|t| t.index == idx)?;
            let padding = 16.0 * dpi;
            let avail_w = tab.rect_px.w - padding;
            if avail_w > 0.0 && estimate_text_width_px(&tab.title, font_size) > avail_w {
                return Some(TooltipHint { label: tab.title.clone(), target_rect: tab.rect_px });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::NoopMeasure;
    use crate::core::widget::MouseButton;

    fn metrics(dpi: f32) -> crate::settings::UiMetrics {
        crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi)
    }

    fn make_tabs(n: usize) -> Vec<super::super::TabInfo> {
        (0..n)
            .map(|i| super::super::TabInfo {
                title: format!("tab_{i}.rs"),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            })
            .collect()
    }

    fn setup_widget(n_tabs: usize) -> TabBarWidget {
        let mut w = TabBarWidget::new();
        let tabs = make_tabs(n_tabs);
        w.set_input(
            TabBarWidgetInput {
                tabs,
                active_index: Some(0),
                back_enabled: false,
                forward_enabled: false,
                screen_size_px: (800.0, 600.0),
                hovered_index: None,
                scroll_offset_px: 0.0,
                metrics: metrics(1.0),
            },
            None,
        );
        // Layout the widget so hit testing works
        let t = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut ctx = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 800.0, 32.0), &mut ctx);
        w
    }

    #[test]
    fn mouse_move_on_tab_returns_hover_tab_some() {
        let mut w = setup_widget(3);
        let t = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&t, 1.0);
        // First tab should be near the left edge
        let result = w.on_event(&Event::MouseMove { px: 30.0, py: 10.0 }, &mut ctx);
        match result {
            Some(WidgetAction::TabBar(TabBarAction::HoverTab(Some(idx)))) => {
                assert_eq!(idx, 0, "Should hover first tab");
            }
            other => panic!("Expected HoverTab(Some(0)), got {:?}", other),
        }
    }

    #[test]
    fn mouse_move_outside_tabs_returns_hover_tab_none() {
        let mut w = setup_widget(1);
        let t = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&t, 1.0);
        // Far right beyond any tab
        let result = w.on_event(&Event::MouseMove { px: 750.0, py: 10.0 }, &mut ctx);
        match result {
            Some(WidgetAction::TabBar(TabBarAction::HoverTab(None))) => {}
            other => panic!("Expected HoverTab(None), got {:?}", other),
        }
    }

    #[test]
    fn mouse_move_no_longer_returns_switch_tab() {
        let mut w = setup_widget(3);
        let t = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(&Event::MouseMove { px: 30.0, py: 10.0 }, &mut ctx);
        assert!(
            !matches!(result, Some(WidgetAction::TabBar(TabBarAction::SwitchTab(_)))),
            "MouseMove should never return SwitchTab"
        );
    }

    #[test]
    fn mouse_down_still_returns_switch_tab() {
        let mut w = setup_widget(3);
        let t = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(
            &Event::MouseDown { px: 30.0, py: 10.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(
            matches!(result, Some(WidgetAction::TabBar(TabBarAction::SwitchTab(_)))),
            "MouseDown should still return SwitchTab, got {:?}",
            result
        );
    }

    #[test]
    fn autoscroll_target_returns_none_when_active_tab_visible() {
        // With few tabs on a wide screen, all tabs are visible → no autoscroll needed
        let w = setup_widget(2);
        let target = w.autoscroll_target(0, 0.0);
        assert!(target.is_none(), "Active tab 0 should be visible, no autoscroll needed");
    }

    #[test]
    fn autoscroll_target_returns_target_when_active_tab_hidden() {
        // Many tabs on a narrow screen, last tab should be hidden

        let mut w = TabBarWidget::new();
        let tabs = make_tabs(20);
        // Narrow screen (400px) with 20 tabs → overflow
        w.set_input(
            TabBarWidgetInput {
                tabs,
                active_index: Some(19),
                back_enabled: false,
                forward_enabled: false,
                screen_size_px: (400.0, 600.0),
                hovered_index: None,
                scroll_offset_px: 0.0,
                metrics: metrics(1.0),
            },
            None,
        );
        let t = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut ctx = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 400.0, 32.0), &mut ctx);

        let result = w.autoscroll_target(19, 0.0);
        assert!(result.is_some(), "Active tab 19 should be hidden at scroll_offset=0");
        let (target, max_scroll) = result.unwrap();
        assert!(target > 0.0, "Target should be positive to scroll right");
        assert!(max_scroll > 0.0, "max_scroll should be positive with 20 tabs");
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    fn metrics(dpi: f32) -> crate::settings::UiMetrics {
        crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi)
    }

    fn tab(title: &str, pinned: bool) -> super::super::TabInfo {
        super::super::TabInfo {
            title: title.into(),
            file_path: None,
            is_dirty: false,
            pinned,
            language: String::new(),
        }
    }

    #[test]
    fn set_input_replaces_every_frame_field() {
        let mut widget = TabBarWidget::new();
        widget.set_input(
            TabBarWidgetInput {
                tabs: vec![tab("first", true)],
                active_index: Some(0),
                back_enabled: true,
                forward_enabled: false,
                screen_size_px: (800.0, 600.0),
                hovered_index: Some(0),
                scroll_offset_px: 30.0,
                metrics: metrics(2.0),
            },
            None,
        );
        widget.set_input(
            TabBarWidgetInput {
                tabs: vec![tab("second", false)],
                active_index: None,
                back_enabled: false,
                forward_enabled: true,
                screen_size_px: (400.0, 300.0),
                hovered_index: None,
                scroll_offset_px: 0.0,
                metrics: metrics(1.0),
            },
            None,
        );

        let input = widget.input.as_ref().unwrap();
        assert_eq!(input.tabs[0].title, "second");
        assert_eq!(input.active_index, None);
        assert!(!input.back_enabled);
        assert!(input.forward_enabled);
        assert_eq!(input.screen_size_px, (400.0, 300.0));
        assert_eq!(widget.state.hovered_index(), None);
        assert_eq!(widget.state.scroll_offset(), 0.0);
        assert_eq!(input.metrics.dpi, 1.0);
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    fn make_tabs(n: usize) -> Vec<super::super::TabInfo> {
        (0..n)
            .map(|i| super::super::TabInfo {
                title: format!("tab{i}"),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            })
            .collect()
    }

    fn metrics(dpi: f32) -> crate::settings::UiMetrics {
        crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi)
    }

    #[test]
    fn scroll_by_clamps_to_zero_before_layout() {
        let mut state = super::super::TabBarState::new();
        state.scroll_by(100.0);
        assert_eq!(state.scroll_target(), 0.0, "no layout → max_scroll=0 → clamp to 0");
    }

    #[test]
    fn scroll_by_updates_target_after_layout() {
        let mut widget = TabBarWidget::new();
        widget.set_input(
            TabBarWidgetInput {
                tabs: make_tabs(20),
                active_index: Some(0),
                back_enabled: false,
                forward_enabled: false,
                screen_size_px: (200.0, 600.0),
                hovered_index: None,
                scroll_offset_px: 0.0,
                metrics: metrics(1.0),
            },
            None,
        );
        let t = crate::theme::test_theme();
        let mut m = crate::core::measure::NoopMeasure;
        let mut ctx = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        widget.set_rect(Rect::new(0.0, 0.0, 200.0, 32.0), &mut ctx);

        widget.scroll_by(-50.0);
        assert!(
            widget.scroll_target() > 0.0,
            "scroll_by with negative delta should increase target"
        );
    }

    #[test]
    fn set_scroll_target_direct() {
        let mut state = super::super::TabBarState::new();
        state.set_scroll_target(42.0);
        assert_eq!(state.scroll_target(), 42.0);
    }
}
