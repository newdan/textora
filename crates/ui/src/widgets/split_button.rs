//! SplitButtonWidget — 主操作与菜单操作分离的通用按钮。

use std::any::Any;

use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, KeyCode, LayoutCtx, Modifiers,
    MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::button::{ButtonStyle, ButtonVisualState};
use crate::widgets::icon::draw_icon;
use crate::widgets::tooltip::TooltipHint;

/// 菜单区域的固定逻辑宽度。
pub const SPLIT_BUTTON_MENU_WIDTH_LOGICAL: f32 = 28.0;
/// 按钮内侧横向留白。
pub const SPLIT_BUTTON_HORIZONTAL_PADDING_LOGICAL: f32 = 10.0;
const SPLIT_BUTTON_ICON_SIZE_LOGICAL: f32 = 14.0;
const SPLIT_BUTTON_ICON_GAP_LOGICAL: f32 = 6.0;
const SPLIT_BUTTON_DIVIDER_INSET_LOGICAL: f32 = 6.0;

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
    icon: Option<String>,
    menu_open: bool,
    hovered_region: Option<SplitButtonRegion>,
    pressed_region: Option<SplitButtonRegion>,
    focused: bool,
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
            icon: None,
            menu_open: false,
            hovered_region: None,
            pressed_region: None,
            focused: false,
        }
    }

    pub fn set_input(&mut self, input: SplitButtonInput) {
        self.input = input;
        if !self.input.enabled {
            self.hovered_region = None;
            self.pressed_region = None;
            self.focused = false;
        }
    }

    pub fn set_action_ids(&mut self, main_action_id: WidgetId, menu_action_id: WidgetId) {
        self.main_action_id = main_action_id;
        self.menu_action_id = menu_action_id;
    }

    pub fn set_icon(&mut self, icon: Option<String>) {
        self.icon = icon;
    }

    pub fn set_menu_open(&mut self, menu_open: bool) {
        self.menu_open = menu_open;
        if menu_open {
            self.pressed_region = None;
        }
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

    fn region_state(&self, region: SplitButtonRegion) -> ButtonVisualState {
        if !self.input.enabled {
            return ButtonVisualState::Disabled;
        }
        if region == SplitButtonRegion::Menu && self.menu_open {
            return ButtonVisualState::Selected;
        }
        if self.hovered_region != Some(region) {
            return ButtonVisualState::Normal;
        }
        if self.pressed_region == Some(region) {
            ButtonVisualState::Pressed
        } else {
            ButtonVisualState::Hovered
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

        let style = ButtonStyle::from_theme(ctx.theme);
        let state = if self.input.enabled {
            ButtonVisualState::Normal
        } else {
            ButtonVisualState::Disabled
        };
        let alpha = ctx.global_alpha;
        let corner_radius = style.corner_radius_logical * ctx.dpi;

        for (region, rect) in
            [(SplitButtonRegion::Main, self.main_rect), (SplitButtonRegion::Menu, self.menu_rect)]
        {
            let region_color = style.background_color(self.region_state(region), alpha);
            ctx.list.clip(rect, |draw_list| {
                draw_list.fill_rounded(self.rect, region_color, corner_radius);
            });
        }

        let divider_color = style.border_color(state, alpha);
        let divider_inset = SPLIT_BUTTON_DIVIDER_INSET_LOGICAL * ctx.dpi;
        ctx.list.fill(
            Rect::new(
                self.menu_rect.x,
                self.menu_rect.y + divider_inset,
                ctx.dpi,
                (self.menu_rect.h - divider_inset * 2.0).max(0.0),
            ),
            divider_color,
        );
        let foreground = style.foreground_color(state, alpha);
        let font_size = style.font_size_logical * ctx.dpi;
        let baseline = self.main_rect.y + self.main_rect.h * 0.5 + font_size * 0.35;
        let content_x = self.main_rect.x + SPLIT_BUTTON_HORIZONTAL_PADDING_LOGICAL * ctx.dpi;
        let text_x = if let Some(icon) = &self.icon {
            let icon_size = SPLIT_BUTTON_ICON_SIZE_LOGICAL * ctx.dpi;
            draw_icon(
                ctx.list,
                icon,
                content_x,
                self.main_rect.y + (self.main_rect.h - icon_size) * 0.5,
                icon_size,
                foreground,
            );
            content_x + icon_size + SPLIT_BUTTON_ICON_GAP_LOGICAL * ctx.dpi
        } else {
            content_x
        };
        ctx.text(text_x, baseline, font_size, foreground, &self.input.label);
        let center_x = self.menu_rect.x + self.menu_rect.w * 0.5;
        let center_y = self.menu_rect.y + self.menu_rect.h * 0.5;
        let arrow_radius = 4.0 * ctx.dpi;
        ctx.list.fill_triangle(
            [center_x - arrow_radius, center_y - arrow_radius * 0.4],
            [center_x + arrow_radius, center_y - arrow_radius * 0.4],
            [center_x, center_y + arrow_radius * 0.6],
            foreground,
        );
        style.paint_outline(ctx, self.rect, state);
        if self.focused && self.input.enabled {
            let mut focus_ring = ctx.theme.settings_theme().focus_ring;
            focus_ring[3] *= alpha;
            ctx.list.stroke_rounded(
                self.rect,
                focus_ring,
                corner_radius,
                ctx.theme.control_metrics().focus_ring_width_logical * ctx.dpi,
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        (self.main_action_id != WidgetId(0)).then_some(self.main_action_id)
    }

    fn is_focusable(&self) -> bool {
        self.input.enabled && self.id().is_some()
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused = self.input.enabled && focused_id == self.id();
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let main_id = self.id()?;
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let root_id = AccessibilityId::from(main_id).named_child("split-button");
        let mut main = AccessibilityNode::new(
            AccessibilityId::from(main_id),
            AccessibilityRole::Button,
            ctx.screen_bounds(self.main_rect),
        )
        .with_name(self.input.label.clone())
        .with_disabled(!self.input.enabled)
        .with_focused(self.focused);
        let mut menu = AccessibilityNode::new(
            AccessibilityId::from(self.menu_action_id),
            AccessibilityRole::Button,
            ctx.screen_bounds(self.menu_rect),
        )
        .with_name(format!("{} 菜单", self.input.label))
        .with_disabled(!self.input.enabled)
        .with_expanded(self.menu_open);
        if self.input.enabled {
            main = main
                .with_action(AccessibilityAction::Focus)
                .with_action(AccessibilityAction::Activate);
            menu = menu.with_action(AccessibilityAction::Activate);
        }
        Some(
            AccessibilityNode::new(root_id, AccessibilityRole::Group, ctx.screen_bounds(self.rect))
                .with_name(self.input.label.clone())
                .with_child(main)
                .with_child(menu),
        )
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if !self.input.enabled {
            return None;
        }
        if request.target == AccessibilityId::from(self.main_action_id) {
            return match request.action {
                AccessibilityAction::Focus => {
                    Some(WidgetAction::Control(ControlAction::FocusRequested {
                        id: self.main_action_id,
                    }))
                }
                AccessibilityAction::Activate => {
                    Some(WidgetAction::Control(ControlAction::Activated {
                        id: self.main_action_id,
                    }))
                }
                _ => None,
            };
        }
        (request.target == AccessibilityId::from(self.menu_action_id)
            && request.action == AccessibilityAction::Activate)
            .then_some(WidgetAction::Control(ControlAction::Activated { id: self.menu_action_id }))
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.input.enabled {
            return None;
        }

        match event {
            Event::PointerLeave => self.hovered_region.take().map(|_| WidgetAction::Consumed),
            Event::InteractionCancel => {
                let interaction_changed =
                    self.hovered_region.take().is_some() | self.pressed_region.take().is_some();
                interaction_changed.then_some(WidgetAction::Consumed)
            }
            Event::MouseMove { px, py } => {
                let next_hovered_region = self.region_at(*px, *py);
                let hover_changed = self.hovered_region != next_hovered_region;
                self.hovered_region = next_hovered_region;
                if self.hovered_region.is_some() {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                    return Some(WidgetAction::Consumed);
                }
                hover_changed.then_some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.pressed_region = self.region_at(*px, *py);
                self.hovered_region = self.pressed_region;
                self.pressed_region?;
                Some(WidgetAction::Consumed)
            }
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                let pressed_region = self.pressed_region.take()?;
                let released_region = self.region_at(*px, *py);
                self.hovered_region = released_region;
                if released_region == Some(pressed_region) {
                    Some(WidgetAction::Control(ControlAction::Activated {
                        id: self.action_id_for(pressed_region),
                    }))
                } else {
                    Some(WidgetAction::Consumed)
                }
            }
            Event::KeyDown(KeyCode::Enter, modifiers)
                if self.focused && *modifiers == Modifiers::NONE =>
            {
                Some(WidgetAction::Control(ControlAction::Activated { id: self.main_action_id }))
            }
            Event::KeyDown(KeyCode::Down, modifiers)
                if self.focused && *modifiers == Modifiers::NONE =>
            {
                Some(WidgetAction::Control(ControlAction::Activated { id: self.menu_action_id }))
            }
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.pressed_region.is_some()
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        self.menu_rect.contains(px, py).then(|| TooltipHint {
            label: format!("更多{}选项", self.input.label),
            target_rect: self.menu_rect,
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paint::{DrawCmd, DrawList};
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
        widget.set_keyboard_focus(Some(WidgetId(41)));
        widget
    }

    fn event_context(theme: &crate::Theme) -> EventCtx<'_> {
        EventCtx::new(theme, 1.0)
    }

    #[test]
    fn accessibility_exposes_two_button_regions_and_requires_keyboard_focus() {
        let mut widget = widget();
        widget.set_menu_open(true);
        widget.set_keyboard_focus(None);
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            None
        );
        widget.set_keyboard_focus(Some(WidgetId(41)));
        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(100.0, 200.0))
            .expect("split button should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::Group);
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.children[0].name.as_deref(), Some("New note"));
        assert!(node.children[0].state.focused);
        assert_eq!(node.children[1].name.as_deref(), Some("New note 菜单"));
        assert_eq!(node.children[1].state.expanded, Some(true));
        assert_eq!(node.children[0].bounds, Rect::new(110.0, 220.0, 132.0, 32.0));
        assert_eq!(
            widget.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.children[1].id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(42) }))
        );
    }

    #[test]
    fn menu_region_exposes_a_tooltip_without_repeating_the_visible_main_label() {
        let widget = widget();
        let menu_rect = widget.menu_rect();

        assert_eq!(
            widget
                .tooltip_at(menu_rect.x + menu_rect.w * 0.5, menu_rect.y + menu_rect.h * 0.5,)
                .map(|hint| hint.label),
            Some("更多New note选项".to_owned())
        );
        assert_eq!(
            widget
                .tooltip_at(
                    widget.main_rect().x + widget.main_rect().w * 0.5,
                    widget.main_rect().y + widget.main_rect().h * 0.5,
                )
                .map(|hint| hint.label),
            None
        );
    }

    #[test]
    fn split_button_leave_preserves_press_and_cancel_clears_it() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::MouseMove { px: 20.0, py: 30.0 }, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 20.0, py: 30.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );
        assert!(widget.is_capturing());
        assert_eq!(
            widget.on_event(&Event::PointerLeave, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert!(widget.is_capturing());
        assert_eq!(
            widget.on_event(&Event::InteractionCancel, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.is_capturing());
        assert_eq!(widget.on_event(&Event::InteractionCancel, &mut context), None);
    }

    #[test]
    fn split_button_matches_standard_button_surface_when_enabled_or_disabled() {
        use crate::button::{Button, ButtonStyle};

        for (mode, system) in [
            (crate::settings::ThemeMode::Light, winit::window::Theme::Light),
            (crate::settings::ThemeMode::Dark, winit::window::Theme::Dark),
        ] {
            let theme = crate::Theme::resolve_builtin(mode, system);
            for dpi in [1.0, 1.5, 2.0] {
                for enabled in [true, false] {
                    let mut split = widget();
                    split.set_keyboard_focus(None);
                    split.set_input(SplitButtonInput { label: String::new(), enabled });
                    let mut regular = Button::new(WidgetId(90), ButtonStyle::from_theme(&theme));
                    regular.set_enabled(enabled);
                    let mut measure = NoopMeasure;
                    let mut layout_context =
                        LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
                    let rect = Rect::new(0.0, 0.0, 160.0 * dpi, 32.0 * dpi);
                    split.set_rect(rect, &mut layout_context);
                    regular.set_rect(rect, &mut layout_context);
                    let mut expected = DrawList::new();
                    let mut context = PaintCtx::new(&mut expected, &theme, dpi);
                    context.global_alpha = 0.5;
                    regular.paint(&mut context);
                    let mut actual = DrawList::new();
                    let mut context = PaintCtx::new(&mut actual, &theme, dpi);
                    context.global_alpha = 0.5;
                    split.paint(&mut context);
                    for command in &expected.cmds {
                        assert!(
                            actual.cmds.contains(command),
                            "分段按钮与普通按钮应共享背景和边框: {command:?}"
                        );
                    }
                }
            }
        }
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

    #[test]
    fn moving_outside_clears_the_hovered_region() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        let _ = widget.on_event(&Event::MouseMove { px: 20.0, py: 30.0 }, &mut context);
        assert_eq!(widget.hovered_region, Some(SplitButtonRegion::Main));

        let _ = widget.on_event(&Event::MouseMove { px: 500.0, py: 500.0 }, &mut context);
        assert_eq!(widget.hovered_region, None);
    }

    #[test]
    fn hover_highlight_is_limited_to_the_target_region_and_keeps_an_outline() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut event_context = event_context(&theme);
        let _ = widget.on_event(&Event::MouseMove { px: 20.0, py: 30.0 }, &mut event_context);
        let mut draw_list = DrawList::new();

        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        assert!(draw_list.cmds.windows(3).any(|commands| {
            matches!(commands[0], DrawCmd::PushClip(rect) if rect == widget.main_rect())
                && matches!(
                    commands[1],
                    DrawCmd::FillRect { rect, color, .. }
                        if rect == widget.rect && color == theme.application_theme().button_hover_surface
                )
                && matches!(commands[2], DrawCmd::PopClip)
        }));
        assert!(draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::StrokeRect { rect, color, .. }
                    if *rect == widget.rect
                        && *color == theme.application_theme().button_border
            )
        }));
        assert!(!draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::FillRect { rect, color, .. }
                    if *rect == widget.menu_rect()
                        && *color == theme.application_theme().button_hover_surface
            )
        }));
    }

    #[test]
    fn translucent_active_regions_each_paint_their_final_background_once() {
        let theme = crate::theme::test_theme();
        let style = ButtonStyle::from_theme(&theme);
        for region in [SplitButtonRegion::Main, SplitButtonRegion::Menu] {
            for state in [
                ButtonVisualState::Hovered,
                ButtonVisualState::Pressed,
                ButtonVisualState::Selected,
            ] {
                let mut widget = widget();
                widget.set_keyboard_focus(None);
                if state == ButtonVisualState::Selected {
                    if region == SplitButtonRegion::Main {
                        continue;
                    }
                    widget.set_menu_open(true);
                } else {
                    widget.hovered_region = Some(region);
                    widget.pressed_region = (state == ButtonVisualState::Pressed).then_some(region);
                }
                let mut draw_list = DrawList::new();
                let mut context = PaintCtx::new(&mut draw_list, &theme, 1.0);
                context.global_alpha = 0.5;
                widget.paint(&mut context);
                for (target, rect) in [
                    (SplitButtonRegion::Main, widget.main_rect()),
                    (SplitButtonRegion::Menu, widget.menu_rect()),
                ] {
                    let target_state =
                        if target == region { state } else { ButtonVisualState::Normal };
                    let expected = style.background_color(target_state, 0.5);
                    assert!(draw_list.cmds.windows(3).any(|commands| {
                        matches!(commands[0], DrawCmd::PushClip(bounds) if bounds == rect)
                            && matches!(commands[1], DrawCmd::FillRect { color, .. } if color == expected)
                            && matches!(commands[2], DrawCmd::PopClip)
                    }), "每个分段应只绘制自身最终背景，不能叠加半透明底色");
                }
                assert_eq!(
                    draw_list
                        .cmds
                        .iter()
                        .filter(|command| matches!(command,
                            DrawCmd::FillRect { rect, .. } if *rect == widget.rect
                        ))
                        .count(),
                    2,
                    "每个区域只允许一次背景填充"
                );
            }
        }
    }

    #[test]
    fn translucent_hover_highlight_does_not_overlap_itself() {
        let mut widget = widget();
        let mut theme = crate::theme::test_theme();
        theme.palette.bg_hover[3] = 0.5;
        let mut event_context = event_context(&theme);
        let _ = widget.on_event(&Event::MouseMove { px: 20.0, py: 30.0 }, &mut event_context);
        let mut draw_list = DrawList::new();

        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        let hover_fill_count = draw_list
            .cmds
            .iter()
            .filter(|command| {
                matches!(command, DrawCmd::FillRect { color, .. } if *color == theme.application_theme().button_hover_surface)
            })
            .count();
        assert_eq!(hover_fill_count, 1, "hover 色不得在同一区域重复合成");
    }

    #[test]
    fn open_menu_keeps_only_the_menu_region_active() {
        let mut widget = widget();
        widget.set_menu_open(true);
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();

        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        assert!(draw_list.cmds.windows(3).any(|commands| {
            matches!(commands[0], DrawCmd::PushClip(rect) if rect == widget.menu_rect())
                && matches!(
                    commands[1],
                    DrawCmd::FillRect { rect, color, .. }
                        if rect == widget.rect && color == theme.application_theme().button_pressed_surface
                )
                && matches!(commands[2], DrawCmd::PopClip)
        }));
        assert!(!draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::FillRect { rect, color, .. }
                    if *rect == widget.main_rect()
                        && *color == theme.application_theme().button_pressed_surface
            )
        }));
    }
}
