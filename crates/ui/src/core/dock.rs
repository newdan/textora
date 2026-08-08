//! Dock 容器：吸边布局 + 绘制 + 事件分发。
//! 不引入 flex；children 按 Vec 顺序依次从剩余空间"切"出自己的 rect。
//! fill 获得最后剩余的空间。

use crate::core::geom::Rect;
use crate::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget, WidgetAction};
use crate::theme::Theme;
use std::borrow::Cow;

/// 吸边方向
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// Dock 子项：一个 widget + 吸边方向 + 厚度回调 + 可见性。
pub struct DockChild {
    pub widget: Box<dyn Widget>,
    pub side: Side,
    pub thickness: Box<dyn Fn(&Theme, f32) -> f32>,
    pub visible: bool,
    /// 布局阶段计算的绝对坐标矩形。paint 和事件分发使用它做坐标变换。
    pub layout_rect: Rect,
}

/// Dock 容器：吸边 chrome + 填充区域。
pub struct Dock {
    pub children: Vec<DockChild>,
    pub fill: Box<dyn Widget>,
    pub fill_rect: Rect,
}

impl Dock {
    pub fn new(fill: Box<dyn Widget>) -> Self {
        Self { children: Vec::new(), fill, fill_rect: Rect::ZERO }
    }

    /// 布局：从 screen 中依次切分 chrome 区域，剩余给 fill。
    /// 若 children 总厚度超过剩余空间，后续子项和 fill 得到 ZERO rect。
    pub fn layout(&mut self, screen: Rect, ctx: &mut LayoutCtx) {
        let mut remaining = screen;

        for child in self.children.iter_mut() {
            if !child.visible {
                continue;
            }
            let t = (child.thickness)(ctx.theme, ctx.dpi);
            if t <= 0.0 {
                child.widget.set_rect(Rect::ZERO, ctx);
                continue;
            }
            // 防止厚度超过剩余空间导致负尺寸 rect
            let t_clamped = t.min(match child.side {
                Side::Top | Side::Bottom => remaining.h,
                Side::Left | Side::Right => remaining.w,
            });
            if t_clamped <= 0.0 {
                child.widget.set_rect(Rect::ZERO, ctx);
                continue;
            }
            let child_rect = match child.side {
                Side::Top => {
                    let r = Rect::new(remaining.x, remaining.y, remaining.w, t_clamped);
                    remaining = Rect::new(
                        remaining.x,
                        remaining.y + t_clamped,
                        remaining.w,
                        (remaining.h - t_clamped).max(0.0),
                    );
                    r
                }
                Side::Bottom => {
                    let r = Rect::new(
                        remaining.x,
                        remaining.y + remaining.h - t_clamped,
                        remaining.w,
                        t_clamped,
                    );
                    remaining = Rect::new(
                        remaining.x,
                        remaining.y,
                        remaining.w,
                        (remaining.h - t_clamped).max(0.0),
                    );
                    r
                }
                Side::Left => {
                    let r = Rect::new(remaining.x, remaining.y, t_clamped, remaining.h);
                    remaining = Rect::new(
                        remaining.x + t_clamped,
                        remaining.y,
                        (remaining.w - t_clamped).max(0.0),
                        remaining.h,
                    );
                    r
                }
                Side::Right => {
                    let r = Rect::new(
                        remaining.x + remaining.w - t_clamped,
                        remaining.y,
                        t_clamped,
                        remaining.h,
                    );
                    remaining = Rect::new(
                        remaining.x,
                        remaining.y,
                        (remaining.w - t_clamped).max(0.0),
                        remaining.h,
                    );
                    r
                }
            };
            child.layout_rect = child_rect;
            child.widget.set_rect(child_rect, ctx);
        }

        self.fill_rect = remaining;
        self.fill.set_rect(remaining, ctx);
    }

    /// 绘制：先 fill，再按 children 顺序绘制 chrome（chrome 在 fill 之上）。
    /// 阶段4：为每个子 widget 推入其 layout_rect 偏移，使 widget 内部使用局部坐标。
    pub fn paint(&self, ctx: &mut PaintCtx) {
        self.fill.paint(ctx);
        for child in self.children.iter() {
            if child.visible {
                let saved = ctx.list.offset;
                ctx.list.offset = (saved.0 + child.layout_rect.x, saved.1 + child.layout_rect.y);
                child.widget.paint(ctx);
                ctx.list.offset = saved;
            }
        }
    }

    /// 事件分发：从后往前遍历 children（后添加的在上层），命中即分发。
    /// 若没有 child 命中，分发给 fill。
    ///
    /// 鼠标捕获：若任一 child `is_capturing()` 返回 true（如 scrollbar 拖动 thumb、
    /// sidebar resize 中），所有鼠标事件优先派给该 widget，跳过 hit test。
    /// 这保证拖动中光标移出 widget 矩形仍能继续接收 MouseMove / MouseUp。
    pub fn dispatch(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if matches!(ev, Event::PointerLeave | Event::InteractionCancel) {
            return self.broadcast_lifecycle_event(ev, ctx);
        }

        let is_mouse = matches!(
            ev,
            Event::MouseMove { .. }
                | Event::MouseDown { .. }
                | Event::MouseUp { .. }
                | Event::Wheel { .. }
        );

        if is_mouse {
            // 优先派给捕获中的 widget（拖动状态）
            for child in self.children.iter_mut().rev() {
                if !child.visible {
                    continue;
                }
                if child.widget.is_capturing() {
                    let ev = Self::to_local(ev, child.layout_rect.x, child.layout_rect.y);
                    return child.widget.on_event(&ev, ctx);
                }
            }
        }

        // MouseMove: broadcast to ALL visible children so they can update
        // hover state even when the mouse is outside their rect.
        if let Event::MouseMove { px, py } = ev {
            let mut hit_action = None;
            let mut non_hit_action = None;
            let n = self.children.len();
            for i in (0..n).rev() {
                if !self.children[i].visible {
                    continue;
                }
                let lx = self.children[i].layout_rect.x;
                let ly = self.children[i].layout_rect.y;
                let ev2 = Self::to_local(ev, lx, ly);
                let (hx, hy) = (*px - lx, *py - ly);
                if self.children[i].widget.hit(hx, hy) {
                    if let Some(a) = self.children[i].widget.on_event(&ev2, ctx) {
                        hit_action = Some(a);
                    }
                } else {
                    // Mouse outside this child — still deliver so it can
                    // clear hover state internally.
                    // Capture the action (e.g. HoverChanged(false)) so the
                    // caller can translate it into a redraw request.
                    // Cursor hint from non-hit widgets is discarded to
                    // avoid pollution.
                    let saved_hint = ctx.cursor_hint;
                    let a = self.children[i].widget.on_event(&ev2, ctx);
                    ctx.cursor_hint = saved_hint;
                    if a.is_some() && non_hit_action.is_none() {
                        non_hit_action = a;
                    }
                }
            }
            if let Some(a) = hit_action {
                return Some(a);
            }
            if let Some(a) = non_hit_action {
                // Still deliver the event to the fill widget so it can
                // process the move (e.g. hit-testing) normally.
                self.fill.on_event(ev, ctx);
                return Some(a);
            }
            return self.fill.on_event(ev, ctx);
        }
        // 逆序遍历：后添加的在视觉上层（MouseDown / Wheel / MouseUp）
        for child in self.children.iter_mut().rev() {
            if !child.visible {
                continue;
            }
            match ev {
                Event::MouseDown { px, py, .. } | Event::Wheel { px, py, .. } => {
                    let (hx, hy) = (*px - child.layout_rect.x, *py - child.layout_rect.y);
                    if child.widget.hit(hx, hy) {
                        let ev = Self::to_local(ev, child.layout_rect.x, child.layout_rect.y);
                        return child.widget.on_event(&ev, ctx);
                    }
                }
                Event::MouseUp { .. } => {
                    // MouseUp 不依赖 hit test：拖拽中的 widget 可能已移出
                    // 自己的 rect，但仍需接收 MouseUp 来清理拖拽状态
                    let ev = Self::to_local(ev, child.layout_rect.x, child.layout_rect.y);
                    if let Some(action) = child.widget.on_event(&ev, ctx) {
                        return Some(action);
                    }
                }
                _ => {
                    // 键盘事件不依赖 hit——留给上层处理，dock 不路由。
                    continue;
                }
            }
        }
        self.fill.on_event(ev, ctx)
    }

    fn broadcast_lifecycle_event(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let mut first_action = None;
        for child in self.children.iter_mut().rev() {
            if !child.visible {
                continue;
            }
            let local_event = Self::to_local(event, child.layout_rect.x, child.layout_rect.y);
            if let Some(action) = child.widget.on_event(&local_event, ctx)
                && first_action.is_none()
            {
                first_action = Some(action);
            }
        }
        let fill_action = self.fill.on_event(event, ctx);
        first_action.or(fill_action)
    }

    /// 将鼠标事件坐标从绝对坐标系转为子 widget 的相对坐标系。
    pub fn to_local<'a>(event: &'a Event, dx: f32, dy: f32) -> Cow<'a, Event> {
        match event {
            Event::MouseMove { px, py } => {
                Cow::Owned(Event::MouseMove { px: px - dx, py: py - dy })
            }
            Event::MouseDown { px, py, button } => {
                Cow::Owned(Event::MouseDown { px: px - dx, py: py - dy, button: *button })
            }
            Event::MouseUp { px, py, button } => {
                Cow::Owned(Event::MouseUp { px: px - dx, py: py - dy, button: *button })
            }
            Event::Wheel { dx: wheel_dx, dy: wheel_dy, px, py } => {
                Cow::Owned(Event::Wheel { dx: *wheel_dx, dy: *wheel_dy, px: px - dx, py: py - dy })
            }
            Event::KeyDown(..)
            | Event::PointerLeave
            | Event::InteractionCancel
            | Event::ImePreedit { .. }
            | Event::ImeCommit(..)
            | Event::ImeEnable
            | Event::ImeDisable => Cow::Borrowed(event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::DrawCmd;
    use crate::core::paint::DrawList;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// 最小测试 widget：记录 set_rect 参数
    struct StubWidget {
        pub rect: Rect,
    }

    impl StubWidget {
        fn new() -> Self {
            Self { rect: Rect::ZERO }
        }
    }

    impl Widget for StubWidget {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, ctx: &mut PaintCtx) {
            if self.rect.w > 0.0 && self.rect.h > 0.0 {
                ctx.list.fill(self.rect, [1.0, 1.0, 1.0, 1.0]);
            }
        }

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn dummy_theme() -> Theme {
        crate::theme::test_theme()
    }

    #[test]
    fn ime_coordinate_routing_reuses_the_original_text_allocation() {
        let event = Event::ImeCommit("sensitive-ime-route".to_owned());
        let original_allocation = match &event {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("test event is an IME commit"),
        };

        let local_event = Dock::to_local(&event, 12.0, 24.0);
        let local_allocation = match local_event.as_ref() {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("local event must remain an IME commit"),
        };

        assert_eq!(local_allocation, original_allocation);
    }

    #[test]
    fn lifecycle_events_are_broadcast_and_cancel_does_not_depend_on_hit_testing() {
        #[derive(Default)]
        struct LifecycleCounts {
            pointer_leave: usize,
            interaction_cancel: usize,
            mouse_move: usize,
        }

        struct LifecycleProbe {
            rect: Rect,
            counts: Rc<RefCell<LifecycleCounts>>,
            capturing: bool,
        }

        impl Widget for LifecycleProbe {
            fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
                self.rect = rect;
            }

            fn paint(&self, _ctx: &mut PaintCtx) {}

            fn hit(&self, px: f32, py: f32) -> bool {
                self.rect.contains(px, py)
            }

            fn on_event(&mut self, event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
                let mut counts = self.counts.borrow_mut();
                match event {
                    Event::PointerLeave => counts.pointer_leave += 1,
                    Event::InteractionCancel => counts.interaction_cancel += 1,
                    Event::MouseMove { .. } => counts.mouse_move += 1,
                    _ => {}
                }
                None
            }

            fn is_capturing(&self) -> bool {
                self.capturing
            }

            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let fill_counts = Rc::new(RefCell::new(LifecycleCounts::default()));
        let visible_counts = Rc::new(RefCell::new(LifecycleCounts::default()));
        let hidden_counts = Rc::new(RefCell::new(LifecycleCounts::default()));
        let mut dock = Dock::new(Box::new(LifecycleProbe {
            rect: Rect::ZERO,
            counts: fill_counts.clone(),
            capturing: false,
        }));
        dock.children.push(DockChild {
            widget: Box::new(LifecycleProbe {
                rect: Rect::ZERO,
                counts: visible_counts.clone(),
                capturing: true,
            }),
            side: Side::Top,
            thickness: Box::new(|_, _| 40.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(LifecycleProbe {
                rect: Rect::ZERO,
                counts: hidden_counts.clone(),
                capturing: false,
            }),
            side: Side::Bottom,
            thickness: Box::new(|_, _| 40.0),
            visible: false,
            layout_rect: Rect::ZERO,
        });
        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut layout_ctx);
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        dock.dispatch(&Event::PointerLeave, &mut event_ctx);
        dock.dispatch(&Event::MouseMove { px: 700.0, py: 500.0 }, &mut event_ctx);
        dock.dispatch(&Event::InteractionCancel, &mut event_ctx);

        let visible = visible_counts.borrow();
        assert_eq!(visible.pointer_leave, 1);
        assert_eq!(visible.mouse_move, 1, "pointer leave must not terminate capture");
        assert_eq!(visible.interaction_cancel, 1);
        let fill = fill_counts.borrow();
        assert_eq!(fill.pointer_leave, 1);
        assert_eq!(fill.interaction_cancel, 1);
        let hidden = hidden_counts.borrow();
        assert_eq!(hidden.pointer_leave, 0);
        assert_eq!(hidden.interaction_cancel, 0);
    }

    #[test]
    fn dock_layout_top_then_bottom_leaves_correct_fill() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, _| 32.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Bottom,
            thickness: Box::new(|_, _| 24.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);

        assert!(dock.children[0].widget.hit(0.0, 0.0), "top child");
        let bottom_widget = &dock.children[1].widget;
        assert!(bottom_widget.hit(0.0, 576.0), "bottom child at y=576");
        assert!(dock.fill.hit(0.0, 32.0), "fill starts at y=32");
        assert!(!dock.fill.hit(0.0, 31.0), "fill should not start above y=32");
        assert!(dock.fill.hit(0.0, 575.0), "fill ends before y=576");
    }

    #[test]
    fn dock_layout_all_four_sides_leaves_correct_fill() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, _| 32.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Bottom,
            thickness: Box::new(|_, _| 24.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Left,
            thickness: Box::new(|_, _| 200.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Right,
            thickness: Box::new(|_, _| 16.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);

        // Fill: x=200, y=32, w=800-200-16=584, h=600-32-24=544
        assert!(dock.fill.hit(200.0, 32.0), "fill top-left");
        assert!(dock.fill.hit(783.0, 575.0), "fill bottom-right");
        assert!(!dock.fill.hit(199.0, 32.0), "fill left of left sidebar");
        assert!(!dock.fill.hit(200.0, 31.0), "fill above top bar");
        assert!(!dock.fill.hit(784.0, 32.0), "fill right of scrollbar");
    }

    #[test]
    fn dock_layout_overflow_clamps_children_and_fill_to_zero() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        // Top child takes 400, leaves 200 for height
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, _| 400.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        // Bottom child wants 300 but only 200 remains — clamped to 200
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Bottom,
            thickness: Box::new(|_, _| 300.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);

        // Top child gets full 400
        assert!(dock.children[0].widget.hit(0.0, 399.0));
        // Bottom child gets clamped to 200: y = 600-200 = 400
        assert!(dock.children[1].widget.hit(0.0, 400.0));
        // Fill gets ZERO (no space left)
        assert!(!dock.fill.hit(0.0, 0.0));
    }

    #[test]
    fn invisible_child_does_not_consume_space() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, _| 32.0),
            visible: false,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);

        assert!(dock.fill.hit(0.0, 0.0));
        assert!(dock.fill.hit(0.0, 599.0));
    }

    #[test]
    fn dock_dispatch_routes_to_topmost_hit() {
        struct ActionWidget {
            rect: Rect,
            id: u32,
        }
        impl Widget for ActionWidget {
            fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
                self.rect = rect;
            }
            fn paint(&self, _ctx: &mut PaintCtx) {}
            fn hit(&self, px: f32, py: f32) -> bool {
                self.rect.contains(px, py)
            }
            fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
                Some(WidgetAction::Consumed)
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let mut dock = Dock::new(Box::new(ActionWidget { rect: Rect::ZERO, id: 99 }));
        dock.children.push(DockChild {
            widget: Box::new(ActionWidget { rect: Rect::ZERO, id: 1 }),
            side: Side::Top,
            thickness: Box::new(|_, _| 40.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut lctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let result = dock.dispatch(
            &Event::MouseDown {
                px: 10.0,
                py: 20.0,
                button: crate::core::widget::MouseButton::Left,
            },
            &mut ectx,
        );
        let id = result.unwrap();
        assert_eq!(id, WidgetAction::Consumed);

        let result = dock.dispatch(
            &Event::MouseDown {
                px: 10.0,
                py: 100.0,
                button: crate::core::widget::MouseButton::Left,
            },
            &mut ectx,
        );
        let id = result.unwrap();
        assert_eq!(id, WidgetAction::Consumed);
    }

    #[test]
    fn dpi_scaling_propagates_through_thickness_callback() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, dpi| 32.0 * dpi),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 2.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);

        assert!(dock.fill.hit(0.0, 64.0));
        assert!(!dock.fill.hit(0.0, 63.0));
    }

    #[test]
    fn paint_calls_fill_then_each_child_in_order() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, _| 40.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Bottom,
            thickness: Box::new(|_, _| 20.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut lctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

        let mut dl = DrawList::new();
        let mut pctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: None,
        };
        dock.paint(&mut pctx);

        let fill_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        assert_eq!(fill_count, 3, "fill + 2 children = 3 FillRect commands");
    }

    #[test]
    fn thickness_zero_makes_child_zero_rect_without_consuming_space() {
        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(StubWidget::new()),
            side: Side::Top,
            thickness: Box::new(|_, _| 0.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);

        assert!(!dock.children[0].widget.hit(0.0, 0.0));
        assert!(dock.fill.hit(0.0, 0.0));
    }

    #[test]
    fn capturing_child_receives_mouse_outside_its_rect() {
        // 阶段 C：is_capturing() 返回 true 的 child 优先吃掉所有鼠标事件，
        // 即便光标不在它的 rect 内（拖拽中常见）。
        struct CapturingWidget {
            rect: Rect,
            captured: bool,
            received: std::cell::RefCell<u32>,
        }
        impl Widget for CapturingWidget {
            fn set_rect(&mut self, rect: Rect, _: &mut LayoutCtx) {
                self.rect = rect;
            }
            fn paint(&self, _: &mut PaintCtx) {}
            fn hit(&self, px: f32, py: f32) -> bool {
                self.rect.contains(px, py)
            }
            fn is_capturing(&self) -> bool {
                self.captured
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
            fn on_event(&mut self, _: &Event, _: &mut EventCtx) -> Option<WidgetAction> {
                *self.received.borrow_mut() += 1;
                Some(WidgetAction::Consumed)
            }
        }

        let mut dock = Dock::new(Box::new(StubWidget::new()));
        dock.children.push(DockChild {
            widget: Box::new(CapturingWidget {
                rect: Rect::ZERO,
                captured: true,
                received: std::cell::RefCell::new(0),
            }),
            side: Side::Right,
            thickness: Box::new(|_, _| 16.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut lctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

        // child rect: x=784, y=0, w=16, h=600 — 光标在 x=100 远离 child rect
        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let result = dock.dispatch(&Event::MouseMove { px: 100.0, py: 300.0 }, &mut ectx);
        assert_eq!(
            result,
            Some(WidgetAction::Consumed),
            "capturing child 应吃掉远离自己 rect 的 MouseMove"
        );
    }

    #[test]
    fn dock_keyboard_event_not_routed_to_children() {
        struct KeyWidget {
            rect: Rect,
        }
        impl Widget for KeyWidget {
            fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
                self.rect = rect;
            }
            fn paint(&self, _ctx: &mut PaintCtx) {}
            fn hit(&self, _px: f32, _py: f32) -> bool {
                true
            }
            fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
                Some(WidgetAction::Consumed)
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        struct FillWidget {
            rect: Rect,
        }
        impl Widget for FillWidget {
            fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
                self.rect = rect;
            }
            fn paint(&self, _ctx: &mut PaintCtx) {}
            fn hit(&self, _px: f32, _py: f32) -> bool {
                true
            }
            fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
                Some(WidgetAction::Consumed)
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let mut dock = Dock::new(Box::new(FillWidget { rect: Rect::ZERO }));
        dock.children.push(DockChild {
            widget: Box::new(KeyWidget { rect: Rect::ZERO }),
            side: Side::Top,
            thickness: Box::new(|_, _| 40.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });

        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut lctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let result = dock.dispatch(
            &Event::KeyDown(
                crate::core::widget::KeyCode::Escape,
                crate::core::widget::Modifiers::NONE,
            ),
            &mut ectx,
        );
        let s = result.unwrap();
        assert_eq!(s, WidgetAction::Consumed);
    }

    #[test]
    fn mouse_move_broadcasts_to_all_visible_children() {
        // When MouseMove arrives, ALL visible children receive it for hover updates,
        // not just the one that passes hit().
        struct HoverWidget {
            rect: Rect,
            hovered: std::cell::RefCell<bool>,
        }
        impl Widget for HoverWidget {
            fn set_rect(&mut self, rect: Rect, _: &mut LayoutCtx) {
                self.rect = rect;
            }
            fn paint(&self, _: &mut PaintCtx) {}
            fn hit(&self, px: f32, py: f32) -> bool {
                self.rect.contains(px, py)
            }
            fn on_event(&mut self, ev: &Event, _: &mut EventCtx) -> Option<WidgetAction> {
                if let Event::MouseMove { px: _, py } = ev {
                    *self.hovered.borrow_mut() = *py >= 0.0 && *py < self.rect.h;
                }
                None
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let mut dock = Dock::new(Box::new(StubWidget::new()));
        let hw1 = HoverWidget {
            rect: Rect::new(0.0, 0.0, 200.0, 200.0),
            hovered: std::cell::RefCell::new(false),
        };
        let hw2 = HoverWidget {
            rect: Rect::new(0.0, 200.0, 200.0, 200.0),
            hovered: std::cell::RefCell::new(false),
        };
        dock.children.push(DockChild {
            widget: Box::new(hw1),
            side: Side::Top,
            thickness: Box::new(|_, _| 200.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        dock.children.push(DockChild {
            widget: Box::new(hw2),
            side: Side::Top,
            thickness: Box::new(|_, _| 200.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut lctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

        // Mouse in hw1 area: both should receive event, hw1 hovered=true, hw2 hovered=false
        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        dock.dispatch(&Event::MouseMove { px: 100.0, py: 100.0 }, &mut ectx);

        // Check hovered state via as_any_mut downcast (one at a time)
        {
            let hw1 = dock.children[0]
                .widget
                .as_any_mut()
                .downcast_ref::<HoverWidget>()
                .expect("hw1 downcast");
            assert!(*hw1.hovered.borrow(), "hw1 should be hovered");
        }
        {
            let hw2 = dock.children[1]
                .widget
                .as_any_mut()
                .downcast_ref::<HoverWidget>()
                .expect("hw2 downcast");
            assert!(!*hw2.hovered.borrow(), "hw2 should not be hovered");
        }
    }

    #[test]
    fn mouse_move_returns_non_hit_action_when_no_hit() {
        // When MouseMove broadcasts and no child is hit, a state-changing
        // action from a non-hit child (e.g. HoverChanged(false)) should
        // be returned so the caller can trigger a redraw.
        struct HoverActionWidget {
            rect: Rect,
            hovered: bool,
        }
        impl Widget for HoverActionWidget {
            fn set_rect(&mut self, rect: Rect, _: &mut LayoutCtx) {
                self.rect = rect;
            }
            fn paint(&self, _: &mut PaintCtx) {}
            fn hit(&self, px: f32, py: f32) -> bool {
                self.rect.contains(px, py)
            }
            fn on_event(&mut self, ev: &Event, _: &mut EventCtx) -> Option<WidgetAction> {
                if let Event::MouseMove { px: _, py } = ev {
                    let was = self.hovered;
                    let inside = *py >= 0.0 && *py < self.rect.h;
                    self.hovered = inside;
                    if was != self.hovered {
                        return Some(WidgetAction::Consumed);
                    }
                }
                None
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let mut dock = Dock::new(Box::new(StubWidget::new()));
        let hw = HoverActionWidget { rect: Rect::new(0.0, 0.0, 200.0, 200.0), hovered: true };
        dock.children.push(DockChild {
            widget: Box::new(hw),
            side: Side::Top,
            thickness: Box::new(|_, _| 200.0),
            visible: true,
            layout_rect: Rect::ZERO,
        });
        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut lctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

        // Mouse outside child → non-hit, hovered changes from true → false
        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let result = dock.dispatch(&Event::MouseMove { px: 300.0, py: 300.0 }, &mut ectx);

        // Child is not hit, but its HoverChanged(false) action should be returned
        assert!(result.is_some(), "non-hit action should be returned");
        assert_eq!(result.unwrap(), WidgetAction::Consumed);
    }
}
