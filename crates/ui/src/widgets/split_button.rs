//! SplitButtonWidget — 主操作与菜单操作分离的通用按钮。

use std::any::Any;

use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};

/// 菜单区域的固定逻辑宽度。
pub const SPLIT_BUTTON_MENU_WIDTH_LOGICAL: f32 = 28.0;
/// 按钮内侧横向留白。
pub const SPLIT_BUTTON_HORIZONTAL_PADDING_LOGICAL: f32 = 10.0;
/// 按钮标签字号。
pub const SPLIT_BUTTON_FONT_SIZE_LOGICAL: f32 = 14.0;

/// Split button 的纯展示输入。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitButtonInput {
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitButtonRegion {
    Main,
    Menu,
}

/// 主按钮和菜单按钮动作分别由调用方提供 `WidgetId` 映射。
pub struct SplitButtonWidget {
    rect: Rect,
    main_rect: Rect,
    menu_rect: Rect,
    input: SplitButtonInput,
    main_action_id: WidgetId,
    menu_action_id: WidgetId,
    hovered_region: Option<SplitButtonRegion>,
    pressed_region: Option<SplitButtonRegion>,
}

impl Default for SplitButtonWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SplitButtonWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            main_rect: Rect::ZERO,
            menu_rect: Rect::ZERO,
            input: SplitButtonInput { enabled: true, ..SplitButtonInput::default() },
            main_action_id: WidgetId(0),
            menu_action_id: WidgetId(0),
            hovered_region: None,
            pressed_region: None,
        }
    }

    pub fn set_input(&mut self, input: SplitButtonInput) {
        self.input = input;
        if !self.input.enabled {
            self.hovered_region = None;
            self.pressed_region = None;
        }
    }

    pub fn set_action_ids(&mut self, main_action_id: WidgetId, menu_action_id: WidgetId) {
        self.main_action_id = main_action_id;
        self.menu_action_id = menu_action_id;
    }

    pub fn main_rect(&self) -> Rect {
        self.main_rect
    }

    pub fn menu_rect(&self) -> Rect {
        self.menu_rect
    }

    fn region_at(&self, px: f32, py: f32) -> Option<SplitButtonRegion> {
        if self.main_rect.contains(px, py) {
            Some(SplitButtonRegion::Main)
        } else if self.menu_rect.contains(px, py) {
            Some(SplitButtonRegion::Menu)
        } else {
            None
        }
    }

    fn action_id_for(&self, region: SplitButtonRegion) -> WidgetId {
        match region {
            SplitButtonRegion::Main => self.main_action_id,
            SplitButtonRegion::Menu => self.menu_action_id,
        }
    }
}

impl Widget for SplitButtonWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let menu_width = (SPLIT_BUTTON_MENU_WIDTH_LOGICAL * ctx.dpi).min(rect.w);
        self.main_rect = Rect::new(rect.x, rect.y, (rect.w - menu_width).max(0.0), rect.h);
        self.menu_rect = Rect::new(rect.right() - menu_width, rect.y, menu_width, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let background = if !self.input.enabled {
            ctx.theme.palette.bg_surface
        } else if self.pressed_region.is_some() {
            ctx.theme.palette.bg_active
        } else if self.hovered_region.is_some() {
            ctx.theme.palette.bg_hover
        } else {
            ctx.theme.palette.bg_elevated
        };
        let alpha = ctx.global_alpha;
        let mut fill_color = background;
        fill_color[3] *= alpha;
        ctx.list.fill_rounded(self.rect, fill_color, 6.0 * ctx.dpi);

        let mut divider_color = ctx.theme.palette.border_subtle;
        divider_color[3] *= alpha;
        ctx.list.fill(
            Rect::new(
                self.menu_rect.x,
                self.menu_rect.y + 4.0 * ctx.dpi,
                ctx.dpi,
                (self.menu_rect.h - 8.0 * ctx.dpi).max(0.0),
            ),
            divider_color,
        );
        let foreground = if self.input.enabled {
            ctx.theme.palette.text_main
        } else {
            ctx.theme.palette.text_muted
        };
        let font_size = SPLIT_BUTTON_FONT_SIZE_LOGICAL * ctx.dpi;
        let baseline = self.main_rect.y + self.main_rect.h * 0.5 + font_size * 0.35;
        ctx.text(
            self.main_rect.x + SPLIT_BUTTON_HORIZONTAL_PADDING_LOGICAL * ctx.dpi,
            baseline,
            font_size,
            foreground,
            &self.input.label,
        );
        let center_x = self.menu_rect.x + self.menu_rect.w * 0.5;
        let center_y = self.menu_rect.y + self.menu_rect.h * 0.5;
        let arrow_radius = 4.0 * ctx.dpi;
        ctx.list.fill_triangle(
            [center_x - arrow_radius, center_y - arrow_radius * 0.4],
            [center_x + arrow_radius, center_y - arrow_radius * 0.4],
            [center_x, center_y + arrow_radius * 0.6],
            foreground,
        );
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.input.enabled {
            return None;
        }

        match event {
            Event::MouseMove { px, py } => {
                let next_hovered_region = self.region_at(*px, *py);
                if next_hovered_region.is_none() && !self.is_capturing() {
                    return None;
                }
                self.hovered_region = next_hovered_region;
                ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                Some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.pressed_region = self.region_at(*px, *py);
                self.pressed_region.map(|_| WidgetAction::Consumed)
            }
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                let pressed_region = self.pressed_region.take()?;
                let released_region = self.region_at(*px, *py);
                if released_region == Some(pressed_region) {
                    Some(WidgetAction::Control(ControlAction::Activated {
                        id: self.action_id_for(pressed_region),
                    }))
                } else {
                    Some(WidgetAction::Consumed)
                }
            }
            Event::KeyDown(KeyCode::Enter, _) => {
                Some(WidgetAction::Control(ControlAction::Activated { id: self.main_action_id }))
            }
            Event::KeyDown(KeyCode::Down, _) => {
                Some(WidgetAction::Control(ControlAction::Activated { id: self.menu_action_id }))
            }
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.pressed_region.is_some()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{EventCtx, LayoutCtx, Modifiers, NoopMeasure};

    fn layout(widget: &mut SplitButtonWidget, rect: Rect, dpi: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        widget.set_rect(rect, &mut context);
    }

    fn widget() -> SplitButtonWidget {
        let mut widget = SplitButtonWidget::new();
        widget.set_action_ids(WidgetId(41), WidgetId(42));
        widget.set_input(SplitButtonInput { label: "New note".to_owned(), enabled: true });
        layout(&mut widget, Rect::new(10.0, 20.0, 160.0, 32.0), 1.0);
        widget
    }

    fn event_context(theme: &crate::Theme) -> EventCtx<'_> {
        EventCtx { theme, dpi: 1.0, cursor_hint: None }
    }

    #[test]
    fn creates_a_split_button() {
        let widget = SplitButtonWidget::new();
        assert_eq!(widget.main_rect(), Rect::ZERO);
        assert_eq!(widget.menu_rect(), Rect::ZERO);
    }

    #[test]
    fn layout_scales_menu_region_with_dpi() {
        let mut widget = SplitButtonWidget::new();
        layout(&mut widget, Rect::new(0.0, 0.0, 200.0, 60.0), 2.0);

        assert_eq!(widget.menu_rect().w, SPLIT_BUTTON_MENU_WIDTH_LOGICAL * 2.0);
        assert_eq!(widget.main_rect().right(), widget.menu_rect().x);
    }

    #[test]
    fn main_and_menu_regions_emit_distinct_control_actions() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 20.0, py: 30.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseUp { px: 20.0, py: 30.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(41) }))
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 155.0, py: 30.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseUp { px: 155.0, py: 30.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(42) }))
        );
    }

    #[test]
    fn pointer_capture_prevents_activation_after_release_outside() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        let _ = widget.on_event(
            &Event::MouseDown { px: 20.0, py: 30.0, button: MouseButton::Left },
            &mut context,
        );
        assert!(widget.is_capturing());
        assert_eq!(
            widget.on_event(
                &Event::MouseUp { px: 500.0, py: 500.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.is_capturing());
    }

    #[test]
    fn keyboard_triggers_the_main_or_menu_action() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(41) }))
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Down, Modifiers::NONE), &mut context),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(42) }))
        );
    }
}
