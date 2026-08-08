//! 画布覆盖式双向滚动条。

use std::any::Any;

use crate::canvas::CanvasAxis;
use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use crate::widgets::scrollbar::{
    SCROLLBAR_RESERVE_PX, ScrollbarAction, ScrollbarInput, ScrollbarWidget,
};

/// 画布滚动条每帧输入；`None` 表示对应方向无需显示。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasScrollbarsInput {
    pub horizontal: Option<ScrollbarInput>,
    pub vertical: Option<ScrollbarInput>,
}

/// 画布滚动条发出的动作，携带产生动作的轴。
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasScrollbarsAction {
    pub axis: CanvasAxis,
    pub action: ScrollbarAction,
}

/// 双向滚动条的局部布局。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasScrollbarsLayout {
    pub horizontal: Rect,
    pub vertical: Rect,
}

/// 在画布右侧和底部布局按需显示的滚动条。
pub fn layout_scrollbars(
    canvas_rect: Rect,
    has_horizontal: bool,
    has_vertical: bool,
    scrollbar_thickness: f32,
) -> CanvasScrollbarsLayout {
    let thickness = scrollbar_thickness.max(0.0);
    let horizontal_height = thickness.min(canvas_rect.h.max(0.0));
    let vertical_width = thickness.min(canvas_rect.w.max(0.0));
    let horizontal_reserved_width = if has_vertical { vertical_width } else { 0.0 };
    let vertical_reserved_height = if has_horizontal { horizontal_height } else { 0.0 };
    let horizontal = if has_horizontal {
        Rect::new(
            canvas_rect.x,
            canvas_rect.bottom() - horizontal_height,
            (canvas_rect.w - horizontal_reserved_width).max(0.0),
            horizontal_height,
        )
    } else {
        Rect::ZERO
    };
    let vertical = if has_vertical {
        Rect::new(
            canvas_rect.right() - vertical_width,
            canvas_rect.y,
            vertical_width,
            (canvas_rect.h - vertical_reserved_height).max(0.0),
        )
    } else {
        Rect::ZERO
    };

    CanvasScrollbarsLayout { horizontal, vertical }
}

/// 组合横向与纵向滚动条的画布覆盖组件。
pub struct CanvasScrollbarsWidget {
    rect: Rect,
    layout: CanvasScrollbarsLayout,
    input: CanvasScrollbarsInput,
    horizontal: ScrollbarWidget,
    vertical: ScrollbarWidget,
}

impl Default for CanvasScrollbarsWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl CanvasScrollbarsWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            layout: CanvasScrollbarsLayout::default(),
            input: CanvasScrollbarsInput::default(),
            horizontal: ScrollbarWidget::horizontal(),
            vertical: ScrollbarWidget::vertical(),
        }
    }

    /// 注入本帧滚动条输入。
    pub fn set_input(&mut self, input: CanvasScrollbarsInput) {
        self.input = input;
        if let Some(horizontal) = input.horizontal {
            self.horizontal.set_input(horizontal);
        }
        if let Some(vertical) = input.vertical {
            self.vertical.set_input(vertical);
        }
    }

    fn refresh_layout(&mut self, ctx: &mut LayoutCtx) {
        let scrollbar_thickness = SCROLLBAR_RESERVE_PX * ctx.dpi;
        self.layout = layout_scrollbars(
            self.rect,
            self.input.horizontal.is_some(),
            self.input.vertical.is_some(),
            scrollbar_thickness,
        );
        self.horizontal.set_rect(self.layout.horizontal, ctx);
        self.vertical.set_rect(self.layout.vertical, ctx);
    }

    fn to_local_event(event: &Event, offset: Rect) -> Event {
        match event {
            Event::MouseMove { px, py } => {
                Event::MouseMove { px: px - offset.x, py: py - offset.y }
            }
            Event::MouseDown { px, py, button } => {
                Event::MouseDown { px: px - offset.x, py: py - offset.y, button: *button }
            }
            Event::MouseUp { px, py, button } => {
                Event::MouseUp { px: px - offset.x, py: py - offset.y, button: *button }
            }
            Event::Wheel { dx, dy, px, py } => {
                Event::Wheel { dx: *dx, dy: *dy, px: px - offset.x, py: py - offset.y }
            }
            other => other.clone(),
        }
    }

    fn forward_event(
        scrollbar: &mut ScrollbarWidget,
        axis: CanvasAxis,
        layout: Rect,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let local_event = Self::to_local_event(event, layout);
        match scrollbar.on_event(&local_event, ctx) {
            Some(WidgetAction::Scrollbar(action)) => {
                Some(WidgetAction::CanvasScrollbars(CanvasScrollbarsAction { axis, action }))
            }
            Some(WidgetAction::Consumed) => Some(WidgetAction::Consumed),
            Some(_) | None => None,
        }
    }

    fn paint_scrollbar(scrollbar: &ScrollbarWidget, layout: Rect, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;
        ctx.list.offset = (saved_offset.0 + layout.x, saved_offset.1 + layout.y);
        scrollbar.paint(ctx);
        ctx.list.offset = saved_offset;
    }

    fn forwards_to_horizontal(&self) -> bool {
        self.input.horizontal.is_some() || self.horizontal.is_capturing()
    }

    fn forwards_to_vertical(&self) -> bool {
        self.input.vertical.is_some() || self.vertical.is_capturing()
    }
}

impl Widget for CanvasScrollbarsWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        self.refresh_layout(ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.input.horizontal.is_some() {
            Self::paint_scrollbar(&self.horizontal, self.layout.horizontal, ctx);
        }
        if self.input.vertical.is_some() {
            Self::paint_scrollbar(&self.vertical, self.layout.vertical, ctx);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        (self.input.horizontal.is_some()
            && self.horizontal.hit(px - self.layout.horizontal.x, py - self.layout.horizontal.y))
            || (self.input.vertical.is_some()
                && self.vertical.hit(px - self.layout.vertical.x, py - self.layout.vertical.y))
    }

    fn is_capturing(&self) -> bool {
        self.horizontal.is_capturing() || self.vertical.is_capturing()
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let forwards_to_horizontal = self.forwards_to_horizontal();
        let forwards_to_vertical = self.forwards_to_vertical();

        if matches!(event, Event::PointerLeave | Event::InteractionCancel) {
            let horizontal_action = forwards_to_horizontal.then(|| {
                Self::forward_event(
                    &mut self.horizontal,
                    CanvasAxis::Horizontal,
                    self.layout.horizontal,
                    event,
                    ctx,
                )
            });
            let vertical_action = forwards_to_vertical.then(|| {
                Self::forward_event(
                    &mut self.vertical,
                    CanvasAxis::Vertical,
                    self.layout.vertical,
                    event,
                    ctx,
                )
            });
            return horizontal_action.flatten().or_else(|| vertical_action.flatten());
        }

        if matches!(event, Event::MouseMove { .. }) {
            let horizontal_action = forwards_to_horizontal.then(|| {
                Self::forward_event(
                    &mut self.horizontal,
                    CanvasAxis::Horizontal,
                    self.layout.horizontal,
                    event,
                    ctx,
                )
            });
            let vertical_action = forwards_to_vertical.then(|| {
                Self::forward_event(
                    &mut self.vertical,
                    CanvasAxis::Vertical,
                    self.layout.vertical,
                    event,
                    ctx,
                )
            });
            return horizontal_action.flatten().or_else(|| vertical_action.flatten());
        }

        if forwards_to_horizontal
            && let Some(action) = Self::forward_event(
                &mut self.horizontal,
                CanvasAxis::Horizontal,
                self.layout.horizontal,
                event,
                ctx,
            )
        {
            return Some(action);
        }
        if forwards_to_vertical {
            return Self::forward_event(
                &mut self.vertical,
                CanvasAxis::Vertical,
                self.layout.vertical,
                event,
                ctx,
            );
        }
        None
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
    use crate::canvas::CanvasAxis;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::{Event, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction};
    use crate::theme::test_theme;
    use crate::widgets::scrollbar::{ScrollbarAction, ScrollbarInput};

    const SCROLLBAR_THICKNESS_PX: f32 = 14.0;
    fn overflowing_input() -> ScrollbarInput {
        ScrollbarInput {
            viewport_height_px: 100.0,
            total_display_rows: 1_000,
            scroll_top_rows: 0.0,
        }
    }

    fn canvas_rect() -> Rect {
        Rect::new(0.0, 0.0, 800.0, 600.0)
    }

    fn paint_fill_count(widget: &CanvasScrollbarsWidget, theme: &crate::Theme) -> usize {
        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx::new(&mut draw_list, theme, 1.0);
        widget.paint(&mut paint_ctx);
        draw_list.cmds.iter().filter(|command| matches!(command, DrawCmd::FillRect { .. })).count()
    }

    #[test]
    fn two_axes_reserve_bottom_right_intersection() {
        let layout = layout_scrollbars(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            true,
            true,
            SCROLLBAR_THICKNESS_PX,
        );

        assert_eq!(layout.horizontal.w, 786.0);
        assert_eq!(layout.vertical.h, 586.0);
    }

    #[test]
    fn scrollbars_remain_inside_canvas_when_canvas_is_smaller_than_thickness() {
        let horizontal =
            layout_scrollbars(Rect::new(0.0, 0.0, 800.0, 5.0), true, false, SCROLLBAR_THICKNESS_PX)
                .horizontal;
        let vertical =
            layout_scrollbars(Rect::new(0.0, 0.0, 5.0, 600.0), false, true, SCROLLBAR_THICKNESS_PX)
                .vertical;

        assert_eq!(horizontal.y, 0.0);
        assert_eq!(horizontal.h, 5.0);
        assert_eq!(vertical.x, 0.0);
        assert_eq!(vertical.w, 5.0);
    }

    #[test]
    fn only_horizontal_scrollbar_omits_vertical_hit_area_and_paint() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let mut widget = CanvasScrollbarsWidget::new();
        widget.set_input(CanvasScrollbarsInput {
            horizontal: Some(overflowing_input()),
            vertical: None,
        });
        let canvas_rect = canvas_rect();
        widget.set_rect(canvas_rect, &mut layout_ctx);

        assert!(widget.hit(100.0, canvas_rect.bottom() - 1.0));
        assert!(!widget.hit(canvas_rect.right() - 1.0, 100.0));

        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx::new(&mut draw_list, &theme, 1.0);
        widget.paint(&mut paint_ctx);

        assert!(!draw_list.cmds.is_empty());
        assert!(draw_list.cmds.iter().all(|command| match command {
            DrawCmd::FillRect { rect, .. } =>
                rect.y >= canvas_rect.bottom() - SCROLLBAR_THICKNESS_PX,
            _ => true,
        }));
    }

    #[test]
    fn horizontal_drag_emits_axis_action_and_captures_pointer() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let mut event_ctx = crate::core::EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let mut widget = CanvasScrollbarsWidget::new();
        widget.set_input(CanvasScrollbarsInput {
            horizontal: Some(overflowing_input()),
            vertical: None,
        });
        widget.set_rect(canvas_rect(), &mut layout_ctx);

        let action = widget.on_event(
            &Event::MouseDown { px: 40.0, py: 590.0, button: MouseButton::Left },
            &mut event_ctx,
        );

        assert_eq!(
            action,
            Some(WidgetAction::CanvasScrollbars(CanvasScrollbarsAction {
                axis: CanvasAxis::Horizontal,
                action: ScrollbarAction::StartDrag,
            }))
        );
        assert!(widget.is_capturing());

        let action = widget.on_event(
            &Event::MouseUp { px: 1_200.0, py: 1_200.0, button: MouseButton::Left },
            &mut event_ctx,
        );
        assert_eq!(
            action,
            Some(WidgetAction::CanvasScrollbars(CanvasScrollbarsAction {
                axis: CanvasAxis::Horizontal,
                action: ScrollbarAction::EndDrag,
            }))
        );
        assert!(!widget.is_capturing());
    }

    #[test]
    fn lifecycle_events_reach_both_axes_and_cancel_capture_once() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let mut event_ctx = crate::core::EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let mut widget = CanvasScrollbarsWidget::new();
        widget.set_input(CanvasScrollbarsInput {
            horizontal: Some(overflowing_input()),
            vertical: Some(overflowing_input()),
        });
        widget.set_rect(canvas_rect(), &mut layout_ctx);

        let _ = widget.on_event(&Event::MouseMove { px: 795.0, py: 40.0 }, &mut event_ctx);
        let _ = widget.on_event(
            &Event::MouseDown { px: 40.0, py: 590.0, button: MouseButton::Left },
            &mut event_ctx,
        );
        assert!(widget.horizontal.is_capturing());
        assert_eq!(paint_fill_count(&widget, &theme), 4);

        assert_eq!(
            widget.on_event(&Event::PointerLeave, &mut event_ctx),
            Some(WidgetAction::CanvasScrollbars(CanvasScrollbarsAction {
                axis: CanvasAxis::Vertical,
                action: ScrollbarAction::HoverChanged(false),
            }))
        );
        assert!(widget.horizontal.is_capturing());
        assert_eq!(paint_fill_count(&widget, &theme), 3);
        assert_eq!(
            widget.on_event(&Event::InteractionCancel, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.is_capturing());
        assert_eq!(paint_fill_count(&widget, &theme), 2);
        assert_eq!(widget.on_event(&Event::InteractionCancel, &mut event_ctx), None);
    }
}
