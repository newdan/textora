//! Button Widget — icon + optional text label.
//! Icon and text are both optional; whichever is set gets drawn.

use crate::core::text_layout::UiTextLayout;
use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, LayoutCtx, MouseButton, PaintCtx, Rect,
    Widget, WidgetAction,
};
use crate::widgets::icon::draw_icon;
use std::any::Any;
use std::sync::Arc;

const BUTTON_DISABLED_ALPHA: f32 = 0.45;

/// Visual style for a Button.
#[derive(Clone, Debug)]
pub struct ButtonStyle {
    pub font_size_logical: f32,
    pub pad_x_logical: f32,
    pub foreground: [f32; 4],
    pub selected_foreground: [f32; 4],
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub hover_background: [f32; 4],
    pub pressed_background: [f32; 4],
    pub selected_background: [f32; 4],
    pub disabled_foreground: [f32; 4],
    pub disabled_background: [f32; 4],
    pub corner_radius_logical: f32,
}

impl ButtonStyle {
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        let metrics = theme.control_metrics();
        let application = theme.application_theme();
        Self {
            font_size_logical: metrics.font_size_logical,
            pad_x_logical: metrics.horizontal_padding_logical,
            foreground: application.text_primary,
            selected_foreground: application.navigation_selected_text,
            background: application.control_surface,
            border: application.control_border,
            hover_background: application.hover_surface,
            pressed_background: application.selected_surface,
            selected_background: application.selected_surface,
            disabled_foreground: with_alpha(application.text_primary, BUTTON_DISABLED_ALPHA),
            disabled_background: with_alpha(application.control_surface, BUTTON_DISABLED_ALPHA),
            corner_radius_logical: metrics.corner_radius_logical,
        }
    }
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

pub struct Button {
    id: WidgetId,
    rect: Rect,
    icon: Option<String>,
    icon_size_logical: f32,
    text: Option<String>,
    accessibility_label: Option<String>,
    style: ButtonStyle,
    hovered: bool,
    enabled: bool,
    focused: bool,
    pressed: bool,
    selected: bool,
}

impl Button {
    pub fn new(id: WidgetId, style: ButtonStyle) -> Self {
        Self {
            id,
            rect: Rect::ZERO,
            icon: None,
            icon_size_logical: crate::constants::BUTTON_SIZE,
            text: None,
            accessibility_label: None,
            style,
            hovered: false,
            enabled: true,
            focused: false,
            pressed: false,
            selected: false,
        }
    }

    pub fn set_icon(&mut self, name: Option<String>) {
        self.icon = name;
    }
    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
    }
    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }
    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }
    pub fn set_active(&mut self, active: bool) {
        self.set_selected(active);
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.hovered = false;
            self.focused = false;
            self.pressed = false;
        }
    }
    pub fn set_icon_size(&mut self, sz: f32) {
        self.icon_size_logical = sz;
    }
    pub fn set_style(&mut self, s: ButtonStyle) {
        self.style = s;
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn background_color(&self) -> [f32; 4] {
        if !self.enabled {
            self.style.disabled_background
        } else if self.pressed {
            self.style.pressed_background
        } else if self.selected {
            self.style.selected_background
        } else if self.hovered {
            self.style.hover_background
        } else {
            self.style.background
        }
    }

    fn foreground_color(&self) -> [f32; 4] {
        if !self.enabled {
            self.style.disabled_foreground
        } else if self.selected {
            self.style.selected_foreground
        } else {
            self.style.foreground
        }
    }
}

impl Widget for Button {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        let metrics = ctx.theme.control_metrics();
        let alpha = ctx.global_alpha;
        let corner_radius = self.style.corner_radius_logical * dpi;
        let mut background = self.background_color();
        background[3] *= alpha;
        if background[3] > 0.0 {
            ctx.list.fill_rounded(self.rect, background, corner_radius);
        }

        let font_size = self.style.font_size_logical * dpi;
        let icon_size = self.icon_size_logical * dpi;
        let pad_x = self.style.pad_x_logical * dpi;
        let mut fg = self.foreground_color();
        fg[3] *= alpha;
        let mut border = self.style.border;
        border[3] *= alpha;
        if border[3] > 0.0 {
            ctx.list.stroke_rounded(self.rect, border, corner_radius, dpi);
        }
        if self.focused && self.enabled {
            let mut focus_ring = ctx.theme.settings_theme().focus_ring;
            focus_ring[3] *= alpha;
            ctx.list.stroke_rounded(
                self.rect,
                focus_ring,
                corner_radius,
                metrics.focus_ring_width_logical * dpi,
            );
        }

        let icon_gap = metrics.compact_spacing_logical * dpi;
        let mut cursor_x = self.rect.x + pad_x;

        if let Some(ref icon_name) = self.icon {
            let icon_y = self.rect.y + (self.rect.h - icon_size) * 0.5;
            draw_icon(ctx.list, icon_name, cursor_x, icon_y, icon_size, fg);
            cursor_x += icon_size + icon_gap;
        }

        if let Some(ref text) = self.text {
            let baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;
            if let Some(ref mut shaper) = ctx.shaper {
                let layout = UiTextLayout::new(
                    text,
                    font_size,
                    None,
                    shaping::Weight::NORMAL,
                    shaping::Style::Normal,
                    false,
                    shaper,
                );
                if let Some(layout) = layout {
                    let text_x = if self.icon.is_some() {
                        cursor_x
                    } else {
                        self.rect.x + (self.rect.w - layout.shaped.width) * 0.5
                    };
                    ctx.list.text_layout(Arc::new(layout), text_x, baseline, fg);
                }
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
            AccessibilityRole::Button,
            ctx.screen_bounds(self.rect),
        )
        .with_disabled(!self.enabled)
        .with_focused(self.focused)
        .with_selected(self.selected);
        if let Some(name) = self.accessibility_label.as_ref().or(self.text.as_ref()) {
            node = node.with_name(name.clone());
        }
        if self.enabled {
            node = node
                .with_action(AccessibilityAction::Focus)
                .with_action(AccessibilityAction::Activate);
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
            AccessibilityAction::Activate => {
                Some(WidgetAction::Control(ControlAction::Activated { id: self.id }))
            }
            _ => None,
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            Event::MouseMove { px, py } => {
                let inside = self.rect.contains(*px, *py);
                if inside {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                if inside != self.hovered {
                    self.hovered = inside;
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::PointerLeave => {
                std::mem::take(&mut self.hovered).then_some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if !self.enabled {
                    return None;
                }
                if self.rect.contains(*px, *py) {
                    self.pressed = true;
                    if self.focused {
                        Some(WidgetAction::Consumed)
                    } else {
                        Some(WidgetAction::Control(ControlAction::FocusRequested { id: self.id }))
                    }
                } else {
                    None
                }
            }
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                if !self.pressed {
                    return None;
                }

                self.pressed = false;
                let released_inside = self.rect.contains(*px, *py);
                self.hovered = released_inside;
                if released_inside && self.enabled {
                    Some(WidgetAction::Control(ControlAction::Activated { id: self.id }))
                } else {
                    Some(WidgetAction::Consumed)
                }
            }
            Event::InteractionCancel => {
                let interaction_changed =
                    std::mem::take(&mut self.hovered) | std::mem::take(&mut self.pressed);
                interaction_changed.then_some(WidgetAction::Consumed)
            }
            Event::KeyDown(key, modifiers)
                if self.enabled
                    && self.focused
                    && *modifiers == crate::core::Modifiers::NONE
                    && matches!(
                        key,
                        crate::core::KeyCode::Enter | crate::core::KeyCode::Char(' ')
                    ) =>
            {
                Some(WidgetAction::Control(ControlAction::Activated { id: self.id }))
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
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::widget::{ControlAction, KeyCode, LayoutCtx, Modifiers, WidgetId};

    fn test_style() -> ButtonStyle {
        ButtonStyle {
            font_size_logical: 14.0,
            pad_x_logical: 8.0,
            foreground: [0.9, 0.9, 0.9, 1.0],
            selected_foreground: [1.0, 1.0, 1.0, 1.0],
            background: [0.0, 0.0, 0.0, 0.0],
            border: [0.0, 0.0, 0.0, 0.0],
            hover_background: [0.2, 0.2, 0.2, 1.0],
            pressed_background: [0.25, 0.25, 0.25, 1.0],
            selected_background: [0.3, 0.3, 0.3, 1.0],
            disabled_foreground: [0.5, 0.5, 0.5, 1.0],
            disabled_background: [0.0, 0.0, 0.0, 0.0],
            corner_radius_logical: 4.0,
        }
    }

    fn make_button_with_rect(id: WidgetId, rect: Rect) -> Button {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut b = Button::new(id, test_style());
        b.set_rect(rect, &mut lc);
        b
    }

    fn make_button(id: WidgetId) -> Button {
        make_button_with_rect(id, Rect::new(0.0, 0.0, 100.0, 28.0))
    }

    fn mouse_down(button: &mut Button, px: f32, py: f32) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        button.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut event_ctx)
    }

    fn mouse_up(button: &mut Button, px: f32, py: f32) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        button.on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut event_ctx)
    }

    #[test]
    fn accessibility_exposes_button_semantics_and_reuses_control_actions() {
        let id = WidgetId(90);
        let mut button = make_button_with_rect(id, Rect::new(5.0, 6.0, 100.0, 28.0));
        button.set_text(Some("保存".into()));
        button.set_selected(true);
        button.set_keyboard_focus(Some(id));
        let context = crate::core::AccessibilityContext::new(10.0, 20.0);
        let node = button.accessibility_node(&context).expect("button should expose semantics");

        assert_eq!(node.id, crate::core::AccessibilityId::from(id));
        assert_eq!(node.role, crate::core::AccessibilityRole::Button);
        assert_eq!(node.name.as_deref(), Some("保存"));
        assert_eq!(node.bounds, Rect::new(15.0, 26.0, 100.0, 28.0));
        assert!(node.state.focused);
        assert_eq!(node.state.selected, Some(true));
        assert!(node.actions.contains(&crate::core::AccessibilityAction::Focus));
        assert!(node.actions.contains(&crate::core::AccessibilityAction::Activate));
        assert_eq!(
            button.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::Control(ControlAction::Activated { id }))
        );

        button.set_enabled(false);
        let disabled_node =
            button.accessibility_node(&context).expect("disabled button remains discoverable");
        assert!(disabled_node.state.disabled);
        assert!(disabled_node.actions.is_empty());
    }

    #[test]
    fn standard_style_uses_shared_metrics_and_semantic_theme_colors() {
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
            let metrics = theme.control_metrics();
            let application = theme.application_theme();
            let style = ButtonStyle::from_theme(&theme);

            assert_eq!(style.font_size_logical, metrics.font_size_logical);
            assert_eq!(style.pad_x_logical, metrics.horizontal_padding_logical);
            assert_eq!(style.corner_radius_logical, metrics.corner_radius_logical);
            assert_eq!(style.background, application.control_surface);
            assert_eq!(style.foreground, application.text_primary);
            assert_eq!(style.hover_background, application.hover_surface);
        }
    }

    #[test]
    fn paint_text_only_emits_text() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(1));
        b.set_text(Some("Hello".into()));
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        b.paint(&mut pc);
        let text_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).count();
        assert_eq!(text_count, 1);
    }

    #[test]
    fn paint_text_only_centers_text_in_button() {
        let theme = crate::theme::test_theme();
        let button_rect = Rect::new(0.0, 0.0, 100.0, 28.0);
        let mut button = make_button_with_rect(WidgetId(14), button_rect);
        button.set_text(Some("Hello".into()));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };

        button.paint(&mut paint_ctx);

        let DrawCmd::TextLayout { layout, x, .. } = draw_list
            .cmds
            .iter()
            .find(|command| matches!(command, DrawCmd::TextLayout { .. }))
            .expect("text-only button should emit a text layout")
        else {
            unreachable!("text layout was checked above");
        };
        let expected_x = (button_rect.w - layout.shaped.width) * 0.5;
        assert!((x - expected_x).abs() < 0.01, "text x {x} should be {expected_x}");
    }

    #[test]
    fn paint_icon_only_emits_triangle() {
        let theme = crate::theme::test_theme();
        let mut b = make_button_with_rect(WidgetId(2), Rect::new(0.0, 0.0, 32.0, 28.0));
        b.set_icon(Some("plus".into()));
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        b.paint(&mut pc);
        let tri_count =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillTriangle { .. })).count();
        assert!(tri_count > 0, "icon should emit fill triangles");
    }

    #[test]
    fn paint_hover_emits_bg() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(3));
        b.set_text(Some("X".into()));
        // Force hovered state
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        b.paint(&mut pc);
        let rect_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        assert!(rect_count >= 1, "hover should emit background fill rect");
    }

    #[test]
    fn paint_active_emits_bg() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(4));
        b.set_active(true);
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        b.paint(&mut pc);
        let rect_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        assert!(rect_count >= 1, "active should emit background fill rect");
    }

    #[test]
    fn button_activates_only_after_inside_press_and_release() {
        let mut button = make_button(WidgetId(7));

        assert_eq!(
            mouse_down(&mut button, 20.0, 10.0),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(7) }))
        );
        assert_eq!(
            mouse_up(&mut button, 20.0, 10.0),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(7) }))
        );
    }

    #[test]
    fn dragging_outside_cancels_button_activation() {
        let mut button = make_button(WidgetId(8));

        mouse_down(&mut button, 20.0, 10.0);
        assert_eq!(mouse_up(&mut button, 200.0, 200.0), Some(WidgetAction::Consumed));
    }

    #[test]
    fn hit_contains() {
        let b = make_button_with_rect(WidgetId(9), Rect::new(10.0, 10.0, 80.0, 20.0));
        assert!(b.hit(50.0, 20.0));
        assert!(!b.hit(5.0, 20.0));
    }

    #[test]
    fn mouse_move_updates_hovered() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(10));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let r = b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        assert!(r.is_some()); // Consumed on hover state change
        assert!(matches!(r.unwrap(), WidgetAction::Consumed));
    }

    #[test]
    fn paint_empty_button_emits_nothing() {
        let theme = crate::theme::test_theme();
        let b = make_button(WidgetId(11));
        // No icon, no text
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        b.paint(&mut pc);
        // Empty button with no hover should emit nothing
        assert_eq!(dl.cmds.len(), 0, "Empty button without hover should emit no draw commands");
    }

    #[test]
    fn paint_zero_rect_emits_nothing() {
        let theme = crate::theme::test_theme();
        let mut b = make_button_with_rect(WidgetId(12), Rect::ZERO);
        b.set_text(Some("Hello".into()));
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        b.paint(&mut pc);
        // Zero-size rect: bg fill has zero area, text is still emitted
        // This verifies no panic on zero rect
    }

    #[test]
    fn mousedown_right_button_no_action() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(13));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        let result = b.on_event(
            &Event::MouseDown { px: 50.0, py: 14.0, button: MouseButton::Right },
            &mut ec,
        );
        assert!(result.is_none(), "Right-click should not trigger button action");
    }

    #[test]
    fn mouseup_event_ignored() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(14));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let result =
            b.on_event(&Event::MouseUp { px: 50.0, py: 14.0, button: MouseButton::Left }, &mut ec);
        assert!(result.is_none(), "MouseUp should be ignored");
    }

    #[test]
    fn hover_exit_clears_hovered() {
        let theme = crate::theme::test_theme();
        let mut b = make_button(WidgetId(15));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        // Enter
        b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        assert!(b.hovered);
        // Exit
        b.on_event(&Event::MouseMove { px: 200.0, py: 200.0 }, &mut ec);
        assert!(!b.hovered);
    }

    #[test]
    fn inside_press_outside_release_does_not_activate() {
        let mut button = make_button(WidgetId(16));

        assert_eq!(
            mouse_down(&mut button, 50.0, 14.0),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(16) }))
        );
        assert_eq!(mouse_up(&mut button, 150.0, 140.0), Some(WidgetAction::Consumed));
    }

    #[test]
    fn enabled_button_is_collected_as_focusable() {
        let mut button = make_button(WidgetId(17));
        let mut focusable_ids = Vec::new();

        button.collect_focusable_ids(&mut focusable_ids);
        assert_eq!(focusable_ids, vec![WidgetId(17)]);

        button.set_enabled(false);
        focusable_ids.clear();
        button.collect_focusable_ids(&mut focusable_ids);
        assert!(focusable_ids.is_empty());
    }

    #[test]
    fn button_keyboard_activation_requires_focus() {
        let id = WidgetId(18);
        let mut button = make_button(id);
        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        for key in [KeyCode::Enter, KeyCode::Char(' ')] {
            assert_eq!(
                button.on_event(&Event::KeyDown(key, Modifiers::NONE), &mut event_ctx),
                None
            );
        }

        button.set_keyboard_focus(Some(id));
        for key in [KeyCode::Enter, KeyCode::Char(' ')] {
            assert_eq!(
                button.on_event(&Event::KeyDown(key, Modifiers::NONE), &mut event_ctx),
                Some(WidgetAction::Control(ControlAction::Activated { id }))
            );
        }
    }

    #[test]
    fn button_mouse_down_requests_focus_and_release_still_activates() {
        let id = WidgetId(19);
        let mut button = make_button(id);

        assert_eq!(
            mouse_down(&mut button, 20.0, 10.0),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id }))
        );
        assert_eq!(
            mouse_up(&mut button, 20.0, 10.0),
            Some(WidgetAction::Control(ControlAction::Activated { id }))
        );
    }

    #[test]
    fn disabling_button_clears_focus_and_pressed_state() {
        let id = WidgetId(20);
        let mut button = make_button(id);
        button.set_keyboard_focus(Some(id));
        assert_eq!(mouse_down(&mut button, 20.0, 10.0), Some(WidgetAction::Consumed));

        button.set_enabled(false);
        assert!(!button.is_capturing());
        button.set_enabled(true);

        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        assert_eq!(
            button.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx),
            None
        );
        assert_eq!(mouse_up(&mut button, 20.0, 10.0), None);
    }

    #[test]
    fn button_leave_preserves_press_and_cancel_is_idempotent() {
        let id = WidgetId(22);
        let mut button = make_button(id);
        button.set_keyboard_focus(Some(id));
        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(
            button.on_event(&Event::MouseMove { px: 20.0, py: 10.0 }, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(mouse_down(&mut button, 20.0, 10.0), Some(WidgetAction::Consumed));
        assert!(button.is_capturing());
        assert_eq!(
            button.on_event(&Event::PointerLeave, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!button.hovered);
        assert!(button.is_capturing());

        assert_eq!(
            button.on_event(&Event::InteractionCancel, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!button.is_capturing());
        assert_eq!(button.on_event(&Event::InteractionCancel, &mut event_ctx), None);
        assert_eq!(mouse_up(&mut button, 20.0, 10.0), None);
    }

    #[test]
    fn focused_button_paints_theme_focus_ring() {
        let id = WidgetId(21);
        let mut button = make_button(id);
        button.set_keyboard_focus(Some(id));
        let theme = crate::theme::test_theme();
        let focus_ring = theme.settings_theme().focus_ring;
        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx::new(&mut draw_list, &theme, 1.0);

        button.paint(&mut paint_ctx);

        assert!(draw_list.cmds.iter().any(
            |command| matches!(command, DrawCmd::StrokeRect { color, .. } if *color == focus_ring)
        ));
    }
}
