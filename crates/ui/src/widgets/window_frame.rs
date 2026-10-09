//! 沉浸式窗口外壳。只接收标题与平台窗口状态，不访问产品模型。

use crate::core::text_layout::UiTextLayout;
use crate::core::widget::{PointerClickKind, PointerClickTracker};
use crate::{Event, MouseButton, PaintCtx, Rect};
use std::sync::Arc;
use winit::window::{CursorIcon, ResizeDirection};

const TITLE_HEIGHT_LOGICAL: f32 = 36.0;
const CONTROL_WIDTH_LOGICAL: f32 = 46.0;
const CONTROL_ICON_SIZE_LOGICAL: f32 = 14.0;
const TITLE_INSET_LOGICAL: f32 = 16.0;
const TITLE_FONT_SIZE_LOGICAL: f32 = 16.0;
const TITLE_FONT_FAMILY: &str = "Segoe UI";
const TITLE_FONT_WEIGHT: shaping::Weight = shaping::Weight::SEMIBOLD;
const TEXT_BASELINE_EM: f32 = 0.35;
const RESIZE_MARGIN_LOGICAL: f32 = 6.0;
const FRAME_BORDER_PHYSICAL: f32 = 1.0;
const RESTORE_SQUARE_LOGICAL: f32 = 9.0;
const RESTORE_OFFSET_LOGICAL: f32 = 3.0;
const NAVIGATION_BUTTON_SIZE_LOGICAL: f32 = 28.0;
const TITLE_NAVIGATION_GAP_LOGICAL: f32 = 8.0;
const NAVIGATION_ICON_WIDTH_LOGICAL: f32 = 18.0;
const NAVIGATION_ICON_HEIGHT_LOGICAL: f32 = 14.0;
const NAVIGATION_ICON_RADIUS_LOGICAL: f32 = 3.0;
const NAVIGATION_ICON_DIVIDER_LOGICAL: f32 = 5.0;
const NAVIGATION_ICON_STROKE_LOGICAL: f32 = 1.25;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowFrameNavigationToggle {
    #[default]
    Hidden,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowFrameState {
    #[default]
    Native,
    Restored,
    Maximized,
}

impl WindowFrameState {
    pub fn title_height(self, dpi: f32) -> f32 {
        if self == Self::Native { 0.0 } else { TITLE_HEIGHT_LOGICAL * dpi }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowFrameInput {
    pub title: String,
    pub state: WindowFrameState,
    pub navigation_toggle: WindowFrameNavigationToggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowFrameAction {
    ToggleNavigation,
    Drag,
    Resize(ResizeDirection),
    Minimize,
    ToggleMaximize,
    Close,
}

#[derive(Default)]
pub struct WindowFrameEvent {
    pub consumed: bool,
    pub needs_redraw: bool,
    pub action: Option<WindowFrameAction>,
    pub cursor: Option<CursorIcon>,
}

#[derive(Default)]
pub struct WindowFrameWidget {
    input: WindowFrameInput,
    window_rect: Rect,
    dpi: f32,
    title_layout: Option<Arc<UiTextLayout>>,
    hovered_control: Option<usize>,
    hovered_navigation: bool,
    title_clicks: PointerClickTracker,
    pointer_origin: PointerOrigin,
}

#[derive(Clone, Copy, Default)]
enum PointerOrigin {
    #[default]
    Released,
    Content(MouseButton),
}

impl WindowFrameWidget {
    pub fn set_input(&mut self, input: WindowFrameInput, window_rect: Rect, dpi: f32) {
        let dpi = dpi.max(1.0);
        if self.input.title != input.title || self.dpi != dpi {
            self.title_layout = None;
        }
        self.input = input;
        self.window_rect = window_rect;
        self.dpi = dpi;
    }

    fn title_rect(&self) -> Rect {
        Rect::new(
            self.window_rect.x,
            self.window_rect.y,
            self.window_rect.w,
            self.input.state.title_height(self.dpi).min(self.window_rect.h),
        )
    }

    fn control_rects(&self) -> [Rect; 3] {
        let title = self.title_rect();
        let width = (CONTROL_WIDTH_LOGICAL * self.dpi).min(title.w / 3.0);
        std::array::from_fn(|index| {
            Rect::new(title.right() - (3 - index) as f32 * width, title.y, width, title.h)
        })
    }

    fn navigation_toggle_rect(&self) -> Rect {
        if self.input.navigation_toggle == WindowFrameNavigationToggle::Hidden {
            return Rect::ZERO;
        }
        let title = self.title_rect();
        let size = NAVIGATION_BUTTON_SIZE_LOGICAL * self.dpi;
        let left = self.title_text_rect().right() + TITLE_NAVIGATION_GAP_LOGICAL * self.dpi;
        if title.h < size || left + size > self.control_rects()[0].x {
            return Rect::ZERO;
        }
        Rect::new(left, title.y + (title.h - size) * 0.5, size, size)
    }

    fn title_text_rect(&self) -> Rect {
        let title = self.title_rect();
        let inset = TITLE_INSET_LOGICAL * self.dpi;
        let left = title.x + inset;
        let available_width = (self.control_rects()[0].x - left - inset).max(0.0);
        let navigation_width =
            if self.input.navigation_toggle == WindowFrameNavigationToggle::Hidden {
                0.0
            } else {
                (NAVIGATION_BUTTON_SIZE_LOGICAL + TITLE_NAVIGATION_GAP_LOGICAL) * self.dpi
            };
        let width =
            self.title_layout.as_ref().map(|layout| layout.shaped.width).unwrap_or_else(|| {
                crate::core::text_util::estimate_text_width_px(
                    &self.input.title,
                    TITLE_FONT_SIZE_LOGICAL * self.dpi,
                )
            });
        Rect::new(left, title.y, width.min((available_width - navigation_width).max(0.0)), title.h)
    }

    fn prepare_title_layout(&mut self, context: &mut PaintCtx<'_>) {
        if self.title_layout.is_some() {
            return;
        }
        let Some(shaper) = context.shaper.as_mut() else {
            return;
        };
        self.title_layout = UiTextLayout::new(
            &self.input.title,
            TITLE_FONT_SIZE_LOGICAL * self.dpi,
            Some(TITLE_FONT_FAMILY.to_owned()),
            TITLE_FONT_WEIGHT,
            shaping::Style::Normal,
            false,
            shaper,
        )
        .map(Arc::new);
    }

    fn resize_direction(&self, px: f32, py: f32) -> Option<ResizeDirection> {
        if self.input.state != WindowFrameState::Restored || !self.window_rect.contains(px, py) {
            return None;
        }
        let margin = RESIZE_MARGIN_LOGICAL * self.dpi;
        let left = px < self.window_rect.x + margin;
        let right = px >= self.window_rect.right() - margin;
        let top = py < self.window_rect.y + margin;
        let bottom = py >= self.window_rect.bottom() - margin;
        match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        }
    }

    pub fn on_event(&mut self, event: &Event) -> WindowFrameEvent {
        let previous_hover = (self.hovered_control, self.hovered_navigation);
        let mut route = match (self.pointer_origin, event) {
            (PointerOrigin::Content(button), Event::MouseUp { button: released_button, .. })
                if button == *released_button =>
            {
                self.pointer_origin = PointerOrigin::Released;
                self.hovered_control = None;
                self.hovered_navigation = false;
                WindowFrameEvent::default()
            }
            (PointerOrigin::Content(_), Event::MouseMove { .. }) => {
                self.hovered_control = None;
                self.hovered_navigation = false;
                WindowFrameEvent::default()
            }
            _ => self.route_event(event),
        };
        route.needs_redraw = previous_hover != (self.hovered_control, self.hovered_navigation);
        route
    }

    fn route_event(&mut self, event: &Event) -> WindowFrameEvent {
        if self.input.state == WindowFrameState::Native {
            return WindowFrameEvent::default();
        }
        let (px, py) = match event {
            Event::MouseMove { px, py }
            | Event::MouseDown { px, py, .. }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => (*px, *py),
            Event::PointerLeave | Event::InteractionCancel => {
                self.hovered_control = None;
                self.hovered_navigation = false;
                self.title_clicks.reset();
                if matches!(event, Event::InteractionCancel) {
                    self.pointer_origin = PointerOrigin::Released;
                }
                return WindowFrameEvent::default();
            }
            _ => return WindowFrameEvent::default(),
        };
        self.hovered_control = self.control_rects().iter().position(|rect| rect.contains(px, py));
        self.hovered_navigation = self.navigation_toggle_rect().contains(px, py);
        if let Some(direction) = self.resize_direction(px, py) {
            self.hovered_control = None;
            self.hovered_navigation = false;
            let pressed = matches!(event, Event::MouseDown { button: MouseButton::Left, .. });
            return WindowFrameEvent {
                consumed: true,
                action: pressed.then_some(WindowFrameAction::Resize(direction)),
                cursor: Some(direction.into()),
                ..WindowFrameEvent::default()
            };
        }
        if !self.title_rect().contains(px, py) {
            if let Event::MouseDown { button, .. } = event {
                self.pointer_origin = PointerOrigin::Content(*button);
                self.title_clicks.reset();
            }
            return WindowFrameEvent::default();
        }
        if self.hovered_navigation {
            self.title_clicks.reset();
            return WindowFrameEvent {
                consumed: true,
                action: (self.input.navigation_toggle == WindowFrameNavigationToggle::Enabled
                    && matches!(event, Event::MouseDown { button: MouseButton::Left, .. }))
                .then_some(WindowFrameAction::ToggleNavigation),
                cursor: Some(CursorIcon::Default),
                ..WindowFrameEvent::default()
            };
        }
        let action = matches!(event, Event::MouseDown { button: MouseButton::Left, .. })
            .then(|| self.title_press_action(px, py));
        WindowFrameEvent {
            consumed: true,
            action,
            cursor: Some(CursorIcon::Default),
            ..WindowFrameEvent::default()
        }
    }

    fn title_press_action(&mut self, px: f32, py: f32) -> WindowFrameAction {
        if let Some(index) = self.hovered_control {
            self.title_clicks.reset();
            return [
                WindowFrameAction::Minimize,
                WindowFrameAction::ToggleMaximize,
                WindowFrameAction::Close,
            ][index];
        }
        if self.title_clicks.record_press((px, py)) == PointerClickKind::Double {
            self.title_clicks.reset();
            WindowFrameAction::ToggleMaximize
        } else {
            WindowFrameAction::Drag
        }
    }

    pub fn paint_title(&mut self, context: &mut PaintCtx<'_>) {
        let title = self.title_rect();
        if title.w <= 0.0 || title.h <= 0.0 {
            return;
        }
        self.prepare_title_layout(context);
        let application = context.theme.application_theme();
        context.list.fill(title, application.window_surface);
        self.paint_navigation_toggle(context);
        let controls = self.control_rects();
        let text_rect = self.title_text_rect();
        let font_size = TITLE_FONT_SIZE_LOGICAL * self.dpi;
        let baseline = title.y + title.h * 0.5 + font_size * TEXT_BASELINE_EM;
        if let Some(layout) = &self.title_layout {
            context.list.clip(text_rect, |list| {
                list.text_layout(
                    Arc::clone(layout),
                    text_rect.x,
                    baseline,
                    application.text_primary,
                );
            });
        }
        for (index, rect) in controls.into_iter().enumerate() {
            let hovered = self.hovered_control == Some(index);
            let foreground = if hovered && index == 2 {
                application.text_inverse
            } else {
                application.text_secondary
            };
            if hovered {
                context.list.fill(
                    rect,
                    if index == 2 {
                        application.danger
                    } else {
                        application.navigation_hover_surface
                    },
                );
            }
            self.paint_control(context, rect, index, foreground);
        }
    }

    fn paint_navigation_toggle(&self, context: &mut PaintCtx<'_>) {
        let rect = self.navigation_toggle_rect();
        if rect == Rect::ZERO {
            return;
        }
        let application = context.theme.application_theme();
        if self.hovered_navigation
            && self.input.navigation_toggle == WindowFrameNavigationToggle::Enabled
        {
            context.list.fill_rounded(
                rect,
                application.navigation_hover_surface,
                NAVIGATION_ICON_RADIUS_LOGICAL * self.dpi,
            );
        }
        let icon = Rect::new(
            rect.x + (rect.w - NAVIGATION_ICON_WIDTH_LOGICAL * self.dpi) * 0.5,
            rect.y + (rect.h - NAVIGATION_ICON_HEIGHT_LOGICAL * self.dpi) * 0.5,
            NAVIGATION_ICON_WIDTH_LOGICAL * self.dpi,
            NAVIGATION_ICON_HEIGHT_LOGICAL * self.dpi,
        );
        let stroke = NAVIGATION_ICON_STROKE_LOGICAL * self.dpi;
        context.list.stroke_rounded(
            icon,
            application.text_secondary,
            NAVIGATION_ICON_RADIUS_LOGICAL * self.dpi,
            stroke,
        );
        context.list.fill(
            Rect::new(icon.x + NAVIGATION_ICON_DIVIDER_LOGICAL * self.dpi, icon.y, stroke, icon.h),
            application.text_secondary,
        );
    }

    fn paint_control(&self, context: &mut PaintCtx<'_>, rect: Rect, index: usize, color: [f32; 4]) {
        if index == 1 && self.input.state == WindowFrameState::Maximized {
            self.paint_restore_control(context, rect, color);
            return;
        }
        let icon_size = CONTROL_ICON_SIZE_LOGICAL * self.dpi;
        let icon = match index {
            0 => "minus",
            1 => "maximize",
            _ => "x",
        };
        crate::icon::draw_icon(
            context.list,
            icon,
            rect.x + (rect.w - icon_size) * 0.5,
            rect.y + (rect.h - icon_size) * 0.5,
            icon_size,
            color,
        );
    }

    fn paint_restore_control(&self, context: &mut PaintCtx<'_>, rect: Rect, color: [f32; 4]) {
        let size = RESTORE_SQUARE_LOGICAL * self.dpi;
        let offset = RESTORE_OFFSET_LOGICAL * self.dpi;
        let left = rect.x + (rect.w - size - offset) * 0.5;
        let top = rect.y + (rect.h - size - offset) * 0.5;
        context.list.stroke_rounded(
            Rect::new(left + offset, top, size, size),
            color,
            0.0,
            self.dpi,
        );
        let front = Rect::new(left, top + offset, size, size);
        let application = context.theme.application_theme();
        let background = if self.hovered_control == Some(1) {
            application.navigation_hover_surface
        } else {
            application.window_surface
        };
        context.list.fill(front, background);
        context.list.stroke_rounded(front, color, 0.0, self.dpi);
    }

    /// 最后绘制，确保内容或弹窗不会覆盖窗口边界。
    pub fn paint_border(&self, context: &mut PaintCtx<'_>) {
        if self.input.state != WindowFrameState::Restored {
            return;
        }
        let rect = self.window_rect;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let color = context.theme.application_theme().strong_border;
        let radius = (crate::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL * self.dpi)
            .min(rect.w * 0.5)
            .min(rect.h * 0.5);
        context.list.stroke_rounded(rect, color, radius, FRAME_BORDER_PHYSICAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawCmd, DrawList, Theme};

    fn frame(state: WindowFrameState, dpi: f32) -> WindowFrameWidget {
        let mut widget = WindowFrameWidget::default();
        widget.set_input(
            WindowFrameInput { title: "notora".to_owned(), state, ..WindowFrameInput::default() },
            Rect::new(0.0, 0.0, 1200.0 * dpi, 800.0 * dpi),
            dpi,
        );
        widget
    }

    fn press(px: f32, py: f32) -> Event {
        Event::MouseDown { px, py, button: MouseButton::Left }
    }

    #[test]
    fn title_precedes_navigation_toggle_and_uses_readable_system_typography() {
        let mut shaper =
            shaping::Shaper::new().expect("title typography test should load system fonts");
        let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        for dpi in [1.0, 1.5, 2.0] {
            let mut widget = frame(WindowFrameState::Restored, dpi);
            widget.input.navigation_toggle = WindowFrameNavigationToggle::Enabled;
            let mut list = DrawList::new();
            let mut context = PaintCtx::new(&mut list, &theme, dpi);
            context.shaper = Some(&mut shaper);
            widget.paint_title(&mut context);
            let (layout, text_x) = list
                .cmds
                .iter()
                .find_map(|command| {
                    if let DrawCmd::TextLayout { layout, x, .. } = command {
                        Some((layout, *x))
                    } else {
                        None
                    }
                })
                .expect("title should emit shaped text");
            assert!(
                widget.navigation_toggle_rect().x > text_x + layout.shaped.width,
                "navigation toggle must follow the rendered title"
            );
            assert_eq!(text_x, 16.0 * dpi);
            assert_eq!(layout.font_size, 16.0 * dpi);
            assert_eq!(layout.font_weight, shaping::Weight::SEMIBOLD);
            assert_eq!(layout.font_family.as_deref(), Some("Segoe UI"));
            let toggle = widget.navigation_toggle_rect();
            assert_eq!(
                widget
                    .on_event(&press(toggle.x + toggle.w * 0.5, toggle.y + toggle.h * 0.5))
                    .action,
                Some(WindowFrameAction::ToggleNavigation)
            );
        }
    }

    #[test]
    fn navigation_toggle_uses_a_fixed_title_bar_target_and_never_drags_the_window() {
        for dpi in [1.0, 1.5, 2.0] {
            let mut widget = frame(WindowFrameState::Restored, dpi);
            widget.input.navigation_toggle = WindowFrameNavigationToggle::Enabled;
            let rect = widget.navigation_toggle_rect();
            let click = press(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
            assert!(rect.x > TITLE_INSET_LOGICAL * dpi);
            assert!(rect.bottom() < widget.title_rect().bottom());
            for _ in 0..2 {
                let route = widget.on_event(&click);
                assert!(route.consumed);
                assert_eq!(route.action, Some(WindowFrameAction::ToggleNavigation));
            }
            widget.input.navigation_toggle = WindowFrameNavigationToggle::Disabled;
            let route = widget.on_event(&click);
            assert!(route.consumed);
            assert_eq!(route.action, None);
            widget.input.state = WindowFrameState::Native;
            assert_eq!(widget.navigation_toggle_rect(), Rect::ZERO);
            assert!(!widget.on_event(&click).consumed);
        }
    }

    #[test]
    fn title_drag_double_click_and_controls_have_distinct_actions() {
        let mut widget = frame(WindowFrameState::Restored, 1.0);
        assert_eq!(widget.on_event(&press(100.0, 18.0)).action, Some(WindowFrameAction::Drag));
        assert_eq!(
            widget.on_event(&press(100.0, 18.0)).action,
            Some(WindowFrameAction::ToggleMaximize)
        );
        for (rect, expected) in widget.control_rects().into_iter().zip([
            WindowFrameAction::Minimize,
            WindowFrameAction::ToggleMaximize,
            WindowFrameAction::Close,
        ]) {
            assert_eq!(
                widget.on_event(&press(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)).action,
                Some(expected)
            );
        }
        assert!(!widget.on_event(&press(300.0, 200.0)).consumed);
    }

    #[test]
    fn edges_resize_at_each_dpi_but_maximized_windows_do_not() {
        for dpi in [1.0, 1.5, 2.0] {
            let mut widget = frame(WindowFrameState::Restored, dpi);
            for (px, py, direction) in [
                (1.0, 1.0, ResizeDirection::NorthWest),
                (1199.0, 1.0, ResizeDirection::NorthEast),
                (1.0, 799.0, ResizeDirection::SouthWest),
                (1199.0, 799.0, ResizeDirection::SouthEast),
                (1.0, 400.0, ResizeDirection::West),
                (1199.0, 400.0, ResizeDirection::East),
                (600.0, 1.0, ResizeDirection::North),
                (600.0, 799.0, ResizeDirection::South),
            ] {
                assert_eq!(
                    widget.on_event(&press(px * dpi, py * dpi)).action,
                    Some(WindowFrameAction::Resize(direction))
                );
            }
        }
        let mut widget = frame(WindowFrameState::Maximized, 1.0);
        assert_eq!(widget.resize_direction(1.0, 1.0), None);
        assert_eq!(widget.on_event(&press(1199.0, 18.0)).action, Some(WindowFrameAction::Close));
    }

    #[test]
    fn frame_uses_warm_shell_surface_and_the_same_radius_as_content() {
        let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        for dpi in [1.0, 1.5, 2.0] {
            let mut widget = frame(WindowFrameState::Restored, dpi);
            let mut list = DrawList::new();
            widget.paint_title(&mut PaintCtx::new(&mut list, &theme, dpi));
            assert!(
                matches!(list.cmds.first(), Some(DrawCmd::FillRect { rect, color, .. }) if rect.h == TITLE_HEIGHT_LOGICAL * dpi && *color == theme.application_theme().window_surface)
            );
            list.cmds.clear();
            widget.paint_border(&mut PaintCtx::new(&mut list, &theme, dpi));
            assert_eq!(list.cmds.len(), 1);
            assert!(matches!(list.cmds[0], DrawCmd::StrokeRect { radius, line_width, color, .. }
                    if radius == crate::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL * dpi
                        && line_width == FRAME_BORDER_PHYSICAL
                        && color == theme.application_theme().strong_border));
        }
    }

    #[test]
    fn maximized_frame_has_no_rounded_outer_border() {
        let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        let widget = frame(WindowFrameState::Maximized, 1.0);
        let mut list = DrawList::new();
        widget.paint_border(&mut PaintCtx::new(&mut list, &theme, 1.0));
        assert!(list.cmds.is_empty());
    }

    #[test]
    fn native_frame_does_not_paint_or_consume_product_events() {
        let mut widget = frame(WindowFrameState::Native, 1.0);
        assert!(!widget.on_event(&press(100.0, 18.0)).consumed);
        let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_dark());
        let mut list = DrawList::new();
        let mut context = PaintCtx::new(&mut list, &theme, 1.0);
        widget.paint_title(&mut context);
        widget.paint_border(&mut context);
        assert!(list.cmds.is_empty());
    }

    #[test]
    fn content_drag_keeps_move_and_release_when_it_crosses_the_title_bar() {
        let mut widget = frame(WindowFrameState::Restored, 1.0);
        assert!(!widget.on_event(&press(300.0, 200.0)).consumed);
        assert!(!widget.on_event(&Event::MouseMove { px: 300.0, py: 18.0 }).consumed);
        assert!(
            !widget
                .on_event(&Event::MouseUp { px: 300.0, py: 18.0, button: MouseButton::Left })
                .consumed
        );
        assert!(widget.on_event(&Event::MouseMove { px: 300.0, py: 18.0 }).consumed);
    }
}
