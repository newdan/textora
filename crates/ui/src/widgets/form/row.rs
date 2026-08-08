use std::any::Any;
use std::borrow::Cow;

use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId};
use crate::widgets::label::Label;
use crate::widgets::text_box::TextBox;

const DEFAULT_FORM_ROW_MIN_HEIGHT_LOGICAL: f32 = 56.0;
const DEFAULT_FORM_ROW_LABEL_WIDTH_LOGICAL: f32 = 220.0;
const DEFAULT_FORM_ROW_COLUMN_GAP_LOGICAL: f32 = 16.0;
const DEFAULT_FORM_ROW_STACK_GAP_LOGICAL: f32 = 12.0;
const DEFAULT_FORM_ROW_RESPONSIVE_THRESHOLD_LOGICAL: f32 = 640.0;
const DEFAULT_FORM_ROW_PADDING_LOGICAL: [f32; 4] = [8.0, 0.0, 8.0, 0.0];
const STACKED_HEADER_HEIGHT_RATIO: f32 = 0.5;
const DESCRIPTION_PRIMARY_LINE_RATIO: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormRowLayoutMode {
    Columns,
    Stacked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormRowStyle {
    pub min_height_logical: f32,
    pub label_width_logical: f32,
    pub column_gap_logical: f32,
    pub stack_gap_logical: f32,
    pub responsive_threshold_logical: f32,
    pub padding_logical: [f32; 4],
}

impl Default for FormRowStyle {
    fn default() -> Self {
        Self {
            min_height_logical: DEFAULT_FORM_ROW_MIN_HEIGHT_LOGICAL,
            label_width_logical: DEFAULT_FORM_ROW_LABEL_WIDTH_LOGICAL,
            column_gap_logical: DEFAULT_FORM_ROW_COLUMN_GAP_LOGICAL,
            stack_gap_logical: DEFAULT_FORM_ROW_STACK_GAP_LOGICAL,
            responsive_threshold_logical: DEFAULT_FORM_ROW_RESPONSIVE_THRESHOLD_LOGICAL,
            padding_logical: DEFAULT_FORM_ROW_PADDING_LOGICAL,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormRowChildTarget {
    Label,
    Description,
    Control,
}

pub struct FormRow {
    rect: Rect,
    label: Label,
    description: Option<Label>,
    control: Box<dyn Widget>,
    style: FormRowStyle,
    layout_mode: FormRowLayoutMode,
    label_rect: Rect,
    description_rect: Option<Rect>,
    control_rect: Rect,
    focused_id: Option<WidgetId>,
    pointer_target: Option<FormRowChildTarget>,
    hover_target: Option<FormRowChildTarget>,
}

impl FormRow {
    pub fn new(
        label: Label,
        description: Option<Label>,
        control: Box<dyn Widget>,
        style: FormRowStyle,
    ) -> Self {
        Self {
            rect: Rect::ZERO,
            label,
            description,
            control,
            style,
            layout_mode: FormRowLayoutMode::Columns,
            label_rect: Rect::ZERO,
            description_rect: None,
            control_rect: Rect::ZERO,
            focused_id: None,
            pointer_target: None,
            hover_target: None,
        }
    }

    pub fn layout_mode(&self) -> FormRowLayoutMode {
        self.layout_mode
    }

    pub fn label_rect(&self) -> Rect {
        self.label_rect
    }

    pub fn description_rect(&self) -> Option<Rect> {
        self.description_rect
    }

    pub fn control_rect(&self) -> Rect {
        self.control_rect
    }

    pub(crate) fn focused_ime_cursor_rect(&self) -> Option<Rect> {
        if !self.control_has_focus() {
            return None;
        }

        self.control
            .as_any()
            .downcast_ref::<TextBox>()
            .or_else(|| (&*self.control as &dyn Any).downcast_ref::<TextBox>())
            .map(TextBox::ime_cursor_rect)
    }

    fn logical_to_px(value_logical: f32, dpi: f32) -> f32 {
        value_logical * dpi
    }

    fn style_padding_px(&self, dpi: f32) -> [f32; 4] {
        self.style.padding_logical.map(|value| Self::logical_to_px(value, dpi))
    }

    fn local_event<'a>(event: &'a Event, child_rect: Rect) -> Cow<'a, Event> {
        crate::core::dock::Dock::to_local(event, child_rect.x, child_rect.y)
    }

    fn dispatch_to_label(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let local_event = Self::local_event(event, self.label_rect);
        self.label.on_event(&local_event, ctx)
    }

    fn dispatch_to_description(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let description_rect = self.description_rect?;
        let local_event = Self::local_event(event, description_rect);
        self.description.as_mut()?.on_event(&local_event, ctx)
    }

    fn dispatch_to_control(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let local_event = Self::local_event(event, self.control_rect);
        self.control.on_event(&local_event, ctx)
    }

    fn dispatch_to_target(
        &mut self,
        target: FormRowChildTarget,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        match target {
            FormRowChildTarget::Label => self.dispatch_to_label(event, ctx),
            FormRowChildTarget::Description => self.dispatch_to_description(event, ctx),
            FormRowChildTarget::Control => self.dispatch_to_control(event, ctx),
        }
    }

    fn hit_target(&self, px: f32, py: f32) -> Option<FormRowChildTarget> {
        if self.control_rect.contains(px, py) {
            return Some(FormRowChildTarget::Control);
        }
        if self.description_rect.is_some_and(|rect| rect.contains(px, py)) {
            return Some(FormRowChildTarget::Description);
        }
        self.label_rect.contains(px, py).then_some(FormRowChildTarget::Label)
    }

    fn control_has_focus(&self) -> bool {
        let Some(focused_id) = self.focused_id else {
            return false;
        };

        let mut ids = Vec::new();
        self.control.collect_focusable_ids(&mut ids);
        ids.into_iter().any(|id| id == focused_id)
    }

    fn assign_label_block(&mut self, block: Rect, gap_px: f32, ctx: &mut LayoutCtx) {
        if self.description.is_some() {
            let description_gap = gap_px.min(block.h);
            let available_height = (block.h - description_gap).max(0.0);
            let label_height = available_height * DESCRIPTION_PRIMARY_LINE_RATIO;
            let description_height = (available_height - label_height).max(0.0);
            self.label_rect = Rect::new(block.x, block.y, block.w, label_height);
            self.description_rect = Some(Rect::new(
                block.x,
                block.y + label_height + description_gap,
                block.w,
                description_height,
            ));
        } else {
            self.label_rect = block;
            self.description_rect = None;
        }

        self.label.set_rect(Rect::new(0.0, 0.0, self.label_rect.w, self.label_rect.h), ctx);
        if let Some(description) = self.description.as_mut()
            && let Some(description_rect) = self.description_rect
        {
            description.set_rect(Rect::new(0.0, 0.0, description_rect.w, description_rect.h), ctx);
        }
    }

    fn layout_columns(&mut self, content_rect: Rect, ctx: &mut LayoutCtx) {
        let gap_px =
            Self::logical_to_px(self.style.column_gap_logical, ctx.dpi).min(content_rect.w);
        let desired_label_width = Self::logical_to_px(self.style.label_width_logical, ctx.dpi);
        let label_width = desired_label_width.min(content_rect.w);
        let remaining_width_after_label = (content_rect.w - label_width).max(0.0);
        let control_gap = gap_px.min(remaining_width_after_label);
        let control_width = (content_rect.w - label_width - control_gap).max(0.0);
        let label_block = Rect::new(content_rect.x, content_rect.y, label_width, content_rect.h);

        self.layout_mode = FormRowLayoutMode::Columns;
        self.assign_label_block(label_block, gap_px, ctx);
        self.control_rect = Rect::new(
            content_rect.x + label_width + control_gap,
            content_rect.y,
            control_width,
            content_rect.h,
        );
        self.control.set_rect(Rect::new(0.0, 0.0, self.control_rect.w, self.control_rect.h), ctx);
    }

    fn layout_stacked(&mut self, content_rect: Rect, ctx: &mut LayoutCtx) {
        let gap_px = Self::logical_to_px(self.style.stack_gap_logical, ctx.dpi).min(content_rect.h);
        let available_height = (content_rect.h - gap_px).max(0.0);
        let header_height = available_height * STACKED_HEADER_HEIGHT_RATIO;
        let control_height = (available_height - header_height).max(0.0);
        let label_block = Rect::new(content_rect.x, content_rect.y, content_rect.w, header_height);

        self.layout_mode = FormRowLayoutMode::Stacked;
        self.assign_label_block(label_block, gap_px, ctx);
        self.control_rect = Rect::new(
            content_rect.x,
            content_rect.y + header_height + gap_px,
            content_rect.w,
            control_height,
        );
        self.control.set_rect(Rect::new(0.0, 0.0, self.control_rect.w, self.control_rect.h), ctx);
    }
}

impl Widget for FormRow {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        let min_height_px = Self::logical_to_px(self.style.min_height_logical, ctx.dpi);
        let row_height = rect.h.max(min_height_px);
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), row_height);

        let [padding_top, padding_right, padding_bottom, padding_left] =
            self.style_padding_px(ctx.dpi);
        let content_rect =
            self.rect.shrink(padding_top, padding_right, padding_bottom, padding_left);
        let responsive_threshold_px =
            Self::logical_to_px(self.style.responsive_threshold_logical, ctx.dpi);

        if content_rect.w >= responsive_threshold_px {
            self.layout_columns(content_rect, ctx);
        } else {
            self.layout_stacked(content_rect, ctx);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;

        ctx.list.offset = (saved_offset.0 + self.label_rect.x, saved_offset.1 + self.label_rect.y);
        self.label.paint(ctx);

        if let Some(description) = &self.description
            && let Some(description_rect) = self.description_rect
        {
            ctx.list.offset =
                (saved_offset.0 + description_rect.x, saved_offset.1 + description_rect.y);
            description.paint(ctx);
        }

        ctx.list.offset =
            (saved_offset.0 + self.control_rect.x, saved_offset.1 + self.control_rect.y);
        self.control.paint(ctx);
        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        self.control.collect_focusable_ids(output);
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        self.control.set_keyboard_focus(focused_id);
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if self.control.is_capturing()
            && matches!(
                event,
                Event::MouseMove { .. } | Event::MouseUp { .. } | Event::Wheel { .. }
            )
        {
            return self.dispatch_to_control(event, ctx);
        }

        match event {
            Event::MouseDown { px, py, .. } => {
                let target = self.hit_target(*px, *py)?;
                self.pointer_target = Some(target);
                self.hover_target = Some(target);
                self.dispatch_to_target(target, event, ctx)
            }
            Event::MouseMove { px, py } => {
                if let Some(target) = self.pointer_target {
                    return self.dispatch_to_target(target, event, ctx);
                }

                let next_hover_target = self.hit_target(*px, *py);
                let previous_hover_target = self.hover_target;
                let previous_hover_action = if previous_hover_target != next_hover_target {
                    previous_hover_target.and_then(|target| {
                        let saved_cursor_hint = ctx.cursor_hint;
                        let action = self.dispatch_to_target(target, event, ctx);
                        ctx.cursor_hint = saved_cursor_hint;
                        action
                    })
                } else {
                    None
                };
                self.hover_target = next_hover_target;

                if let Some(target) = next_hover_target {
                    return self.dispatch_to_target(target, event, ctx).or(previous_hover_action);
                }

                previous_hover_action
            }
            Event::MouseUp { .. } => {
                let target = self.pointer_target.take()?;
                self.dispatch_to_target(target, event, ctx)
            }
            Event::Wheel { px, py, .. } => {
                let target = self.hit_target(*px, *py)?;
                self.dispatch_to_target(target, event, ctx)
            }
            _ => self
                .control_has_focus()
                .then_some(())
                .and_then(|_| self.dispatch_to_control(event, ctx)),
        }
    }

    fn is_capturing(&self) -> bool {
        self.control.is_capturing()
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
    use crate::core::widget::{ControlAction, KeyCode, Modifiers, MouseButton};
    use crate::theme::Theme;
    use crate::widgets::checkbox::Checkbox;
    use crate::widgets::label::{LabelForeground, LabelStyle};
    use crate::widgets::text_box::TextBox;

    #[derive(Debug)]
    struct TrackingControl {
        id: WidgetId,
        rect: Rect,
        focusable: bool,
        capturing: bool,
        focused_id: Option<WidgetId>,
        events: Vec<Event>,
        next_action: Option<WidgetAction>,
    }

    impl TrackingControl {
        fn new(id: WidgetId, next_action: Option<WidgetAction>) -> Self {
            Self {
                id,
                rect: Rect::ZERO,
                focusable: true,
                capturing: false,
                focused_id: None,
                events: Vec::new(),
                next_action,
            }
        }

        fn capturing(mut self, capturing: bool) -> Self {
            self.capturing = capturing;
            self
        }
    }

    impl Widget for TrackingControl {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, ctx: &mut PaintCtx) {
            ctx.list.fill(self.rect, [0.2, 0.4, 0.8, 1.0]);
        }

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn id(&self) -> Option<WidgetId> {
            Some(self.id)
        }

        fn is_focusable(&self) -> bool {
            self.focusable
        }

        fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
            if self.focusable {
                output.push(self.id);
            }
        }

        fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
            self.focused_id = focused_id;
        }

        fn on_event(&mut self, event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            self.events.push(event.clone());
            self.next_action.clone()
        }

        fn is_capturing(&self) -> bool {
            self.capturing
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn layout_ctx<'a>(theme: &'a Theme, measure: &'a mut NoopMeasure) -> LayoutCtx<'a> {
        LayoutCtx { ui_measure: None, measure, theme, dpi: 1.0 }
    }

    fn event_ctx<'a>(theme: &'a Theme) -> EventCtx<'a> {
        EventCtx { cursor_hint: None, theme, dpi: 1.0 }
    }

    fn layout(row: &mut FormRow, width: f32, height: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        row.set_rect(Rect::new(0.0, 0.0, width, height), &mut layout_ctx);
    }

    fn fixture_label(text: &str) -> Label {
        Label::new(text, LabelStyle::default())
    }

    fn fixture_description(text: &str) -> Label {
        Label::new(
            text,
            LabelStyle { foreground: LabelForeground::ThemeMuted, ..LabelStyle::default() },
        )
    }

    fn checkbox_row() -> FormRow {
        FormRow::new(
            fixture_label("Display name"),
            None,
            Box::new(Checkbox::new(WidgetId(101), false)),
            FormRowStyle::default(),
        )
    }

    #[test]
    fn form_row_switches_from_columns_to_stack_at_threshold() {
        let mut row = checkbox_row();

        layout(&mut row, 760.0, 72.0);
        assert_eq!(row.layout_mode(), FormRowLayoutMode::Columns);

        layout(&mut row, 520.0, 72.0);
        assert_eq!(row.layout_mode(), FormRowLayoutMode::Stacked);
        assert!(row.control_rect().y > row.label_rect().y);
    }

    #[test]
    fn form_row_assigns_description_between_label_and_control() {
        let mut row = FormRow::new(
            fixture_label("Display name"),
            Some(fixture_description("Shown in the left sidebar.")),
            Box::new(Checkbox::new(WidgetId(102), true)),
            FormRowStyle::default(),
        );

        layout(&mut row, 520.0, 96.0);

        let description_rect = row.description_rect().expect("description rect should exist");
        assert_eq!(row.layout_mode(), FormRowLayoutMode::Stacked);
        assert!(description_rect.y >= row.label_rect().bottom());
        assert!(row.control_rect().y >= description_rect.bottom());
    }

    #[test]
    fn form_row_routes_local_events_and_focus_to_control() {
        let forwarded_action =
            WidgetAction::Control(ControlAction::Toggled { id: WidgetId(77), checked: true });
        let mut row = FormRow::new(
            fixture_label("Enable sync"),
            Some(fixture_description("Keep this device updated.")),
            Box::new(TrackingControl::new(WidgetId(77), Some(forwarded_action.clone()))),
            FormRowStyle::default(),
        );
        layout(&mut row, 760.0, 88.0);

        let mut ids = Vec::new();
        row.collect_focusable_ids(&mut ids);
        assert_eq!(ids, vec![WidgetId(77)]);

        row.set_keyboard_focus(Some(WidgetId(77)));

        let theme = crate::theme::test_theme();
        let mut ctx = event_ctx(&theme);
        let control_rect = row.control_rect();
        let click_x = control_rect.x + 14.0;
        let click_y = control_rect.y + 10.0;
        let mouse_action = row.on_event(
            &Event::MouseDown { px: click_x, py: click_y, button: MouseButton::Left },
            &mut ctx,
        );
        assert_eq!(mouse_action, Some(forwarded_action.clone()));

        let key_action = row.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut ctx);
        assert_eq!(key_action, Some(forwarded_action));

        let tracking = row
            .control
            .as_any()
            .downcast_ref::<TrackingControl>()
            .expect("tracking control should be preserved");
        assert_eq!(tracking.focused_id, Some(WidgetId(77)));
        assert_eq!(
            tracking.events,
            vec![
                Event::MouseDown { px: 14.0, py: 10.0, button: MouseButton::Left },
                Event::KeyDown(KeyCode::Enter, Modifiers::NONE),
            ],
        );
    }

    #[test]
    fn form_row_capture_does_not_bypass_focus_for_keyboard_or_ime() {
        let forwarded_action =
            WidgetAction::Control(ControlAction::Toggled { id: WidgetId(88), checked: true });
        let mut row = FormRow::new(
            fixture_label("Input method"),
            None,
            Box::new(
                TrackingControl::new(WidgetId(88), Some(forwarded_action.clone())).capturing(true),
            ),
            FormRowStyle::default(),
        );
        layout(&mut row, 760.0, 72.0);
        row.set_keyboard_focus(Some(WidgetId(999)));

        let theme = crate::theme::test_theme();
        let mut ctx = event_ctx(&theme);
        let control_rect = row.control_rect();
        let mouse_x = control_rect.x + 12.0;
        let mouse_y = control_rect.y + 9.0;

        assert_eq!(row.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut ctx), None);
        assert_eq!(
            row.on_event(
                &Event::ImePreedit { text: String::from("ni"), cursor: Some((1, 1)) },
                &mut ctx,
            ),
            None
        );
        assert_eq!(row.on_event(&Event::ImeCommit(String::from("你")), &mut ctx), None);
        assert_eq!(row.on_event(&Event::ImeEnable, &mut ctx), None);
        assert_eq!(row.on_event(&Event::ImeDisable, &mut ctx), None);

        let mouse_move_action =
            row.on_event(&Event::MouseMove { px: mouse_x, py: mouse_y }, &mut ctx);
        assert_eq!(mouse_move_action, Some(forwarded_action.clone()));

        let mouse_up_action = row.on_event(
            &Event::MouseUp { px: mouse_x, py: mouse_y, button: MouseButton::Left },
            &mut ctx,
        );
        assert_eq!(mouse_up_action, Some(forwarded_action));

        let tracking = row
            .control
            .as_any()
            .downcast_ref::<TrackingControl>()
            .expect("tracking control should be preserved");
        assert_eq!(
            tracking.events,
            vec![
                Event::MouseMove { px: 12.0, py: 9.0 },
                Event::MouseUp { px: 12.0, py: 9.0, button: MouseButton::Left },
            ],
        );
    }

    #[test]
    fn form_row_reports_focused_text_box_ime_cursor_rect_in_local_coordinates() {
        let text_box_id = WidgetId(303);
        let mut row = FormRow::new(
            fixture_label("Input method"),
            None,
            Box::new(TextBox::with_id(text_box_id)),
            FormRowStyle::default(),
        );
        layout(&mut row, 760.0, 72.0);

        assert_eq!(row.focused_ime_cursor_rect(), None);

        row.set_keyboard_focus(Some(text_box_id));

        let expected_ime_rect = {
            let text_box = row
                .control
                .as_any_mut()
                .downcast_mut::<TextBox>()
                .expect("row control should be a text box");
            text_box.on_ime(&crate::widgets::text_box::TextBoxIme::Preedit {
                text: "ni".into(),
                cursor: Some((2, 2)),
            });
            text_box.ime_cursor_rect()
        };

        let row = row;
        let ime_rect = row
            .focused_ime_cursor_rect()
            .expect("focused text box should expose an ime cursor rect");
        assert_eq!(ime_rect, expected_ime_rect);
    }

    #[test]
    fn form_row_returns_none_when_focus_is_missing_or_not_a_text_box() {
        let checkbox_id = WidgetId(101);
        let mut row = checkbox_row();

        let row = row;
        assert_eq!(row.focused_ime_cursor_rect(), None);

        let mut row = row;
        row.set_keyboard_focus(Some(checkbox_id));

        let row = row;
        assert_eq!(row.focused_ime_cursor_rect(), None);
    }

    #[test]
    fn ime_control_routing_reuses_the_original_text_allocation() {
        let event = Event::ImeCommit("form-row-sensitive-ime-route".to_owned());
        let original_allocation = match &event {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("test event is an IME commit"),
        };

        let local_event = FormRow::local_event(&event, Rect::new(8.0, 12.0, 100.0, 60.0));
        let local_allocation = match local_event.as_ref() {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("local event must remain an IME commit"),
        };

        assert_eq!(local_allocation, original_allocation);
    }
}
