use std::any::Any;

use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, KeyCode, LayoutCtx, MouseButton,
    PaintCtx, Rect, Widget, WidgetAction,
};

const CHECKBOX_SIZE_LOGICAL: f32 = 20.0;
const CHECKBOX_CORNER_RADIUS_LOGICAL: f32 = 4.0;
const CHECKBOX_BORDER_WIDTH_LOGICAL: f32 = 1.0;
const CHECK_ICON_INSET_LOGICAL: f32 = 5.0;
const CHECK_MARK_START_RATIO: [f32; 2] = [0.14, 0.52];
const CHECK_MARK_MIDDLE_RATIO: [f32; 2] = [0.38, 0.76];
const CHECK_MARK_END_RATIO: [f32; 2] = [0.84, 0.22];
const CHECK_MARK_THICKNESS_RATIO: f32 = 0.14;
const DISABLED_ALPHA_FACTOR: f32 = 0.5;
const SURFACE_HOVER_BLEND: f32 = 0.08;
const BORDER_HOVER_BLEND: f32 = 0.22;
const CHECK_ICON_HOVER_BLEND: f32 = 0.12;

pub struct Checkbox {
    id: WidgetId,
    rect: Rect,
    checked: bool,
    enabled: bool,
    focused: bool,
    hovered: bool,
    pressed: bool,
    accessibility_label: Option<String>,
}

impl Checkbox {
    pub fn new(id: WidgetId, checked: bool) -> Self {
        Self {
            id,
            rect: Rect::ZERO,
            checked,
            enabled: true,
            focused: false,
            hovered: false,
            pressed: false,
            accessibility_label: None,
        }
    }

    pub fn checked(&self) -> bool {
        self.checked
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.focused = false;
            self.hovered = false;
            self.pressed = false;
        }
    }

    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }

    fn toggle_action(&mut self) -> WidgetAction {
        self.checked = !self.checked;
        WidgetAction::Control(ControlAction::Toggled { id: self.id, checked: self.checked })
    }

    fn outline_color(&self, ctx: &PaintCtx) -> [f32; 4] {
        let settings = ctx.theme.settings_theme();
        let base_color = if self.focused { settings.focus_ring } else { settings.control_border };
        let color = if self.hovered && !self.focused {
            blend_color(base_color, settings.text_primary, BORDER_HOVER_BLEND)
        } else {
            base_color
        };
        apply_alpha(color, ctx.global_alpha * self.alpha_factor())
    }

    fn surface_color(&self, ctx: &PaintCtx) -> [f32; 4] {
        let settings = ctx.theme.settings_theme();
        let base_color = if self.checked { settings.accent } else { settings.control_surface };
        let color = if self.hovered {
            blend_color(base_color, settings.text_primary, SURFACE_HOVER_BLEND)
        } else {
            base_color
        };
        apply_alpha(color, ctx.global_alpha * self.alpha_factor())
    }

    fn check_icon_color(&self, ctx: &PaintCtx) -> [f32; 4] {
        let settings = ctx.theme.settings_theme();
        let base_color = settings.text_primary;
        let color = if self.hovered {
            blend_color(base_color, settings.accent, CHECK_ICON_HOVER_BLEND)
        } else {
            base_color
        };
        apply_alpha(color, ctx.global_alpha * self.alpha_factor())
    }

    fn check_icon_rect(&self, dpi: f32) -> Rect {
        let inset = CHECK_ICON_INSET_LOGICAL * dpi;
        Rect::new(
            self.rect.x + inset,
            self.rect.y + inset,
            (self.rect.w - inset * 2.0).max(0.0),
            (self.rect.h - inset * 2.0).max(0.0),
        )
    }

    fn alpha_factor(&self) -> f32 {
        if self.enabled { 1.0 } else { DISABLED_ALPHA_FACTOR }
    }
}

fn point_in_rect(rect: Rect, ratio: [f32; 2]) -> [f32; 2] {
    [rect.x + rect.w * ratio[0], rect.y + rect.h * ratio[1]]
}

fn draw_check_mark(ctx: &mut PaintCtx, icon_rect: Rect, color: [f32; 4]) {
    let start = point_in_rect(icon_rect, CHECK_MARK_START_RATIO);
    let middle = point_in_rect(icon_rect, CHECK_MARK_MIDDLE_RATIO);
    let end = point_in_rect(icon_rect, CHECK_MARK_END_RATIO);
    let thickness = icon_rect.w.min(icon_rect.h) * CHECK_MARK_THICKNESS_RATIO;

    let start_offset = [start[0] + thickness, start[1] - thickness];
    let middle_offset = [middle[0] + thickness, middle[1] - thickness];
    let end_offset = [end[0] + thickness, end[1] - thickness];

    ctx.list.fill_triangle(start, start_offset, middle, color);
    ctx.list.fill_triangle(start_offset, middle_offset, middle, color);
    ctx.list.fill_triangle(middle, middle_offset, end, color);
    ctx.list.fill_triangle(middle_offset, end_offset, end, color);
}

fn blend_color(base: [f32; 4], target: [f32; 4], factor: f32) -> [f32; 4] {
    [
        base[0] + (target[0] - base[0]) * factor,
        base[1] + (target[1] - base[1]) * factor,
        base[2] + (target[2] - base[2]) * factor,
        base[3] + (target[3] - base[3]) * factor,
    ]
}

fn apply_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

impl Widget for Checkbox {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let dpi = ctx.dpi;
        let corner_radius = CHECKBOX_CORNER_RADIUS_LOGICAL * dpi;
        ctx.list.fill_rounded(self.rect, self.surface_color(ctx), corner_radius);
        ctx.list.stroke_rounded(
            self.rect,
            self.outline_color(ctx),
            corner_radius,
            CHECKBOX_BORDER_WIDTH_LOGICAL * dpi,
        );

        if self.checked {
            let icon_rect = self.check_icon_rect(dpi);
            if icon_rect.w > 0.0 && icon_rect.h > 0.0 {
                draw_check_mark(ctx, icon_rect, self.check_icon_color(ctx));
            }
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn is_focusable(&self) -> bool {
        self.enabled
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused = self.enabled && focused_id == Some(self.id);
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let mut node = AccessibilityNode::new(
            AccessibilityId::from(self.id),
            AccessibilityRole::CheckBox,
            ctx.screen_bounds(self.rect),
        )
        .with_disabled(!self.enabled)
        .with_focused(self.focused)
        .with_checked(self.checked);
        if let Some(label) = &self.accessibility_label {
            node = node.with_name(label.clone());
        }
        if self.enabled {
            node = node
                .with_action(AccessibilityAction::Focus)
                .with_action(AccessibilityAction::Toggle);
        }
        Some(node)
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if !self.enabled || request.target != AccessibilityId::from(self.id) {
            return None;
        }
        match request.action {
            AccessibilityAction::Focus => {
                Some(WidgetAction::Control(ControlAction::FocusRequested { id: self.id }))
            }
            AccessibilityAction::Activate | AccessibilityAction::Toggle => {
                Some(self.toggle_action())
            }
            _ => None,
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.enabled {
            return None;
        }

        match ev {
            Event::MouseMove { px, py } => {
                let inside = self.hit(*px, *py);
                if inside {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                if inside != self.hovered {
                    self.hovered = inside;
                    return Some(WidgetAction::Consumed);
                }
                None
            }
            Event::PointerLeave => {
                std::mem::take(&mut self.hovered).then_some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                let inside = self.hit(*px, *py);
                self.hovered = inside;
                if !inside {
                    self.pressed = false;
                    return None;
                }
                self.pressed = true;
                ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                if !self.focused {
                    return Some(WidgetAction::Control(ControlAction::FocusRequested {
                        id: self.id,
                    }));
                }
                Some(WidgetAction::Consumed)
            }
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                let inside = self.hit(*px, *py);
                self.hovered = inside;
                if inside {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                let was_pressed = self.pressed;
                self.pressed = false;
                if !was_pressed || !inside {
                    return None;
                }
                Some(self.toggle_action())
            }
            Event::InteractionCancel => {
                let interaction_changed =
                    std::mem::take(&mut self.hovered) | std::mem::take(&mut self.pressed);
                interaction_changed.then_some(WidgetAction::Consumed)
            }
            Event::KeyDown(KeyCode::Char(' '), modifiers)
                if self.focused && *modifiers == crate::core::Modifiers::NONE =>
            {
                Some(self.toggle_action())
            }
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.pressed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::measure::NoopMeasure;
    use crate::core::widget::{LayoutCtx, Modifiers};
    use crate::core::{
        DrawCmd, DrawList, Event, EventCtx, KeyCode, PaintCtx, Rect, Widget, WidgetAction,
    };

    fn focused_checkbox(id: WidgetId, checked: bool) -> Checkbox {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut checkbox = Checkbox::new(id, checked);
        checkbox.set_rect(
            Rect::new(0.0, 0.0, CHECKBOX_SIZE_LOGICAL, CHECKBOX_SIZE_LOGICAL),
            &mut layout_ctx,
        );
        checkbox.set_keyboard_focus(Some(id));
        checkbox
    }

    fn event_ctx() -> EventCtx<'static> {
        let theme = Box::leak(Box::new(crate::theme::test_theme()));
        EventCtx { cursor_hint: None, theme, dpi: 1.0 }
    }

    fn key_space(checkbox: &mut Checkbox) -> Option<WidgetAction> {
        let mut ctx = event_ctx();
        checkbox.on_event(&Event::KeyDown(KeyCode::Char(' '), Modifiers::NONE), &mut ctx)
    }

    fn mouse_down(checkbox: &mut Checkbox, focused: bool) -> Option<WidgetAction> {
        checkbox.set_keyboard_focus(focused.then_some(checkbox.id));
        let mut ctx = event_ctx();
        checkbox.on_event(
            &Event::MouseDown {
                px: CHECKBOX_SIZE_LOGICAL * 0.5,
                py: CHECKBOX_SIZE_LOGICAL * 0.5,
                button: MouseButton::Left,
            },
            &mut ctx,
        )
    }

    fn mouse_up(checkbox: &mut Checkbox) -> Option<WidgetAction> {
        let mut ctx = event_ctx();
        checkbox.on_event(
            &Event::MouseUp {
                px: CHECKBOX_SIZE_LOGICAL * 0.5,
                py: CHECKBOX_SIZE_LOGICAL * 0.5,
                button: MouseButton::Left,
            },
            &mut ctx,
        )
    }

    fn assert_focus_requested(action: Option<WidgetAction>, id: WidgetId) {
        assert_eq!(action, Some(WidgetAction::Control(ControlAction::FocusRequested { id })));
    }

    fn assert_toggle(action: Option<WidgetAction>, id: WidgetId, checked: bool) {
        assert_eq!(action, Some(WidgetAction::Control(ControlAction::Toggled { id, checked })));
    }

    fn paint_for_test(checkbox: &Checkbox) -> DrawList {
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx::new(&mut draw_list, &theme, 1.0);
        checkbox.paint(&mut paint_ctx);
        draw_list
    }

    #[test]
    fn accessibility_exposes_checkbox_state_and_toggles_through_typed_action() {
        let id = WidgetId(90);
        let mut checkbox = focused_checkbox(id, false);
        checkbox.set_accessibility_label(Some("显示行号".into()));
        let context = crate::core::AccessibilityContext::new(4.0, 8.0);
        let node = checkbox.accessibility_node(&context).expect("checkbox should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::CheckBox);
        assert_eq!(node.name.as_deref(), Some("显示行号"));
        assert_eq!(node.bounds, Rect::new(4.0, 8.0, 20.0, 20.0));
        assert!(node.state.focused);
        assert_eq!(node.state.checked, Some(false));
        assert!(node.actions.contains(&crate::core::AccessibilityAction::Toggle));
        assert_eq!(
            checkbox.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.id,
                crate::core::AccessibilityAction::Toggle,
            )),
            Some(WidgetAction::Control(ControlAction::Toggled { id, checked: true }))
        );
        assert!(checkbox.checked());

        checkbox.set_enabled(false);
        let disabled_node =
            checkbox.accessibility_node(&context).expect("disabled checkbox remains discoverable");
        assert!(disabled_node.state.disabled);
        assert!(disabled_node.actions.is_empty());
    }

    fn is_checkbox_outline(cmd: &DrawCmd) -> bool {
        matches!(
            cmd,
            DrawCmd::StrokeRect { rect, radius, .. }
                if rect.w == CHECKBOX_SIZE_LOGICAL
                    && rect.h == CHECKBOX_SIZE_LOGICAL
                    && *radius == CHECKBOX_CORNER_RADIUS_LOGICAL
        )
    }

    #[test]
    fn checkbox_uses_box_visual_and_toggle_action() {
        let mut checkbox = focused_checkbox(WidgetId(21), false);
        assert_toggle(key_space(&mut checkbox), WidgetId(21), true);
        assert!(paint_for_test(&checkbox).cmds.iter().any(is_checkbox_outline));
    }

    #[test]
    fn checkbox_mouse_down_requests_focus_when_not_focused() {
        let mut checkbox = focused_checkbox(WidgetId(22), false);
        assert_focus_requested(mouse_down(&mut checkbox, false), WidgetId(22));
    }

    #[test]
    fn checkbox_mouse_click_toggles_when_press_and_release_hit() {
        let mut checkbox = focused_checkbox(WidgetId(23), false);
        assert_eq!(mouse_down(&mut checkbox, true), Some(WidgetAction::Consumed));
        assert_toggle(mouse_up(&mut checkbox), WidgetId(23), true);
    }

    #[test]
    fn checked_checkbox_paint_emits_check_mark_triangles() {
        let draw_list = paint_for_test(&focused_checkbox(WidgetId(24), true));
        assert!(draw_list.cmds.iter().any(is_checkbox_outline));
        assert!(
            draw_list.cmds.iter().any(|cmd| matches!(cmd, DrawCmd::FillTriangle { .. })),
            "checked checkbox should emit triangle commands for check mark"
        );
    }

    #[test]
    fn checkbox_external_state_sync_is_silent_and_idempotent() {
        let mut checkbox = focused_checkbox(WidgetId(25), false);

        assert!(!checkbox.checked());
        checkbox.set_checked(true);
        checkbox.set_checked(true);

        assert!(checkbox.checked());
        assert_toggle(key_space(&mut checkbox), WidgetId(25), false);
        checkbox.set_checked(false);
        assert!(!checkbox.checked());
    }

    #[test]
    fn disabled_checkbox_clears_interaction_and_rejects_all_input() {
        let id = WidgetId(26);
        let mut checkbox = focused_checkbox(id, false);
        assert_eq!(mouse_down(&mut checkbox, true), Some(WidgetAction::Consumed));
        checkbox.set_enabled(false);

        assert!(!checkbox.is_enabled());
        assert!(!checkbox.is_focusable());
        assert!(!checkbox.is_capturing());
        assert_eq!(key_space(&mut checkbox), None);
        assert_eq!(mouse_up(&mut checkbox), None);

        let mut ctx = event_ctx();
        assert_eq!(
            checkbox.on_event(
                &Event::MouseMove {
                    px: CHECKBOX_SIZE_LOGICAL * 0.5,
                    py: CHECKBOX_SIZE_LOGICAL * 0.5,
                },
                &mut ctx,
            ),
            None
        );
        assert_eq!(ctx.cursor_hint, None);
    }

    #[test]
    fn checkbox_leave_preserves_press_and_cancel_is_idempotent() {
        let id = WidgetId(27);
        let mut checkbox = focused_checkbox(id, false);
        let mut ctx = event_ctx();

        assert_eq!(
            checkbox.on_event(
                &Event::MouseMove {
                    px: CHECKBOX_SIZE_LOGICAL * 0.5,
                    py: CHECKBOX_SIZE_LOGICAL * 0.5,
                },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(mouse_down(&mut checkbox, true), Some(WidgetAction::Consumed));
        assert!(checkbox.is_capturing());
        assert_eq!(checkbox.on_event(&Event::PointerLeave, &mut ctx), Some(WidgetAction::Consumed));
        assert!(!checkbox.hovered);
        assert!(checkbox.is_capturing());

        assert_eq!(
            checkbox.on_event(&Event::InteractionCancel, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!checkbox.is_capturing());
        assert!(!checkbox.checked());
        assert_eq!(checkbox.on_event(&Event::InteractionCancel, &mut ctx), None);
        assert_eq!(mouse_up(&mut checkbox), None);
    }
}
