use crate::core::widget::WidgetId;
use crate::core::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode, Dock, Event, EventCtx,
    LayoutCtx, PaintCtx, Rect, Widget, WidgetAction,
};

use super::Button;

/// Routes pointer capture, hover and keyboard focus for buttons with parent supplied geometry.
pub struct ButtonCollection {
    buttons: Vec<Button>,
    rects: Vec<Rect>,
    pressed_index: Option<usize>,
    hovered_index: Option<usize>,
    focused_id: Option<WidgetId>,
}

impl ButtonCollection {
    pub fn new(buttons: Vec<Button>) -> Self {
        let rects = vec![Rect::ZERO; buttons.len()];
        Self { buttons, rects, pressed_index: None, hovered_index: None, focused_id: None }
    }

    pub fn buttons(&self) -> &[Button] {
        &self.buttons
    }

    pub fn buttons_mut(&mut self) -> &mut [Button] {
        &mut self.buttons
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn index_at(&self, px: f32, py: f32) -> Option<usize> {
        self.rects.iter().position(|rect| rect.w > 0.0 && rect.h > 0.0 && rect.contains(px, py))
    }

    pub fn set_rects(&mut self, rects: Vec<Rect>, ctx: &mut LayoutCtx) {
        assert_eq!(rects.len(), self.buttons.len(), "button count must match supplied geometry");
        self.rects = rects;
        for (button, rect) in self.buttons.iter_mut().zip(&self.rects) {
            button.set_rect(Rect::new(0.0, 0.0, rect.w, rect.h), ctx);
        }
    }

    pub fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        for button in &mut self.buttons {
            button.set_keyboard_focus(focused_id);
        }
    }

    pub fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        for (button, rect) in self.buttons.iter().zip(&self.rects) {
            if rect.w > 0.0 && rect.h > 0.0 {
                button.collect_focusable_ids(output);
            }
        }
    }

    pub fn collect_accessibility_nodes(
        &self,
        context: &AccessibilityContext,
        output: &mut Vec<AccessibilityNode>,
    ) {
        for (button, rect) in self.buttons.iter().zip(&self.rects) {
            if rect.w > 0.0 && rect.h > 0.0 {
                button.collect_accessibility_nodes(&context.offset_by(rect.x, rect.y), output);
            }
        }
    }

    pub fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        self.buttons.iter_mut().find_map(|button| button.on_accessibility_action(request))
    }

    pub fn paint(&self, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;
        for (button, rect) in self.buttons.iter().zip(&self.rects) {
            if rect.w <= 0.0 || rect.h <= 0.0 {
                continue;
            }
            ctx.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
            button.paint(ctx);
        }
        ctx.list.offset = saved_offset;
    }

    pub fn is_capturing(&self) -> bool {
        self.pressed_index.is_some() || self.buttons.iter().any(Widget::is_capturing)
    }

    fn dispatch(
        &mut self,
        index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let rect = *self.rects.get(index)?;
        let local_event = Dock::to_local(event, rect.x, rect.y);
        self.buttons.get_mut(index)?.on_event(local_event.as_ref(), ctx)
    }

    fn dispatch_lifecycle(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let hover_changed = self.hovered_index.take().is_some();
        let changed = if matches!(event, Event::InteractionCancel) {
            self.pressed_index.take().is_some() | hover_changed
        } else {
            hover_changed
        };
        let mut first_action = None;
        for index in 0..self.buttons.len() {
            if let Some(action) = self.dispatch(index, event, ctx)
                && first_action.is_none()
            {
                first_action = Some(action);
            }
        }
        first_action.or_else(|| changed.then_some(WidgetAction::Consumed))
    }

    fn dispatch_move(
        &mut self,
        event: &Event,
        px: f32,
        py: f32,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        if let Some(index) = self.pressed_index {
            return self.dispatch(index, event, ctx);
        }
        let next_index = self.index_at(px, py);
        let previous_action = if self.hovered_index != next_index {
            self.hovered_index.and_then(|index| self.dispatch(index, event, ctx))
        } else {
            None
        };
        self.hovered_index = next_index;
        next_index.and_then(|index| self.dispatch(index, event, ctx)).or(previous_action)
    }

    pub fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match event {
            Event::PointerLeave | Event::InteractionCancel => self.dispatch_lifecycle(event, ctx),
            Event::MouseMove { px, py } => self.dispatch_move(event, *px, *py, ctx),
            Event::MouseDown { px, py, .. } => {
                let index = self.index_at(*px, *py)?;
                let action = self.dispatch(index, event, ctx);
                if self.buttons[index].is_capturing() {
                    self.pressed_index = Some(index);
                }
                action
            }
            Event::MouseUp { .. } => {
                let index = self.pressed_index.take()?;
                self.dispatch(index, event, ctx)
            }
            Event::KeyDown(..) => {
                let focused_id = self.focused_id?;
                let index =
                    self.buttons.iter().position(|button| button.id() == Some(focused_id))?;
                self.dispatch(index, event, ctx)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::NoopMeasure;
    use crate::core::widget::ControlAction;
    use crate::core::{KeyCode, Modifiers, MouseButton};
    use crate::widgets::button::ButtonStyle;

    #[test]
    fn captured_press_activates_only_after_inside_release() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut buttons =
            ButtonCollection::new(vec![Button::new(WidgetId(7), ButtonStyle::from_theme(&theme))]);
        buttons.set_rects(vec![Rect::new(10.0, 10.0, 30.0, 20.0)], &mut layout);
        let mut events = EventCtx::new(&theme, 1.0);

        let press = Event::MouseDown { px: 20.0, py: 20.0, button: MouseButton::Left };
        assert_eq!(
            buttons.on_event(&press, &mut events),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(7) }))
        );
        assert!(buttons.is_capturing());
        assert_eq!(
            buttons.on_event(
                &Event::MouseUp { px: 80.0, py: 20.0, button: MouseButton::Left },
                &mut events
            ),
            Some(WidgetAction::Consumed)
        );
        assert!(!buttons.is_capturing());

        let _ = buttons.on_event(&press, &mut events);
        assert_eq!(
            buttons.on_event(
                &Event::MouseUp { px: 20.0, py: 20.0, button: MouseButton::Left },
                &mut events
            ),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(7) }))
        );
    }

    #[test]
    fn focused_button_receives_keyboard_activation() {
        let theme = crate::theme::test_theme();
        let mut buttons =
            ButtonCollection::new(vec![Button::new(WidgetId(7), ButtonStyle::from_theme(&theme))]);
        buttons.set_keyboard_focus(Some(WidgetId(7)));
        let mut events = EventCtx::new(&theme, 1.0);
        assert_eq!(
            buttons.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut events),
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(7) }))
        );
    }

    #[test]
    fn pointer_leave_keeps_press_capture_until_release() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut buttons =
            ButtonCollection::new(vec![Button::new(WidgetId(7), ButtonStyle::from_theme(&theme))]);
        buttons.set_rects(vec![Rect::new(10.0, 10.0, 30.0, 20.0)], &mut layout);
        let mut events = EventCtx::new(&theme, 1.0);

        let _ = buttons.on_event(
            &Event::MouseDown { px: 20.0, py: 20.0, button: MouseButton::Left },
            &mut events,
        );
        let _ = buttons.on_event(&Event::PointerLeave, &mut events);
        assert!(buttons.is_capturing());
        assert_eq!(
            buttons.on_event(
                &Event::MouseUp { px: 80.0, py: 20.0, button: MouseButton::Left },
                &mut events,
            ),
            Some(WidgetAction::Consumed),
        );
        assert!(!buttons.is_capturing());
    }
}
