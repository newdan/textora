//! SplitterWidget — 只报告逻辑位置变化的通用拖动分隔条。

use std::any::Any;

use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};

/// 分隔条可拖动的逻辑位置输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterInput {
    pub logical_position: f32,
    pub minimum_logical_position: f32,
    pub maximum_logical_position: f32,
    pub enabled: bool,
}

impl Default for SplitterInput {
    fn default() -> Self {
        Self {
            logical_position: 0.0,
            minimum_logical_position: 0.0,
            maximum_logical_position: 0.0,
            enabled: true,
        }
    }
}

/// 分隔条生命周期与位置变化。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplitterAction {
    DragStarted,
    LogicalPositionChanged(f32),
    DragEnded(f32),
}

/// 键盘调整分隔条时的逻辑像素步长。
pub const SPLITTER_KEYBOARD_STEP_LOGICAL: f32 = 8.0;

/// 纵向分隔条组件，位置值始终以逻辑像素报告。
pub struct SplitterWidget {
    rect: Rect,
    input: SplitterInput,
    dpi: f32,
    hovered: bool,
    drag_start_px: Option<f32>,
    drag_start_logical_position: f32,
    current_logical_position: f32,
}

impl Default for SplitterWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SplitterWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            input: SplitterInput::default(),
            dpi: 1.0,
            hovered: false,
            drag_start_px: None,
            drag_start_logical_position: 0.0,
            current_logical_position: 0.0,
        }
    }

    pub fn set_input(&mut self, input: SplitterInput) {
        self.input = input;
        if !self.is_capturing() {
            self.current_logical_position = self.clamp_logical_position(input.logical_position);
        }
        if !input.enabled {
            self.hovered = false;
            self.drag_start_px = None;
        }
    }

    pub fn logical_position(&self) -> f32 {
        self.current_logical_position
    }

    fn clamp_logical_position(&self, logical_position: f32) -> f32 {
        logical_position.clamp(
            self.input.minimum_logical_position,
            self.input.maximum_logical_position.max(self.input.minimum_logical_position),
        )
    }

    fn update_drag_position(&mut self, pointer_x: f32) -> Option<SplitterAction> {
        let drag_start_px = self.drag_start_px?;
        let delta_logical = (pointer_x - drag_start_px) / self.dpi;
        let next_logical_position =
            self.clamp_logical_position(self.drag_start_logical_position + delta_logical);
        if (next_logical_position - self.current_logical_position).abs() <= f32::EPSILON {
            return None;
        }
        self.current_logical_position = next_logical_position;
        Some(SplitterAction::LogicalPositionChanged(next_logical_position))
    }

    fn adjust_with_keyboard(&mut self, delta_logical: f32) -> Option<SplitterAction> {
        let next_logical_position =
            self.clamp_logical_position(self.current_logical_position + delta_logical);
        if (next_logical_position - self.current_logical_position).abs() <= f32::EPSILON {
            return None;
        }
        self.current_logical_position = next_logical_position;
        Some(SplitterAction::LogicalPositionChanged(next_logical_position))
    }
}

impl Widget for SplitterWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.dpi = ctx.dpi;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let color = if self.is_capturing() || self.hovered {
            ctx.theme.palette.accent
        } else {
            ctx.theme.palette.border_subtle
        };
        let thickness = if self.is_capturing() || self.hovered { 2.0 * ctx.dpi } else { ctx.dpi };
        let x = self.rect.x + (self.rect.w - thickness) * 0.5;
        ctx.list.fill(Rect::new(x, self.rect.y, thickness, self.rect.h), color);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.input.enabled {
            return None;
        }

        if let Event::MouseMove { px, py } = event
            && !self.is_capturing()
        {
            let was_hovered = self.hovered;
            self.hovered = self.hit(*px, *py);
            if self.hovered || was_hovered {
                ctx.cursor_hint = Some(if self.hovered {
                    winit::window::CursorIcon::EwResize
                } else {
                    winit::window::CursorIcon::Default
                });
                return Some(WidgetAction::Consumed);
            }
            return None;
        }

        let action = match event {
            Event::MouseMove { px, .. } => self.update_drag_position(*px),
            Event::MouseDown { px, py, button: MouseButton::Left } if self.hit(*px, *py) => {
                self.drag_start_px = Some(*px);
                self.drag_start_logical_position = self.current_logical_position;
                Some(SplitterAction::DragStarted)
            }
            Event::MouseUp { px, button: MouseButton::Left, .. } if self.is_capturing() => {
                let _ = self.update_drag_position(*px);
                self.drag_start_px = None;
                Some(SplitterAction::DragEnded(self.current_logical_position))
            }
            Event::KeyDown(KeyCode::Left, _) => {
                self.adjust_with_keyboard(-SPLITTER_KEYBOARD_STEP_LOGICAL)
            }
            Event::KeyDown(KeyCode::Right, _) => {
                self.adjust_with_keyboard(SPLITTER_KEYBOARD_STEP_LOGICAL)
            }
            Event::KeyDown(KeyCode::Home, _) => self.adjust_with_keyboard(
                self.input.minimum_logical_position - self.current_logical_position,
            ),
            Event::KeyDown(KeyCode::End, _) => self.adjust_with_keyboard(
                self.input.maximum_logical_position - self.current_logical_position,
            ),
            _ => None,
        }?;
        if self.is_capturing() || self.hovered {
            ctx.cursor_hint = Some(winit::window::CursorIcon::EwResize);
        }
        Some(WidgetAction::Splitter(action))
    }

    fn is_capturing(&self) -> bool {
        self.drag_start_px.is_some()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{EventCtx, LayoutCtx, Modifiers, NoopMeasure};

    fn layout(widget: &mut SplitterWidget, dpi: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        widget.set_rect(Rect::new(100.0, 0.0, 8.0, 500.0), &mut context);
    }

    fn widget() -> SplitterWidget {
        let mut widget = SplitterWidget::new();
        widget.set_input(SplitterInput {
            logical_position: 200.0,
            minimum_logical_position: 180.0,
            maximum_logical_position: 320.0,
            enabled: true,
        });
        layout(&mut widget, 2.0);
        widget
    }

    fn event_context(theme: &crate::Theme) -> EventCtx<'_> {
        EventCtx { theme, dpi: 2.0, cursor_hint: None }
    }

    #[test]
    fn creates_a_splitter() {
        assert_eq!(SplitterWidget::new().logical_position(), 0.0);
    }

    #[test]
    fn drag_reports_dpi_adjusted_logical_position_and_captures_pointer() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 102.0, py: 20.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Splitter(SplitterAction::DragStarted))
        );
        assert!(widget.is_capturing());
        assert_eq!(
            widget.on_event(&Event::MouseMove { px: 152.0, py: 700.0 }, &mut context),
            Some(WidgetAction::Splitter(SplitterAction::LogicalPositionChanged(225.0)))
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseUp { px: 152.0, py: 700.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Splitter(SplitterAction::DragEnded(225.0)))
        );
        assert!(!widget.is_capturing());
    }

    #[test]
    fn position_is_clamped_and_keyboard_adjustable() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Home, Modifiers::NONE), &mut context),
            Some(WidgetAction::Splitter(SplitterAction::LogicalPositionChanged(180.0)))
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Left, Modifiers::NONE), &mut context),
            None
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::End, Modifiers::NONE), &mut context),
            Some(WidgetAction::Splitter(SplitterAction::LogicalPositionChanged(320.0)))
        );
    }

    #[test]
    fn hover_consumes_pointer_input_without_reporting_a_position_change() {
        let mut widget = widget();
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::MouseMove { px: 102.0, py: 20.0 }, &mut context),
            Some(WidgetAction::Consumed)
        );
    }
}
