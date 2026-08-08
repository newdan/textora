use std::any::Any;

use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, KeyCode, LayoutCtx, MouseButton,
    PaintCtx, Rect, Widget, WidgetAction,
};

const SWITCH_WIDTH_LOGICAL: f32 = 36.0;
const SWITCH_HEIGHT_LOGICAL: f32 = 20.0;
const SWITCH_BORDER_WIDTH_LOGICAL: f32 = 1.0;
const SWITCH_THUMB_INSET_LOGICAL: f32 = 2.0;
const DISABLED_ALPHA_FACTOR: f32 = 0.5;
const UNCHECKED_HOVER_BLEND: f32 = 0.08;
const CHECKED_HOVER_BLEND: f32 = 0.12;
const BORDER_HOVER_BLEND: f32 = 0.22;
const THUMB_HOVER_BLEND: f32 = 0.18;

pub struct Switch {
    id: WidgetId,
    rect: Rect,
    checked: bool,
    enabled: bool,
    focused: bool,
    hovered: bool,
    pressed: bool,
    accessibility_label: Option<String>,
}

impl Switch {
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

    fn track_rect_with_dpi(&self, dpi: f32) -> Rect {
        let width = (SWITCH_WIDTH_LOGICAL * dpi).min(self.rect.w);
        let height = (SWITCH_HEIGHT_LOGICAL * dpi).min(self.rect.h);
        Rect::new(
            self.rect.right() - width,
            self.rect.y + (self.rect.h - height) * 0.5,
            width,
            height,
        )
    }

    fn thumb_rect(&self, dpi: f32) -> Rect {
        let track_rect = self.track_rect_with_dpi(dpi);
        let inset = SWITCH_THUMB_INSET_LOGICAL * dpi;
        let thumb_size = (track_rect.h - inset * 2.0).max(0.0);
        let thumb_x = if self.checked {
            track_rect.right() - inset - thumb_size
        } else {
            track_rect.x + inset
        };
        Rect::new(thumb_x, track_rect.y + inset, thumb_size, thumb_size)
    }

    fn track_color(&self, ctx: &PaintCtx) -> [f32; 4] {
        let settings = ctx.theme.settings_theme();
        let application = ctx.theme.application_theme();
        let base_color = if self.checked { settings.accent } else { settings.control_surface };
        let hover_target =
            if self.checked { application.text_inverse } else { settings.text_primary };
        let hover_blend = if self.checked { CHECKED_HOVER_BLEND } else { UNCHECKED_HOVER_BLEND };
        let color = if self.hovered {
            blend_color(base_color, hover_target, hover_blend)
        } else {
            base_color
        };
        apply_alpha(color, ctx.global_alpha * self.alpha_factor())
    }

    fn border_color(&self, ctx: &PaintCtx) -> [f32; 4] {
        let settings = ctx.theme.settings_theme();
        let base_color = if self.focused {
            settings.focus_ring
        } else if self.checked {
            settings.accent
        } else {
            settings.control_border
        };
        let color = if self.hovered && !self.focused {
            blend_color(base_color, settings.text_primary, BORDER_HOVER_BLEND)
        } else {
            base_color
        };
        apply_alpha(color, ctx.global_alpha * self.alpha_factor())
    }

    fn thumb_color(&self, ctx: &PaintCtx) -> [f32; 4] {
        let settings = ctx.theme.settings_theme();
        let application = ctx.theme.application_theme();
        let base_color =
            if self.checked { application.text_inverse } else { settings.text_secondary };
        let hover_target =
            if self.checked { application.text_primary } else { settings.text_primary };
        let color = if self.hovered {
            blend_color(base_color, hover_target, THUMB_HOVER_BLEND)
        } else {
            base_color
        };
        apply_alpha(color, ctx.global_alpha * self.alpha_factor())
    }

    fn alpha_factor(&self) -> f32 {
        if self.enabled { 1.0 } else { DISABLED_ALPHA_FACTOR }
    }
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

impl Widget for Switch {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let dpi = ctx.dpi;
        let metrics = ctx.theme.control_metrics();
        let track_rect = self.track_rect_with_dpi(dpi);
        let track_radius = track_rect.h * 0.5;
        let border_width = if self.focused {
            metrics.focus_ring_width_logical
        } else {
            SWITCH_BORDER_WIDTH_LOGICAL
        } * dpi;
        ctx.list.fill_rounded(track_rect, self.track_color(ctx), track_radius);
        ctx.list.stroke_rounded(track_rect, self.border_color(ctx), track_radius, border_width);

        let thumb_rect = self.thumb_rect(dpi);
        let thumb_radius = thumb_rect.h * 0.5;
        ctx.list.fill_rounded(thumb_rect, self.thumb_color(ctx), thumb_radius);
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
            AccessibilityRole::Switch,
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
    use crate::core::{DrawCmd, DrawList, PaintCtx};
    use crate::core::{Event, EventCtx, KeyCode, MouseButton, Rect, Widget, WidgetAction};

    fn make_switch(id: WidgetId, checked: bool) -> Switch {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut switch = Switch::new(id, checked);
        switch.set_rect(
            Rect::new(0.0, 0.0, SWITCH_WIDTH_LOGICAL, SWITCH_HEIGHT_LOGICAL),
            &mut layout_ctx,
        );
        switch
    }

    fn focused_switch(id: WidgetId, checked: bool) -> Switch {
        let mut switch = make_switch(id, checked);
        switch.set_keyboard_focus(Some(id));
        switch
    }

    fn event_ctx() -> EventCtx<'static> {
        let theme = Box::leak(Box::new(crate::theme::test_theme()));
        EventCtx { cursor_hint: None, theme, dpi: 1.0 }
    }

    fn mouse_down(switch: &mut Switch, px: f32, py: f32) -> Option<WidgetAction> {
        let mut ctx = event_ctx();
        switch.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut ctx)
    }

    fn mouse_up(switch: &mut Switch, px: f32, py: f32) -> Option<WidgetAction> {
        let mut ctx = event_ctx();
        switch.on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut ctx)
    }

    fn click(switch: &mut Switch) -> Option<WidgetAction> {
        let mut ctx = event_ctx();
        assert_eq!(
            switch.on_event(
                &Event::MouseDown { px: 18.0, py: 10.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        switch.on_event(&Event::MouseUp { px: 18.0, py: 10.0, button: MouseButton::Left }, &mut ctx)
    }

    fn key_space(switch: &mut Switch) -> Option<WidgetAction> {
        let mut ctx = event_ctx();
        switch.on_event(&Event::KeyDown(KeyCode::Char(' '), Modifiers::NONE), &mut ctx)
    }

    fn assert_toggle(action: Option<WidgetAction>, id: WidgetId, checked: bool) {
        assert_eq!(action, Some(WidgetAction::Control(ControlAction::Toggled { id, checked })));
    }

    fn paint_commands(switch: &Switch) -> Vec<DrawCmd> {
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx::new(&mut draw_list, &theme, 1.0);
        switch.paint(&mut paint_ctx);
        draw_list.cmds
    }

    #[test]
    fn accessibility_exposes_switch_state_and_toggles_through_typed_action() {
        let id = WidgetId(90);
        let mut switch = focused_switch(id, true);
        switch.set_accessibility_label(Some("自动换行".into()));
        let context = crate::core::AccessibilityContext::new(7.0, 9.0);
        let node = switch.accessibility_node(&context).expect("switch should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::Switch);
        assert_eq!(node.name.as_deref(), Some("自动换行"));
        assert_eq!(node.bounds, Rect::new(7.0, 9.0, 36.0, 20.0));
        assert!(node.state.focused);
        assert_eq!(node.state.checked, Some(true));
        assert!(node.actions.contains(&crate::core::AccessibilityAction::Toggle));
        assert_eq!(
            switch.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.id,
                crate::core::AccessibilityAction::Toggle,
            )),
            Some(WidgetAction::Control(ControlAction::Toggled { id, checked: false }))
        );
        assert!(!switch.checked());

        switch.set_enabled(false);
        let disabled_node =
            switch.accessibility_node(&context).expect("disabled switch remains discoverable");
        assert!(disabled_node.state.disabled);
        assert!(disabled_node.actions.is_empty());
    }

    fn fill_color(cmd: &DrawCmd) -> [f32; 4] {
        match cmd {
            DrawCmd::FillRect { color, .. } => *color,
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    fn stroke_color(cmd: &DrawCmd) -> [f32; 4] {
        match cmd {
            DrawCmd::StrokeRect { color, .. } => *color,
            other => panic!("expected StrokeRect, got {other:?}"),
        }
    }

    #[test]
    fn switch_toggles_with_click_and_space() {
        let mut switch = focused_switch(WidgetId(20), false);
        assert_toggle(click(&mut switch), WidgetId(20), true);
        assert_toggle(key_space(&mut switch), WidgetId(20), false);
    }

    #[test]
    fn switch_requests_focus_on_first_mouse_down() {
        let mut switch = make_switch(WidgetId(21), false);
        let mut ctx = event_ctx();

        assert_eq!(
            switch.on_event(
                &Event::MouseDown { px: 18.0, py: 10.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(21) }))
        );
    }

    #[test]
    fn switch_requires_matching_press_before_mouse_up_toggle() {
        let mut switch = focused_switch(WidgetId(22), false);

        assert_eq!(mouse_up(&mut switch, 18.0, 10.0), None);
        assert_eq!(mouse_down(&mut switch, 100.0, 100.0), None);
        assert_eq!(mouse_up(&mut switch, 18.0, 10.0), None);

        assert_eq!(mouse_down(&mut switch, 18.0, 10.0), Some(WidgetAction::Consumed));
        assert_eq!(mouse_up(&mut switch, 100.0, 100.0), None);
        assert_eq!(mouse_up(&mut switch, 18.0, 10.0), None);
    }

    #[test]
    fn switch_track_keeps_mac_size_and_trailing_alignment_in_wide_control_column() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut switch = Switch::new(WidgetId(99), false);
        switch.set_rect(Rect::new(0.0, 0.0, 220.0, 32.0), &mut layout);

        assert_eq!(switch.track_rect_with_dpi(1.0), Rect::new(184.0, 6.0, 36.0, 20.0));
    }

    #[test]
    fn switch_hovered_paint_uses_distinct_colors() {
        let idle_switch = make_switch(WidgetId(23), false);
        let mut hovered_switch = make_switch(WidgetId(24), false);
        let mut ctx = event_ctx();
        assert_eq!(
            hovered_switch.on_event(&Event::MouseMove { px: 18.0, py: 10.0 }, &mut ctx,),
            Some(WidgetAction::Consumed)
        );

        let idle_commands = paint_commands(&idle_switch);
        let hovered_commands = paint_commands(&hovered_switch);

        let idle_track_color = fill_color(&idle_commands[0]);
        let hovered_track_color = fill_color(&hovered_commands[0]);
        let idle_border_color = stroke_color(&idle_commands[1]);
        let hovered_border_color = stroke_color(&hovered_commands[1]);
        let idle_thumb_color = fill_color(&idle_commands[2]);
        let hovered_thumb_color = fill_color(&hovered_commands[2]);

        assert_ne!(idle_track_color, hovered_track_color);
        assert_ne!(idle_border_color, hovered_border_color);
        assert_ne!(idle_thumb_color, hovered_thumb_color);
    }

    #[test]
    fn checked_switch_uses_a_light_thumb_and_accent_border_in_light_theme() {
        let switch = Switch::new(WidgetId(25), true);
        let light_theme =
            crate::theme::Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        let mut draw_list = DrawList::new();
        let paint_ctx = PaintCtx::new(&mut draw_list, &light_theme, 1.0);

        let thumb_color = switch.thumb_color(&paint_ctx);
        assert!(thumb_color[0] >= 0.95 && thumb_color[1] >= 0.95 && thumb_color[2] >= 0.95);
        assert_eq!(switch.border_color(&paint_ctx), light_theme.palette.accent);
    }

    #[test]
    fn checked_switch_uses_semantic_thumb_and_focus_metrics_across_themes_and_dpi() {
        for theme in [
            crate::theme::Theme::resolve_builtin(
                crate::settings::ThemeMode::Light,
                winit::window::Theme::Light,
            ),
            crate::theme::Theme::resolve_builtin(
                crate::settings::ThemeMode::Dark,
                winit::window::Theme::Dark,
            ),
        ] {
            for dpi in [1.0, 2.0] {
                let application = theme.application_theme();
                let metrics = theme.control_metrics();
                let mut switch = Switch::new(WidgetId(92), true);
                switch.set_keyboard_focus(Some(WidgetId(92)));
                switch.rect =
                    Rect::new(0.0, 0.0, SWITCH_WIDTH_LOGICAL * dpi, SWITCH_HEIGHT_LOGICAL * dpi);

                let mut draw_list = DrawList::new();
                switch.paint(&mut PaintCtx::new(&mut draw_list, &theme, dpi));

                assert_eq!(fill_color(&draw_list.cmds[2]), application.text_inverse);
                match &draw_list.cmds[1] {
                    DrawCmd::StrokeRect { line_width, .. } => {
                        assert_eq!(*line_width, metrics.focus_ring_width_logical * dpi);
                    }
                    other => panic!("expected StrokeRect, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn switch_external_state_sync_is_silent_and_idempotent() {
        let mut switch = focused_switch(WidgetId(26), false);

        assert!(!switch.checked());
        switch.set_checked(true);
        switch.set_checked(true);

        assert!(switch.checked());
        assert_toggle(key_space(&mut switch), WidgetId(26), false);
        switch.set_checked(false);
        assert!(!switch.checked());
    }

    #[test]
    fn disabled_switch_clears_interaction_and_rejects_all_input() {
        let id = WidgetId(27);
        let mut switch = focused_switch(id, false);
        assert_eq!(mouse_down(&mut switch, 18.0, 10.0), Some(WidgetAction::Consumed));
        switch.set_enabled(false);

        assert!(!switch.is_enabled());
        assert!(!switch.is_focusable());
        assert!(!switch.is_capturing());
        assert_eq!(key_space(&mut switch), None);
        assert_eq!(mouse_up(&mut switch, 18.0, 10.0), None);

        let mut ctx = event_ctx();
        assert_eq!(switch.on_event(&Event::MouseMove { px: 18.0, py: 10.0 }, &mut ctx), None);
        assert_eq!(ctx.cursor_hint, None);
    }

    #[test]
    fn switch_leave_preserves_press_and_cancel_is_idempotent() {
        let id = WidgetId(28);
        let mut switch = focused_switch(id, false);
        let mut ctx = event_ctx();

        assert_eq!(
            switch.on_event(&Event::MouseMove { px: 18.0, py: 10.0 }, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(mouse_down(&mut switch, 18.0, 10.0), Some(WidgetAction::Consumed));
        assert!(switch.is_capturing());
        assert_eq!(switch.on_event(&Event::PointerLeave, &mut ctx), Some(WidgetAction::Consumed));
        assert!(!switch.hovered);
        assert!(switch.is_capturing());

        assert_eq!(
            switch.on_event(&Event::InteractionCancel, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!switch.is_capturing());
        assert!(!switch.checked());
        assert_eq!(switch.on_event(&Event::InteractionCancel, &mut ctx), None);
        assert_eq!(mouse_up(&mut switch, 18.0, 10.0), None);
    }
}
