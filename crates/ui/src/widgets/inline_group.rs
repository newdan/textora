use std::any::Any;

use crate::core::{
    ChildEventRouter, Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
    dispatch_child_event_route,
};

const DEFAULT_GAP_LOGICAL: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InlineWidth {
    Fixed(f32),
    Flex(f32),
    Content(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossAlignment {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainAlignment {
    Start,
    Center,
    End,
}

pub struct InlineChild {
    pub widget: Box<dyn Widget>,
    pub width: InlineWidth,
    rect: Rect,
    cross_size_logical: Option<f32>,
}

impl InlineChild {
    pub fn fixed(widget: Box<dyn Widget>, width_logical: f32) -> Self {
        Self {
            widget,
            width: InlineWidth::Fixed(width_logical),
            rect: Rect::ZERO,
            cross_size_logical: None,
        }
    }

    pub fn flex(widget: Box<dyn Widget>, weight: f32) -> Self {
        Self {
            widget,
            width: InlineWidth::Flex(weight),
            rect: Rect::ZERO,
            cross_size_logical: None,
        }
    }

    pub fn content(widget: Box<dyn Widget>, measured_width_logical: f32) -> Self {
        Self {
            widget,
            width: InlineWidth::Content(measured_width_logical),
            rect: Rect::ZERO,
            cross_size_logical: None,
        }
    }

    pub fn with_cross_size(mut self, cross_size_logical: f32) -> Self {
        self.cross_size_logical = Some(cross_size_logical);
        self
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }
}

pub struct InlineGroup {
    rect: Rect,
    children: Vec<InlineChild>,
    gap_logical: f32,
    main_alignment: MainAlignment,
    alignment: CrossAlignment,
    event_router: ChildEventRouter<usize>,
}

impl InlineGroup {
    pub fn new(children: Vec<InlineChild>) -> Self {
        Self {
            rect: Rect::ZERO,
            children,
            gap_logical: DEFAULT_GAP_LOGICAL,
            main_alignment: MainAlignment::Start,
            alignment: CrossAlignment::Stretch,
            event_router: ChildEventRouter::default(),
        }
    }

    pub fn with_gap(mut self, gap_logical: f32) -> Self {
        self.gap_logical = gap_logical;
        self
    }

    pub fn with_main_alignment(mut self, alignment: MainAlignment) -> Self {
        self.main_alignment = alignment;
        self
    }

    pub fn with_alignment(mut self, alignment: CrossAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn child_rect(&self, index: usize) -> Rect {
        self.children[index].rect()
    }

    fn gap_px(&self, dpi: f32) -> f32 {
        self.gap_logical * dpi
    }

    fn fixed_width_px(width: InlineWidth, dpi: f32) -> Option<f32> {
        match width {
            InlineWidth::Fixed(width_logical) | InlineWidth::Content(width_logical) => {
                Some(width_logical * dpi)
            }
            InlineWidth::Flex(_) => None,
        }
    }

    fn child_local_rect(layout_rect: Rect) -> Rect {
        Rect::new(0.0, 0.0, layout_rect.w, layout_rect.h)
    }

    fn child_height(
        alignment: CrossAlignment,
        group_height: f32,
        child: &InlineChild,
        dpi: f32,
    ) -> f32 {
        match alignment {
            CrossAlignment::Stretch => group_height,
            CrossAlignment::Start | CrossAlignment::Center | CrossAlignment::End => child
                .cross_size_logical
                .map(|cross_size_logical| (cross_size_logical * dpi).clamp(0.0, group_height))
                .unwrap_or(group_height),
        }
    }

    fn child_origin_y(alignment: CrossAlignment, group_height: f32, child_height: f32) -> f32 {
        match alignment {
            CrossAlignment::Start | CrossAlignment::Stretch => 0.0,
            CrossAlignment::Center => (group_height - child_height) * 0.5,
            CrossAlignment::End => group_height - child_height,
        }
    }

    fn child_local_event(&self, event: &Event, child_rect: Rect) -> Event {
        match event {
            Event::MouseMove { px, py } => {
                Event::MouseMove { px: px - child_rect.x, py: py - child_rect.y }
            }
            Event::PointerLeave => Event::PointerLeave,
            Event::MouseDown { px, py, button } => {
                Event::MouseDown { px: px - child_rect.x, py: py - child_rect.y, button: *button }
            }
            Event::MouseUp { px, py, button } => {
                Event::MouseUp { px: px - child_rect.x, py: py - child_rect.y, button: *button }
            }
            Event::InteractionCancel => Event::InteractionCancel,
            Event::Wheel { dx, dy, px, py } => {
                Event::Wheel { dx: *dx, dy: *dy, px: px - child_rect.x, py: py - child_rect.y }
            }
            Event::KeyDown(key_code, modifiers) => Event::KeyDown(*key_code, *modifiers),
            Event::ImePreedit { text, cursor } => {
                Event::ImePreedit { text: text.clone(), cursor: *cursor }
            }
            Event::ImeCommit(text) => Event::ImeCommit(text.clone()),
            Event::ImeEnable => Event::ImeEnable,
            Event::ImeDisable => Event::ImeDisable,
        }
    }

    fn hit_child_index(&self, px: f32, py: f32) -> Option<usize> {
        self.children
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, child)| child.rect.contains(px, py).then_some(index))
    }

    fn child_index_for_focus(&self, focused_id: WidgetId) -> Option<usize> {
        self.children.iter().position(|child| {
            let mut focusable_ids = Vec::new();
            child.widget.collect_focusable_ids(&mut focusable_ids);
            focusable_ids.contains(&focused_id)
        })
    }

    fn dispatch_to_child(
        &mut self,
        child_index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let child_rect = self.children[child_index].rect;
        let local_event = self.child_local_event(event, child_rect);
        self.children[child_index].widget.on_event(&local_event, ctx)
    }
}

impl Widget for InlineGroup {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);

        let gap_px = self.gap_px(ctx.dpi);
        let child_count = self.children.len();
        let total_gap_width = gap_px * child_count.saturating_sub(1) as f32;
        let mut remaining_width = (rect.w - total_gap_width).max(0.0);
        let mut total_flex_weight = 0.0;

        for child in &self.children {
            if let Some(width_px) = Self::fixed_width_px(child.width, ctx.dpi) {
                remaining_width -= width_px;
            } else if let InlineWidth::Flex(weight) = child.width {
                total_flex_weight += weight.max(0.0);
            }
        }

        let flexible_width_budget = remaining_width.max(0.0);
        let fixed_content_width = rect.w - flexible_width_budget;
        let content_offset = if total_flex_weight > 0.0 {
            0.0
        } else {
            match self.main_alignment {
                MainAlignment::Start => 0.0,
                MainAlignment::Center => (rect.w - fixed_content_width) * 0.5,
                MainAlignment::End => rect.w - fixed_content_width,
            }
        }
        .max(0.0);
        let mut cursor_x = content_offset;
        let alignment = self.alignment;
        let group_height = rect.h;

        for child in &mut self.children {
            let desired_width_px = match child.width {
                InlineWidth::Fixed(width_logical) | InlineWidth::Content(width_logical) => {
                    width_logical * ctx.dpi
                }
                InlineWidth::Flex(weight) if total_flex_weight > 0.0 => {
                    flexible_width_budget * (weight.max(0.0) / total_flex_weight)
                }
                InlineWidth::Flex(_) => 0.0,
            };
            let child_x = cursor_x.min(rect.w);
            let width_px = desired_width_px.max(0.0).min((rect.w - child_x).max(0.0));
            let child_height = Self::child_height(alignment, group_height, child, ctx.dpi);
            let child_y = Self::child_origin_y(alignment, group_height, child_height);
            let layout_rect = Rect::new(child_x, child_y, width_px, child_height.max(0.0));
            child.rect = layout_rect;
            child.widget.set_rect(Self::child_local_rect(layout_rect), ctx);
            cursor_x += width_px + gap_px;
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;
        for child in &self.children {
            let child_offset = ctx.list.offset;
            ctx.list.offset = (child_offset.0 + child.rect.x, child_offset.1 + child.rect.y);
            child.widget.paint(ctx);
            ctx.list.offset = child_offset;
        }
        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        for child in &self.children {
            child.widget.collect_focusable_ids(output);
        }
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        let focused_child = focused_id.and_then(|id| self.child_index_for_focus(id));
        self.event_router.set_focused_target(focused_child);
        for child in &mut self.children {
            child.widget.set_keyboard_focus(focused_id);
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let hit_target = match ev {
            Event::MouseDown { px, py, .. }
            | Event::MouseMove { px, py }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => self.hit_child_index(*px, *py),
            _ => None,
        };
        let route = self.event_router.route_event(ev, hit_target);
        let broadcast_targets = (0..self.children.len()).rev();
        let dispatch =
            dispatch_child_event_route(route, ev, broadcast_targets, ctx, |target, event, ctx| {
                self.dispatch_to_child(target, event, ctx)
            });
        dispatch.action.or_else(|| {
            (dispatch.broadcast && dispatch.state_changed).then_some(WidgetAction::Consumed)
        })
    }

    fn is_capturing(&self) -> bool {
        self.event_router.is_capturing()
            || self.children.iter().any(|child| child.widget.is_capturing())
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
    use crate::core::widget::{ControlAction, KeyCode, Modifiers, MouseButton, TextPayload};
    use crate::theme::Theme;
    use winit::window::CursorIcon;

    #[derive(Clone, Debug, PartialEq)]
    enum LoggedEvent {
        MouseDown { px: f32, py: f32 },
        MouseUp { px: f32, py: f32 },
        MouseMove { px: f32, py: f32 },
        PointerLeave,
        InteractionCancel,
        KeyDown(KeyCode),
        ImeCommit(String),
        ImePreedit(String),
        ImeEnable,
        ImeDisable,
    }

    struct TrackingWidget {
        id: Option<WidgetId>,
        focusable: bool,
        rect: Rect,
        color: [f32; 4],
        forwarded_focus: Option<WidgetId>,
        events: Vec<LoggedEvent>,
        action: Option<WidgetAction>,
        consume_mouse_move: bool,
        hover_cursor: Option<CursorIcon>,
    }

    impl TrackingWidget {
        fn new(id: Option<WidgetId>, focusable: bool, color: [f32; 4]) -> Self {
            Self {
                id,
                focusable,
                rect: Rect::ZERO,
                color,
                forwarded_focus: None,
                events: Vec::new(),
                action: None,
                consume_mouse_move: false,
                hover_cursor: None,
            }
        }

        fn with_action(mut self, action: WidgetAction) -> Self {
            self.action = Some(action);
            self
        }

        fn consuming_mouse_move(mut self) -> Self {
            self.consume_mouse_move = true;
            self
        }

        fn with_hover_cursor(mut self, cursor: CursorIcon) -> Self {
            self.hover_cursor = Some(cursor);
            self
        }
    }

    impl Widget for TrackingWidget {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, ctx: &mut PaintCtx) {
            ctx.list.fill(self.rect, self.color);
        }

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn id(&self) -> Option<WidgetId> {
            self.id
        }

        fn is_focusable(&self) -> bool {
            self.focusable
        }

        fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
            self.forwarded_focus = focused_id;
        }

        fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
            match ev {
                Event::MouseDown { px, py, button: MouseButton::Left } => {
                    self.events.push(LoggedEvent::MouseDown { px: *px, py: *py });
                }
                Event::MouseUp { px, py, button: MouseButton::Left } => {
                    self.events.push(LoggedEvent::MouseUp { px: *px, py: *py });
                }
                Event::MouseMove { px, py } => {
                    self.events.push(LoggedEvent::MouseMove { px: *px, py: *py });
                    if self.rect.contains(*px, *py)
                        && let Some(cursor) = self.hover_cursor
                    {
                        ctx.cursor_hint = Some(cursor);
                    }

                    if self.consume_mouse_move {
                        return Some(WidgetAction::Consumed);
                    }
                }
                Event::PointerLeave => {
                    self.events.push(LoggedEvent::PointerLeave);
                }
                Event::InteractionCancel => {
                    self.events.push(LoggedEvent::InteractionCancel);
                }
                Event::KeyDown(key, _modifiers) => {
                    self.events.push(LoggedEvent::KeyDown(*key));
                }
                Event::ImeCommit(text) => {
                    self.events.push(LoggedEvent::ImeCommit(text.clone()));
                }
                Event::ImePreedit { text, .. } => {
                    self.events.push(LoggedEvent::ImePreedit(text.clone()));
                }
                Event::ImeEnable => {
                    self.events.push(LoggedEvent::ImeEnable);
                }
                Event::ImeDisable => {
                    self.events.push(LoggedEvent::ImeDisable);
                }
                _ => {}
            }
            self.action.clone()
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn theme() -> Theme {
        crate::theme::test_theme()
    }

    fn layout_ctx<'a>(theme: &'a Theme, measure: &'a mut NoopMeasure, dpi: f32) -> LayoutCtx<'a> {
        LayoutCtx { ui_measure: None, measure, theme, dpi }
    }

    fn event_ctx<'a>(theme: &'a Theme, dpi: f32) -> EventCtx<'a> {
        EventCtx::new(theme, dpi)
    }

    fn downcast_tracking_widget(widget: &dyn Widget) -> &TrackingWidget {
        widget
            .as_any()
            .downcast_ref::<TrackingWidget>()
            .expect("test helper should always contain TrackingWidget")
    }

    fn fixture_group(children: Vec<InlineChild>, gap_logical: f32) -> InlineGroup {
        InlineGroup::new(children).with_gap(gap_logical)
    }

    #[test]
    fn inline_group_end_alignment_pins_fixed_children_to_right_edge() {
        let mut group = InlineGroup::new(vec![
            InlineChild::fixed(Box::new(TrackingWidget::new(None, false, [0.0; 4])), 40.0),
            InlineChild::fixed(Box::new(TrackingWidget::new(None, false, [0.0; 4])), 20.0),
        ])
        .with_gap(0.0)
        .with_main_alignment(MainAlignment::End);
        let theme = theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&theme, &mut measure, 1.0);

        group.set_rect(Rect::new(0.0, 0.0, 100.0, 20.0), &mut ctx);

        assert_eq!(group.child_rect(0), Rect::new(40.0, 0.0, 40.0, 20.0));
        assert_eq!(group.child_rect(1), Rect::new(80.0, 0.0, 20.0, 20.0));
    }

    #[test]
    fn inline_group_assigns_fixed_content_and_flexible_widths() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [1.0, 0.0, 0.0, 1.0])),
                    80.0,
                ),
                InlineChild::content(
                    Box::new(TrackingWidget::new(None, false, [0.0, 1.0, 0.0, 1.0])),
                    32.0,
                ),
                InlineChild::flex(
                    Box::new(TrackingWidget::new(None, false, [0.0, 0.0, 1.0, 1.0])),
                    1.0,
                ),
            ],
            8.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&app_theme, &mut measure, 1.0);

        group.set_rect(Rect::new(0.0, 0.0, 300.0, 32.0), &mut ctx);

        assert_eq!(group.child_rect(0), Rect::new(0.0, 0.0, 80.0, 32.0));
        assert_eq!(group.child_rect(1), Rect::new(88.0, 0.0, 32.0, 32.0));
        assert_eq!(group.child_rect(2), Rect::new(128.0, 0.0, 172.0, 32.0));
    }

    #[test]
    fn inline_group_scales_gap_and_fixed_width_only_once_per_dpi() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [1.0, 0.0, 0.0, 1.0])),
                    20.0,
                ),
                InlineChild::flex(
                    Box::new(TrackingWidget::new(None, false, [0.0, 0.0, 1.0, 1.0])),
                    1.0,
                ),
            ],
            4.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&app_theme, &mut measure, 2.0);

        group.set_rect(Rect::new(0.0, 0.0, 200.0, 40.0), &mut ctx);

        assert_eq!(group.child_rect(0), Rect::new(0.0, 0.0, 40.0, 40.0));
        assert_eq!(group.child_rect(1), Rect::new(48.0, 0.0, 152.0, 40.0));
    }

    #[test]
    fn inline_group_uses_explicit_cross_size_for_non_stretch_alignment() {
        let mut group = InlineGroup::new(vec![
            InlineChild::fixed(
                Box::new(TrackingWidget::new(None, false, [1.0, 0.0, 0.0, 1.0])),
                40.0,
            )
            .with_cross_size(12.0),
            InlineChild::fixed(
                Box::new(TrackingWidget::new(None, false, [0.0, 1.0, 0.0, 1.0])),
                20.0,
            ),
        ])
        .with_gap(8.0)
        .with_alignment(CrossAlignment::Center);
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&app_theme, &mut measure, 1.0);

        group.set_rect(Rect::new(0.0, 0.0, 80.0, 30.0), &mut ctx);

        assert_eq!(group.child_rect(0), Rect::new(0.0, 9.0, 40.0, 12.0));
        assert_eq!(group.child_rect(1), Rect::new(48.0, 0.0, 20.0, 30.0));
    }

    #[test]
    fn inline_group_paints_children_with_parent_offset_applied_once() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [1.0, 0.0, 0.0, 1.0])),
                    24.0,
                ),
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [0.0, 1.0, 0.0, 1.0])),
                    30.0,
                ),
            ],
            6.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&app_theme, &mut measure, 1.0);
        group.set_rect(Rect::new(10.0, 20.0, 60.0, 18.0), &mut layout_ctx);

        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx::new(&mut draw_list, &app_theme, 1.0);
        paint_ctx.list.offset = (10.0, 20.0);
        group.paint(&mut paint_ctx);

        assert_eq!(
            draw_list.cmds,
            vec![
                DrawCmd::FillRect {
                    rect: Rect::new(10.0, 20.0, 24.0, 18.0),
                    color: [1.0, 0.0, 0.0, 1.0],
                    radius: 0.0,
                },
                DrawCmd::FillRect {
                    rect: Rect::new(40.0, 20.0, 30.0, 18.0),
                    color: [0.0, 1.0, 0.0, 1.0],
                    radius: 0.0,
                },
            ]
        );
    }

    #[test]
    fn inline_group_set_rect_with_non_zero_origin_does_not_reapply_parent_offset() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(
                        TrackingWidget::new(Some(WidgetId(11)), true, [1.0, 0.0, 0.0, 1.0])
                            .with_action(WidgetAction::Consumed),
                    ),
                    24.0,
                ),
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(Some(WidgetId(22)), true, [0.0, 1.0, 0.0, 1.0])),
                    30.0,
                ),
            ],
            6.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&app_theme, &mut measure, 1.0);
        group.set_rect(Rect::new(10.0, 20.0, 60.0, 18.0), &mut layout_ctx);

        assert!(group.hit(20.0, 10.0));
        assert!(!group.hit(80.0, 10.0));

        let mut event_ctx = event_ctx(&app_theme, 1.0);
        let action = group.on_event(
            &Event::MouseDown { px: 20.0, py: 10.0, button: MouseButton::Left },
            &mut event_ctx,
        );

        assert_eq!(action, Some(WidgetAction::Consumed));
        let first = downcast_tracking_widget(group.children[0].widget.as_ref());
        let second = downcast_tracking_widget(group.children[1].widget.as_ref());
        assert_eq!(first.events, vec![LoggedEvent::MouseDown { px: 20.0, py: 10.0 }]);
        assert!(second.events.is_empty());
    }

    #[test]
    fn inline_group_routes_mouse_events_only_to_hit_child() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(
                        TrackingWidget::new(Some(WidgetId(11)), true, [1.0, 0.0, 0.0, 1.0])
                            .with_action(WidgetAction::Control(ControlAction::Activated {
                                id: WidgetId(11),
                            })),
                    ),
                    60.0,
                ),
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(Some(WidgetId(22)), true, [0.0, 1.0, 0.0, 1.0])),
                    60.0,
                ),
            ],
            8.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&app_theme, &mut measure, 1.0);
        group.set_rect(Rect::new(0.0, 0.0, 128.0, 20.0), &mut layout_ctx);
        let mut event_ctx = event_ctx(&app_theme, 1.0);

        let action = group.on_event(
            &Event::MouseDown { px: 20.0, py: 10.0, button: MouseButton::Left },
            &mut event_ctx,
        );

        assert_eq!(
            action,
            Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(11) }))
        );
        let first = downcast_tracking_widget(group.children[0].widget.as_ref());
        let second = downcast_tracking_widget(group.children[1].widget.as_ref());
        assert_eq!(first.events, vec![LoggedEvent::MouseDown { px: 20.0, py: 10.0 }]);
        assert!(second.events.is_empty());
    }

    #[test]
    fn inline_group_sends_outside_move_to_previous_hover_child_and_keeps_new_cursor_hint() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(
                        TrackingWidget::new(Some(WidgetId(1)), true, [1.0, 0.0, 0.0, 1.0])
                            .consuming_mouse_move(),
                    ),
                    40.0,
                ),
                InlineChild::fixed(
                    Box::new(
                        TrackingWidget::new(Some(WidgetId(2)), true, [0.0, 1.0, 0.0, 1.0])
                            .consuming_mouse_move()
                            .with_hover_cursor(CursorIcon::Pointer),
                    ),
                    40.0,
                ),
            ],
            8.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&app_theme, &mut measure, 1.0);
        group.set_rect(Rect::new(0.0, 0.0, 88.0, 20.0), &mut layout_ctx);

        let mut first_move_ctx = event_ctx(&app_theme, 1.0);
        let first_action =
            group.on_event(&Event::MouseMove { px: 10.0, py: 10.0 }, &mut first_move_ctx);
        assert_eq!(first_action, Some(WidgetAction::Consumed));

        let mut second_move_ctx = event_ctx(&app_theme, 1.0);
        let second_action =
            group.on_event(&Event::MouseMove { px: 56.0, py: 10.0 }, &mut second_move_ctx);

        assert_eq!(second_action, Some(WidgetAction::Consumed));
        assert_eq!(second_move_ctx.cursor_hint, Some(CursorIcon::Pointer));
        let first = downcast_tracking_widget(group.children[0].widget.as_ref());
        let second = downcast_tracking_widget(group.children[1].widget.as_ref());
        assert_eq!(
            first.events,
            vec![LoggedEvent::MouseMove { px: 10.0, py: 10.0 }, LoggedEvent::PointerLeave]
        );
        assert_eq!(second.events, vec![LoggedEvent::MouseMove { px: 8.0, py: 10.0 }]);
    }

    #[test]
    fn inline_group_leave_preserves_press_and_cancel_clears_all_transient_ownership() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [1.0, 0.0, 0.0, 1.0])),
                    40.0,
                ),
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [0.0, 1.0, 0.0, 1.0])),
                    40.0,
                ),
            ],
            8.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&app_theme, &mut measure, 1.0);
        group.set_rect(Rect::new(0.0, 0.0, 88.0, 20.0), &mut layout_ctx);
        let mut event_ctx = event_ctx(&app_theme, 1.0);

        group.on_event(&Event::MouseMove { px: 10.0, py: 10.0 }, &mut event_ctx);
        group.on_event(
            &Event::MouseDown { px: 10.0, py: 10.0, button: MouseButton::Left },
            &mut event_ctx,
        );
        group.on_event(&Event::PointerLeave, &mut event_ctx);

        assert!(group.is_capturing(), "pointer leave must preserve the pressed child owner");
        assert_eq!(group.event_router.pointer_capture_target(), Some(0));
        assert_eq!(group.event_router.hovered_target(), None);

        group.on_event(&Event::InteractionCancel, &mut event_ctx);

        assert!(!group.is_capturing());
        assert_eq!(group.event_router.pointer_capture_target(), None);
        assert_eq!(group.event_router.hovered_target(), None);
        let first = downcast_tracking_widget(group.children[0].widget.as_ref());
        let second = downcast_tracking_widget(group.children[1].widget.as_ref());
        assert_eq!(
            first.events,
            vec![
                LoggedEvent::MouseMove { px: 10.0, py: 10.0 },
                LoggedEvent::MouseDown { px: 10.0, py: 10.0 },
                LoggedEvent::PointerLeave,
                LoggedEvent::InteractionCancel,
            ]
        );
        assert_eq!(second.events, vec![LoggedEvent::InteractionCancel]);
    }

    #[test]
    fn inline_group_routes_keyboard_and_ime_to_focused_child() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(Some(WidgetId(1)), true, [1.0, 0.0, 0.0, 1.0])),
                    40.0,
                ),
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(Some(WidgetId(2)), true, [0.0, 1.0, 0.0, 1.0])),
                    40.0,
                ),
            ],
            8.0,
        );
        group.set_keyboard_focus(Some(WidgetId(2)));
        let app_theme = theme();
        let mut event_ctx = event_ctx(&app_theme, 1.0);

        let key_action =
            group.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx);
        let ime_action = group.on_event(&Event::ImeCommit("ni".into()), &mut event_ctx);

        assert_eq!(key_action, None);
        assert_eq!(ime_action, None);
        let first = downcast_tracking_widget(group.children[0].widget.as_ref());
        let second = downcast_tracking_widget(group.children[1].widget.as_ref());
        assert!(first.events.is_empty());
        assert_eq!(
            second.events,
            vec![LoggedEvent::KeyDown(KeyCode::Enter), LoggedEvent::ImeCommit("ni".into())]
        );
    }

    #[test]
    fn inline_group_collects_focusable_ids_and_forwards_selected_focus() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(Some(WidgetId(7)), true, [1.0, 0.0, 0.0, 1.0])),
                    40.0,
                ),
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(Some(WidgetId(8)), true, [0.0, 1.0, 0.0, 1.0])),
                    40.0,
                ),
            ],
            8.0,
        );
        let mut ids = Vec::new();

        group.collect_focusable_ids(&mut ids);
        group.set_keyboard_focus(Some(WidgetId(8)));

        assert_eq!(ids, vec![WidgetId(7), WidgetId(8)]);
        let first = downcast_tracking_widget(group.children[0].widget.as_ref());
        let second = downcast_tracking_widget(group.children[1].widget.as_ref());
        assert_eq!(first.forwarded_focus, Some(WidgetId(8)));
        assert_eq!(second.forwarded_focus, Some(WidgetId(8)));
    }

    #[test]
    fn inline_group_preserves_child_control_actions_without_reinterpretation() {
        let mut group = fixture_group(
            vec![InlineChild::fixed(
                Box::new(
                    TrackingWidget::new(Some(WidgetId(99)), true, [1.0, 0.0, 0.0, 1.0])
                        .with_action(WidgetAction::Control(ControlAction::TextEdited {
                            id: WidgetId(99),
                            value: TextPayload::Plain("raw".into()),
                        })),
                ),
                48.0,
            )],
            8.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&app_theme, &mut measure, 1.0);
        group.set_rect(Rect::new(0.0, 0.0, 48.0, 20.0), &mut layout_ctx);
        let mut event_ctx = event_ctx(&app_theme, 1.0);

        let action = group.on_event(
            &Event::MouseDown { px: 10.0, py: 10.0, button: MouseButton::Left },
            &mut event_ctx,
        );

        assert_eq!(
            action,
            Some(WidgetAction::Control(ControlAction::TextEdited {
                id: WidgetId(99),
                value: TextPayload::Plain("raw".into()),
            }))
        );
    }

    #[test]
    fn inline_group_clamps_child_rects_within_group_width_in_narrow_space() {
        let mut group = fixture_group(
            vec![
                InlineChild::fixed(
                    Box::new(TrackingWidget::new(None, false, [1.0, 0.0, 0.0, 1.0])),
                    50.0,
                ),
                InlineChild::content(
                    Box::new(TrackingWidget::new(None, false, [0.0, 1.0, 0.0, 1.0])),
                    50.0,
                ),
                InlineChild::flex(
                    Box::new(TrackingWidget::new(None, false, [0.0, 0.0, 1.0, 1.0])),
                    1.0,
                ),
            ],
            8.0,
        );
        let app_theme = theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&app_theme, &mut measure, 1.0);

        group.set_rect(Rect::new(0.0, 0.0, 80.0, 20.0), &mut ctx);

        assert_eq!(group.child_rect(0), Rect::new(0.0, 0.0, 50.0, 20.0));
        assert_eq!(group.child_rect(1), Rect::new(58.0, 0.0, 22.0, 20.0));
        assert_eq!(group.child_rect(2), Rect::new(80.0, 0.0, 0.0, 20.0));
    }
}
