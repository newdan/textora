use std::any::Any;
use std::borrow::Cow;

use crate::core::widget::ControlAction;
use crate::core::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode, Event, EventCtx, KeyCode,
    LayoutCtx, Modifiers, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
};
use crate::widgets::form::section::FormSection;
use crate::widgets::scrollbar::{
    SCROLLBAR_RESERVE_PX, ScrollbarAction, ScrollbarInput, ScrollbarWidget,
};

const DEFAULT_FORM_VIEW_SECTION_GAP_LOGICAL: f32 = 24.0;
const FORM_SCROLLBAR_COORDINATE_SCALE: f64 = 1_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FormViewStyle {
    pub section_gap_logical: f32,
}

impl Default for FormViewStyle {
    fn default() -> Self {
        Self { section_gap_logical: DEFAULT_FORM_VIEW_SECTION_GAP_LOGICAL }
    }
}

pub struct FormView {
    rect: Rect,
    sections: Vec<FormSection>,
    scroll_offset: f32,
    content_height: f32,
    focused_id: Option<WidgetId>,
    style: FormViewStyle,
    section_rects: Vec<Rect>,
    pointer_section_index: Option<usize>,
    hover_section_index: Option<usize>,
    scrollbar: ScrollbarWidget,
    scrollbar_rect: Rect,
}

impl FormView {
    pub fn new(style: FormViewStyle) -> Self {
        Self {
            rect: Rect::ZERO,
            sections: Vec::new(),
            scroll_offset: 0.0,
            content_height: 0.0,
            focused_id: None,
            style,
            section_rects: Vec::new(),
            pointer_section_index: None,
            hover_section_index: None,
            scrollbar: ScrollbarWidget::vertical(),
            scrollbar_rect: Rect::ZERO,
        }
    }

    pub fn set_sections(&mut self, sections: Vec<FormSection>, ctx: &mut LayoutCtx) {
        self.sections = sections;
        self.pointer_section_index = None;
        self.hover_section_index = None;
        self.reset_scroll();
        self.layout_sections(ctx);
    }

    pub fn replace_sections_preserving_state(
        &mut self,
        sections: Vec<FormSection>,
        ctx: &mut LayoutCtx,
    ) {
        let previous_scroll = self.scroll_offset;
        let previous_focus = self.focused_id;
        self.sections = sections;
        self.pointer_section_index = None;
        self.hover_section_index = None;
        self.layout_sections(ctx);
        let _ = self.set_scroll_offset(previous_scroll);
        self.set_keyboard_focus(previous_focus);
    }

    pub fn reset_scroll(&mut self) {
        let _ = self.set_scroll_offset(0.0);
    }

    pub fn scroll_offset(&self) -> f32 {
        self.scroll_offset
    }

    pub fn focused_id(&self) -> Option<WidgetId> {
        self.focused_id
    }

    pub fn focused_ime_cursor_rect(&self) -> Option<Rect> {
        let section_index = self.focused_section_index()?;
        let section_rect = *self.section_rects.get(section_index)?;
        let ime_rect = self.sections.get(section_index)?.focused_ime_cursor_rect()?;
        Some(Rect::new(
            section_rect.x + ime_rect.x,
            section_rect.y - self.scroll_offset + ime_rect.y,
            ime_rect.w,
            ime_rect.h,
        ))
    }

    fn logical_to_px(value_logical: f32, dpi: f32) -> f32 {
        value_logical * dpi
    }

    fn section_gap_px(&self, dpi: f32) -> f32 {
        Self::logical_to_px(self.style.section_gap_logical, dpi)
    }

    fn max_scroll_offset(&self) -> f32 {
        (self.content_height - self.rect.h).max(0.0)
    }

    fn has_overflow(&self) -> bool {
        self.content_height > self.rect.h
    }

    fn scrollbar_input(&self) -> ScrollbarInput {
        ScrollbarInput {
            viewport_height_px: self.rect.h as f64 * FORM_SCROLLBAR_COORDINATE_SCALE,
            total_display_rows: (self.content_height.max(0.0)
                * FORM_SCROLLBAR_COORDINATE_SCALE as f32)
                .ceil() as usize,
            scroll_top_rows: self.scroll_offset as f64 * FORM_SCROLLBAR_COORDINATE_SCALE,
        }
    }

    fn sync_scrollbar_input(&mut self) {
        self.scrollbar.set_input(self.scrollbar_input());
    }

    fn layout_scrollbar(&mut self, ctx: &mut LayoutCtx) {
        self.sync_scrollbar_input();
        if !self.has_overflow() || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            self.scrollbar_rect = Rect::ZERO;
            self.scrollbar.set_rect(Rect::ZERO, ctx);
            return;
        }

        let scrollbar_width = (SCROLLBAR_RESERVE_PX * ctx.dpi).min(self.rect.w);
        self.scrollbar_rect =
            Rect::new(self.rect.w - scrollbar_width, 0.0, scrollbar_width, self.rect.h);
        self.scrollbar
            .set_rect(Rect::new(0.0, 0.0, self.scrollbar_rect.w, self.scrollbar_rect.h), ctx);
    }

    fn clamp_scroll_offset(&mut self) {
        let _ = self.set_scroll_offset(self.scroll_offset);
    }

    fn scroll_by(&mut self, delta: f32) -> bool {
        self.set_scroll_offset(self.scroll_offset + delta)
    }

    fn set_scroll_offset(&mut self, scroll_offset: f32) -> bool {
        let previous_offset = self.scroll_offset;
        self.scroll_offset = scroll_offset.clamp(0.0, self.max_scroll_offset());
        self.sync_scrollbar_input();
        self.scroll_offset != previous_offset
    }

    fn layout_sections(&mut self, ctx: &mut LayoutCtx) {
        self.section_rects.clear();

        if self.sections.is_empty() {
            self.content_height = 0.0;
            self.clamp_scroll_offset();
            self.layout_scrollbar(ctx);
            return;
        }

        let section_gap = self.section_gap_px(ctx.dpi);
        let section_width = self.rect.w.max(0.0);
        let mut cursor_y = 0.0;

        for section in &mut self.sections {
            let section_height = section.preferred_height(ctx.dpi);
            section.set_rect(Rect::new(0.0, 0.0, section_width, section_height), ctx);
            self.section_rects.push(Rect::new(0.0, cursor_y, section_width, section_height));
            cursor_y += section_height + section_gap;
        }

        self.content_height = cursor_y - section_gap;
        self.clamp_scroll_offset();
        self.set_keyboard_focus(self.focused_id);
        self.layout_scrollbar(ctx);
    }

    fn section_draw_rect(&self, section_index: usize) -> Option<Rect> {
        let rect = *self.section_rects.get(section_index)?;
        Some(Rect::new(rect.x, rect.y - self.scroll_offset, rect.w, rect.h))
    }

    fn visible_section_rect(&self, section_rect: Rect) -> Option<Rect> {
        let left = section_rect.x.max(0.0);
        let top = section_rect.y.max(0.0);
        let right = section_rect.right().min(self.rect.w);
        let bottom = section_rect.bottom().min(self.rect.h);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Rect::new(left - section_rect.x, top - section_rect.y, right - left, bottom - top))
    }

    fn local_event<'a>(event: &'a Event, child_rect: Rect) -> Cow<'a, Event> {
        crate::core::dock::Dock::to_local(event, child_rect.x, child_rect.y)
    }

    fn section_index_at(&self, px: f32, py: f32) -> Option<usize> {
        self.section_rects
            .iter()
            .enumerate()
            .find_map(|(index, _)| self.section_draw_rect(index)?.contains(px, py).then_some(index))
    }

    fn focused_section_index(&self) -> Option<usize> {
        let focused_id = self.focused_id?;
        self.sections.iter().position(|section| {
            let mut ids = Vec::new();
            section.collect_focusable_ids(&mut ids);
            ids.into_iter().any(|id| id == focused_id)
        })
    }

    fn capturing_section_index(&self) -> Option<usize> {
        self.sections.iter().position(|section| section.is_capturing())
    }

    fn dispatch_to_section(
        &mut self,
        section_index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let section_draw_rect = self.section_draw_rect(section_index)?;
        let local_event = Self::local_event(event, section_draw_rect);
        self.sections.get_mut(section_index)?.on_event(&local_event, ctx)
    }

    fn scrollbar_event<'a>(&self, event: &'a Event) -> Cow<'a, Event> {
        crate::core::dock::Dock::to_local(event, self.scrollbar_rect.x, self.scrollbar_rect.y)
    }

    fn apply_scrollbar_action(&mut self, action: ScrollbarAction) {
        match action {
            ScrollbarAction::DragTo(scroll_offset) => {
                let _ = self
                    .set_scroll_offset((scroll_offset / FORM_SCROLLBAR_COORDINATE_SCALE) as f32);
            }
            ScrollbarAction::PageUp => {
                let _ = self.scroll_by(-self.rect.h);
            }
            ScrollbarAction::PageDown => {
                let _ = self.scroll_by(self.rect.h);
            }
            ScrollbarAction::StartDrag
            | ScrollbarAction::EndDrag
            | ScrollbarAction::HoverChanged(_) => {}
        }
    }

    fn dispatch_scrollbar_event(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let local_event = self.scrollbar_event(event);
        let action = self.scrollbar.on_event(local_event.as_ref(), ctx)?;
        let WidgetAction::Scrollbar(scrollbar_action) = action else {
            return Some(WidgetAction::Consumed);
        };
        self.apply_scrollbar_action(scrollbar_action);
        Some(WidgetAction::Consumed)
    }

    fn clear_hover_section(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let section_index = self.hover_section_index.take()?;
        let saved_cursor_hint = ctx.cursor_hint;
        let action = self.dispatch_to_section(section_index, event, ctx);
        ctx.cursor_hint = saved_cursor_hint;
        action
    }

    fn intersection_rect(first: Rect, second: Rect) -> Option<Rect> {
        let x = first.x.max(second.x);
        let y = first.y.max(second.y);
        let right = first.right().min(second.right());
        let bottom = first.bottom().min(second.bottom());
        let width = right - x;
        let height = bottom - y;
        (width > 0.0 && height > 0.0).then_some(Rect::new(x, y, width, height))
    }

    fn section_local_visible_rect(&self, section_draw_rect: Rect) -> Option<Rect> {
        let viewport_rect = Rect::new(0.0, 0.0, self.rect.w, self.rect.h);
        let visible_rect = Self::intersection_rect(section_draw_rect, viewport_rect)?;
        Some(Rect::new(
            visible_rect.x - section_draw_rect.x,
            visible_rect.y - section_draw_rect.y,
            visible_rect.w,
            visible_rect.h,
        ))
    }

    fn visible_focusable_ids(&self) -> Vec<WidgetId> {
        let mut ids = Vec::new();
        for (index, section) in self.sections.iter().enumerate() {
            let Some(section_draw_rect) = self.section_draw_rect(index) else {
                continue;
            };
            let Some(local_visible_rect) = self.section_local_visible_rect(section_draw_rect)
            else {
                continue;
            };
            section.collect_visible_focusable_ids(local_visible_rect, &mut ids);
        }
        ids
    }

    fn cycle_focus(&mut self, modifiers: Modifiers) -> Option<WidgetAction> {
        let focusable_ids = self.visible_focusable_ids();
        if focusable_ids.is_empty() {
            return None;
        }

        let next_index = match self.focused_id.and_then(|focused_id| {
            focusable_ids.iter().position(|candidate_id| *candidate_id == focused_id)
        }) {
            Some(current_index) if modifiers.shift => {
                if current_index == 0 {
                    focusable_ids.len() - 1
                } else {
                    current_index - 1
                }
            }
            Some(current_index) => (current_index + 1) % focusable_ids.len(),
            None if modifiers.shift => focusable_ids.len() - 1,
            None => 0,
        };
        let next_id = focusable_ids[next_index];
        self.set_keyboard_focus(Some(next_id));
        Some(WidgetAction::Control(ControlAction::FocusRequested { id: next_id }))
    }
}

impl Widget for FormView {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        self.layout_sections(ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let saved_offset = ctx.list.offset;
        let theme = ctx.theme;
        let dpi = ctx.dpi;
        let offset = ctx.offset;
        let global_alpha = ctx.global_alpha;
        let shaper = &mut ctx.shaper;

        ctx.list.clip(self.rect, |list| {
            for (section, section_rect) in self.sections.iter().zip(self.section_rects.iter()) {
                list.offset = (
                    saved_offset.0 + section_rect.x,
                    saved_offset.1 + section_rect.y - self.scroll_offset,
                );
                let mut section_ctx = PaintCtx {
                    list,
                    theme,
                    dpi,
                    offset,
                    global_alpha,
                    shaper: shaper.as_deref_mut(),
                };
                section.paint(&mut section_ctx);
            }
            list.offset = saved_offset;
        });

        if self.scrollbar_rect.w > 0.0 && self.scrollbar_rect.h > 0.0 {
            ctx.list.offset =
                (saved_offset.0 + self.scrollbar_rect.x, saved_offset.1 + self.scrollbar_rect.y);
            self.scrollbar.paint(ctx);
        }
        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn is_capturing(&self) -> bool {
        self.pointer_section_index.is_some()
            || self.scrollbar.is_dragging()
            || self.capturing_section_index().is_some()
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        for section in &self.sections {
            section.collect_focusable_ids(output);
        }
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        for section in &mut self.sections {
            section.set_keyboard_focus(focused_id);
        }
    }

    fn collect_accessibility_nodes(
        &self,
        context: &AccessibilityContext,
        output: &mut Vec<AccessibilityNode>,
    ) {
        for (section_index, section) in self.sections.iter().enumerate() {
            let Some(section_rect) = self.section_draw_rect(section_index) else { continue };
            let Some(visible_rect) = self.visible_section_rect(section_rect) else { continue };
            section.collect_accessibility_nodes_in_viewport(
                &context.offset_by(section_rect.x, section_rect.y),
                visible_rect,
                output,
            );
        }
        if self.scrollbar_rect.w > 0.0 && self.scrollbar_rect.h > 0.0 {
            self.scrollbar.collect_accessibility_nodes(
                &context.offset_by(self.scrollbar_rect.x, self.scrollbar_rect.y),
                output,
            );
        }
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if let Some(action) =
            self.sections.iter_mut().find_map(|section| section.on_accessibility_action(request))
        {
            return Some(action);
        }
        self.scrollbar.on_accessibility_action(request)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if matches!(event, Event::PointerLeave | Event::InteractionCancel) {
            let container_changed = if matches!(event, Event::InteractionCancel) {
                self.pointer_section_index.take().is_some()
                    | self.hover_section_index.take().is_some()
            } else {
                self.hover_section_index.take().is_some()
            };
            let mut first_action = self.dispatch_scrollbar_event(event, ctx);
            for section_index in 0..self.sections.len() {
                if let Some(action) = self.dispatch_to_section(section_index, event, ctx)
                    && first_action.is_none()
                {
                    first_action = Some(action);
                }
            }
            return first_action.or_else(|| container_changed.then_some(WidgetAction::Consumed));
        }

        if self.scrollbar.is_dragging()
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            return self.dispatch_scrollbar_event(event, ctx);
        }

        if let Some(section_index) = self.capturing_section_index()
            && matches!(
                event,
                Event::MouseMove { .. } | Event::MouseUp { .. } | Event::Wheel { .. }
            )
        {
            let action = self.dispatch_to_section(section_index, event, ctx);
            if matches!(event, Event::MouseUp { .. }) {
                self.pointer_section_index = None;
            }
            return action;
        }

        match event {
            Event::MouseDown { px, py, .. } => {
                if self.scrollbar_rect.contains(*px, *py) {
                    return self.dispatch_scrollbar_event(event, ctx);
                }
                let section_index = self.section_index_at(*px, *py)?;
                self.pointer_section_index = Some(section_index);
                self.dispatch_to_section(section_index, event, ctx)
            }
            Event::MouseMove { px, py } => {
                let scrollbar_action = self.dispatch_scrollbar_event(event, ctx);
                if self.scrollbar_rect.contains(*px, *py) {
                    let hover_section_action = self.clear_hover_section(event, ctx);
                    return scrollbar_action
                        .or(hover_section_action)
                        .or(Some(WidgetAction::Consumed));
                }

                if let Some(section_index) = self.pointer_section_index {
                    return self
                        .dispatch_to_section(section_index, event, ctx)
                        .or(scrollbar_action);
                }

                let next_hover_section_index = self.section_index_at(*px, *py);
                let previous_hover_action = if self.hover_section_index != next_hover_section_index
                {
                    self.hover_section_index.and_then(|section_index| {
                        let saved_cursor_hint = ctx.cursor_hint;
                        let action = self.dispatch_to_section(section_index, event, ctx);
                        ctx.cursor_hint = saved_cursor_hint;
                        action
                    })
                } else {
                    None
                };
                self.hover_section_index = next_hover_section_index;

                if let Some(section_index) = next_hover_section_index {
                    return self
                        .dispatch_to_section(section_index, event, ctx)
                        .or(previous_hover_action)
                        .or(scrollbar_action);
                }

                previous_hover_action.or(scrollbar_action)
            }
            Event::MouseUp { .. } => {
                let section_index = self.pointer_section_index.take()?;
                self.dispatch_to_section(section_index, event, ctx)
            }
            Event::Wheel { dy, px, py, .. } => {
                if let Some(action) = self
                    .section_index_at(*px, *py)
                    .and_then(|section_index| self.dispatch_to_section(section_index, event, ctx))
                {
                    return Some(action);
                }
                if !self.rect.contains(*px, *py) {
                    return None;
                }
                let _ = self.scroll_by(-*dy);
                Some(WidgetAction::Consumed)
            }
            Event::KeyDown(KeyCode::Tab, modifiers) => self.cycle_focus(*modifiers),
            _ => self
                .focused_section_index()
                .and_then(|section_index| self.dispatch_to_section(section_index, event, ctx)),
        }
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
    use crate::core::widget::{ControlAction, MouseButton};
    use crate::theme::Theme;
    use crate::widgets::checkbox::Checkbox;
    use crate::widgets::form::{FormRow, FormRowStyle, FormSectionStyle};
    use crate::widgets::label::{Label, LabelForeground, LabelStyle};
    use crate::widgets::scrollbar::ScrollbarInput;
    use crate::widgets::text_box::TextBox;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug, Default)]
    struct TrackingState {
        focused_id: Option<WidgetId>,
        events: Vec<Event>,
    }

    #[derive(Debug)]
    struct TrackingControl {
        id: WidgetId,
        rect: Rect,
        state: Rc<RefCell<TrackingState>>,
        next_action: Option<WidgetAction>,
    }

    impl TrackingControl {
        fn new(
            id: WidgetId,
            state: Rc<RefCell<TrackingState>>,
            next_action: Option<WidgetAction>,
        ) -> Self {
            Self { id, rect: Rect::ZERO, state, next_action }
        }
    }

    impl Widget for TrackingControl {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn id(&self) -> Option<WidgetId> {
            Some(self.id)
        }

        fn is_focusable(&self) -> bool {
            true
        }

        fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
            output.push(self.id);
        }

        fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
            self.state.borrow_mut().focused_id = focused_id;
        }

        fn on_event(&mut self, event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            self.state.borrow_mut().events.push(event.clone());
            self.next_action.clone()
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

    fn fixture_label(text: &str) -> Label {
        Label::new(text, LabelStyle::default())
    }

    fn fixture_description(text: &str) -> Label {
        Label::new(
            text,
            LabelStyle { foreground: LabelForeground::ThemeMuted, ..LabelStyle::default() },
        )
    }

    fn checkbox_row(id: WidgetId) -> FormRow {
        FormRow::new(
            fixture_label("Display name"),
            None,
            Box::new(Checkbox::new(id, false)),
            FormRowStyle::default(),
        )
    }

    fn content_height(height: f32) -> f32 {
        height
    }

    fn viewport_height(height: f32) -> f32 {
        height
    }

    fn laid_out_form_view(content_height: f32, viewport_height: f32) -> FormView {
        let row_count = (content_height / 56.0).ceil().max(1.0) as usize;
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_sections(
            vec![FormSection::new(
                fixture_label("General"),
                Some(fixture_description("Configure this workspace.")),
                (0..row_count).map(|index| checkbox_row(WidgetId(index as u64 + 1))).collect(),
                FormSectionStyle::default(),
            )],
            &mut ctx,
        );
        view.set_rect(Rect::new(0.0, 0.0, 720.0, viewport_height), &mut ctx);
        view.content_height = content_height;
        view.layout_scrollbar(&mut ctx);
        view
    }

    fn wheel(view: &mut FormView, dy: f32) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut ctx = event_ctx(&theme);
        view.on_event(&Event::Wheel { dx: 0.0, dy, px: 16.0, py: 16.0 }, &mut ctx)
    }

    fn pointer_event(view: &mut FormView, event: Event) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut ctx = event_ctx(&theme);
        view.on_event(&event, &mut ctx)
    }

    fn paint_for_test(view: &FormView) -> DrawList {
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        view.paint(&mut paint_ctx);
        draw_list
    }

    #[test]
    fn form_view_hides_scrollbar_when_content_fits() {
        let view = laid_out_form_view(content_height(200.0), viewport_height(300.0));

        assert_eq!(view.scrollbar_rect, Rect::ZERO);
        assert_eq!(view.scroll_offset(), 0.0);
    }

    #[test]
    fn form_view_configures_scrollbar_from_pixel_scroll_state() {
        let view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

        assert_eq!(view.scrollbar_rect, Rect::new(706.0, 0.0, 14.0, 300.0));
        assert_eq!(
            view.scrollbar.input,
            ScrollbarInput {
                viewport_height_px: 300_000.0,
                total_display_rows: 900_000,
                scroll_top_rows: 0.0,
            },
        );
    }

    #[test]
    fn form_view_wheel_keeps_scrollbar_position_in_sync() {
        let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

        assert_eq!(wheel(&mut view, -120.0), Some(WidgetAction::Consumed));

        assert_eq!(view.scroll_offset(), 120.0);
        assert_eq!(view.scrollbar.input.scroll_top_rows, 120_000.0);
    }

    #[test]
    fn form_view_scrollbar_preserves_fractional_scroll_range() {
        let mut view = laid_out_form_view(content_height(300.1), viewport_height(300.0));

        assert_eq!(view.scrollbar.input.viewport_height_px, 300_000.0);
        assert_eq!(view.scrollbar.input.total_display_rows, 300_100);

        view.apply_scrollbar_action(ScrollbarAction::DragTo(100.0));

        assert!((view.scroll_offset() - 0.1).abs() < 0.001);
    }

    #[test]
    fn form_view_scrollbar_pages_and_drags_the_same_scroll_offset() {
        let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

        assert_eq!(
            pointer_event(
                &mut view,
                Event::MouseDown { px: 715.0, py: 200.0, button: MouseButton::Left },
            ),
            Some(WidgetAction::Consumed),
        );
        assert_eq!(view.scroll_offset(), 300.0);

        assert_eq!(
            pointer_event(
                &mut view,
                Event::MouseDown { px: 715.0, py: 150.0, button: MouseButton::Left },
            ),
            Some(WidgetAction::Consumed),
        );
        assert!(view.is_capturing());

        assert_eq!(
            pointer_event(&mut view, Event::MouseMove { px: 715.0, py: 250.0 }),
            Some(WidgetAction::Consumed),
        );
        assert_eq!(view.scroll_offset(), 600.0);

        assert_eq!(
            pointer_event(
                &mut view,
                Event::MouseUp { px: 760.0, py: 250.0, button: MouseButton::Left },
            ),
            Some(WidgetAction::Consumed),
        );
        assert!(!view.is_capturing());
        assert_eq!(view.scrollbar.input.scroll_top_rows, 600_000.0);
    }

    #[test]
    fn form_view_clears_section_hover_when_pointer_enters_scrollbar() {
        let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));
        let theme = crate::theme::test_theme();
        let mut ctx = event_ctx(&theme);

        view.on_event(&Event::MouseMove { px: 200.0, py: 40.0 }, &mut ctx);
        assert_eq!(view.hover_section_index, Some(0));

        view.on_event(&Event::MouseMove { px: 715.0, py: 40.0 }, &mut ctx);

        assert_eq!(view.hover_section_index, None);
    }

    #[test]
    fn form_view_propagates_scrollbar_hover_exit_for_repaint() {
        let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

        assert_eq!(
            pointer_event(&mut view, Event::MouseMove { px: 715.0, py: 40.0 }),
            Some(WidgetAction::Consumed),
        );
        assert_eq!(
            pointer_event(&mut view, Event::MouseMove { px: 900.0, py: 400.0 }),
            Some(WidgetAction::Consumed),
        );
    }

    #[test]
    fn form_view_leave_preserves_nested_press_and_cancel_clears_container_capture() {
        let mut view = laid_out_form_view(content_height(200.0), viewport_height(300.0));
        let pointer = (300.0, 80.0);

        assert!(
            pointer_event(
                &mut view,
                Event::MouseDown { px: pointer.0, py: pointer.1, button: MouseButton::Left },
            )
            .is_some()
        );
        assert!(view.is_capturing());

        assert!(pointer_event(&mut view, Event::PointerLeave).is_some());
        assert!(view.is_capturing());

        assert_eq!(
            pointer_event(&mut view, Event::InteractionCancel),
            Some(WidgetAction::Consumed)
        );
        assert!(!view.is_capturing());
        assert_eq!(pointer_event(&mut view, Event::InteractionCancel), None);
        assert_eq!(
            pointer_event(
                &mut view,
                Event::MouseUp { px: pointer.0, py: pointer.1, button: MouseButton::Left },
            ),
            None
        );
    }

    #[test]
    fn form_view_clips_sections_and_clamps_scroll() {
        let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));
        let action = wheel(&mut view, -10_000.0);
        assert_eq!(action, Some(WidgetAction::Consumed));
        assert_eq!(view.scroll_offset(), 600.0);
        let draw = paint_for_test(&view);
        let pop_clip_index = draw
            .cmds
            .iter()
            .rposition(|command| matches!(command, DrawCmd::PopClip))
            .expect("form content should close its clip");
        assert!(matches!(draw.cmds.first(), Some(DrawCmd::PushClip(_))));
        assert!(
            draw.cmds.len() > pop_clip_index + 1,
            "overflowing form should paint its scrollbar after clipped content",
        );
    }

    #[test]
    fn form_view_set_sections_relayouts_and_resets_scroll() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 300.0), &mut layout_ctx);
        view.set_sections(
            vec![FormSection::new(
                fixture_label("General"),
                Some(fixture_description("Configure this workspace.")),
                vec![
                    checkbox_row(WidgetId(1)),
                    checkbox_row(WidgetId(2)),
                    checkbox_row(WidgetId(3)),
                    checkbox_row(WidgetId(4)),
                    checkbox_row(WidgetId(5)),
                    checkbox_row(WidgetId(6)),
                ],
                FormSectionStyle::default(),
            )],
            &mut layout_ctx,
        );
        let _ = wheel(&mut view, -1_000.0);
        assert!(view.scroll_offset() > 0.0);

        view.set_sections(
            vec![
                FormSection::new(
                    fixture_label("Editor"),
                    Some(fixture_description("Tune the editing experience.")),
                    vec![checkbox_row(WidgetId(11))],
                    FormSectionStyle::default(),
                ),
                FormSection::new(
                    fixture_label("Interface"),
                    Some(fixture_description("Adjust window behavior.")),
                    vec![checkbox_row(WidgetId(12))],
                    FormSectionStyle::default(),
                ),
            ],
            &mut layout_ctx,
        );

        assert_eq!(view.scroll_offset(), 0.0);
        assert_eq!(view.section_rects.len(), 2);
        assert!(view.content_height > view.section_rects[0].h);
    }

    #[test]
    fn replacing_sections_preserves_scroll_and_focus() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 400.0, 120.0), &mut layout_ctx);
        view.set_sections(
            vec![FormSection::new(
                fixture_label("Original"),
                None,
                (1..=6).map(|id| checkbox_row(WidgetId(id))).collect(),
                FormSectionStyle::default(),
            )],
            &mut layout_ctx,
        );
        let _ = wheel(&mut view, -96.0);
        view.set_keyboard_focus(Some(WidgetId(2)));
        let previous_scroll = view.scroll_offset();

        view.replace_sections_preserving_state(
            vec![FormSection::new(
                fixture_label("Replacement"),
                None,
                (1..=6).map(|id| checkbox_row(WidgetId(id))).collect(),
                FormSectionStyle::default(),
            )],
            &mut layout_ctx,
        );

        assert_eq!(view.focused_id(), Some(WidgetId(2)));
        assert_eq!(view.scroll_offset(), previous_scroll);
    }

    #[test]
    fn form_view_uses_each_section_preferred_height_for_layout() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 320.0), &mut layout_ctx);
        view.set_sections(
            vec![
                FormSection::new(
                    fixture_label("Editor"),
                    None,
                    vec![checkbox_row(WidgetId(1))],
                    FormSectionStyle::default(),
                ),
                FormSection::new(
                    fixture_label("General"),
                    Some(fixture_description("Configure this workspace.")),
                    vec![checkbox_row(WidgetId(2))],
                    FormSectionStyle::default(),
                ),
            ],
            &mut layout_ctx,
        );

        assert_eq!(view.section_rects[0], Rect::new(0.0, 0.0, 720.0, 84.0));
        assert_eq!(view.section_rects[1], Rect::new(0.0, 108.0, 720.0, 114.0));
        assert_eq!(view.content_height, 222.0);
    }

    #[test]
    fn form_view_translates_mouse_coordinates_into_scrolled_section_space() {
        let tracking_state = Rc::new(RefCell::new(TrackingState::default()));
        let section = FormSection::new(
            fixture_label("Automation"),
            Some(fixture_description("Choose how updates are applied.")),
            vec![FormRow::new(
                fixture_label("Install automatically"),
                None,
                Box::new(TrackingControl::new(
                    WidgetId(77),
                    tracking_state.clone(),
                    Some(WidgetAction::Control(ControlAction::Toggled {
                        id: WidgetId(77),
                        checked: true,
                    })),
                )),
                FormRowStyle::default(),
            )],
            FormSectionStyle::default(),
        );
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 100.0), &mut layout_ctx);
        view.set_sections(vec![section], &mut layout_ctx);
        view.scroll_offset = 24.0;
        view.set_keyboard_focus(Some(WidgetId(77)));

        let probe_state = Rc::new(RefCell::new(TrackingState::default()));
        let mut probe_row = FormRow::new(
            fixture_label("Install automatically"),
            None,
            Box::new(TrackingControl::new(WidgetId(77), probe_state, None)),
            FormRowStyle::default(),
        );
        probe_row.set_rect(Rect::new(0.0, 0.0, 720.0, 56.0), &mut layout_ctx);
        let control_rect = probe_row.control_rect();
        let section_rect = view.section_rects[0];
        let draw_y = section_rect.y - view.scroll_offset;
        let section_content_y =
            view.sections[0].preferred_height(1.0) - view.sections[0].content_height();
        let control_local_x = 24.0;
        let control_local_y = 14.0;
        let mut event_ctx = event_ctx(&theme);
        let action = view.on_event(
            &Event::MouseDown {
                px: section_rect.x + control_rect.x + control_local_x,
                py: draw_y + section_content_y + control_rect.y + control_local_y,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        );

        assert_eq!(
            action,
            Some(WidgetAction::Control(ControlAction::Toggled { id: WidgetId(77), checked: true })),
        );

        let tracking = tracking_state.borrow();
        assert_eq!(tracking.focused_id, Some(WidgetId(77)));
        assert!(
            tracking.events.contains(&Event::MouseDown {
                px: control_local_x,
                py: control_local_y,
                button: MouseButton::Left,
            }),
            "mouse coordinates should be translated into section-local space",
        );
        assert!(
            tracking.events.iter().any(|event| {
                matches!(event, Event::MouseDown { button: MouseButton::Left, .. })
            }),
            "tracking control should receive the click event",
        );
    }

    #[test]
    fn form_view_cycles_focus_on_tab_and_shift_tab() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 300.0), &mut layout_ctx);
        view.set_sections(
            vec![
                FormSection::new(
                    fixture_label("General"),
                    Some(fixture_description("Configure this workspace.")),
                    vec![checkbox_row(WidgetId(1))],
                    FormSectionStyle::default(),
                ),
                FormSection::new(
                    fixture_label("Editor"),
                    Some(fixture_description("Tune the editor.")),
                    vec![checkbox_row(WidgetId(2))],
                    FormSectionStyle::default(),
                ),
            ],
            &mut layout_ctx,
        );

        let mut event_ctx = event_ctx(&theme);
        let first_action =
            view.on_event(&Event::KeyDown(KeyCode::Tab, Modifiers::NONE), &mut event_ctx);
        assert_eq!(
            first_action,
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(1) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(1)));

        let second_action =
            view.on_event(&Event::KeyDown(KeyCode::Tab, Modifiers::NONE), &mut event_ctx);
        assert_eq!(
            second_action,
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(2) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(2)));

        let previous_action = view.on_event(
            &Event::KeyDown(KeyCode::Tab, Modifiers { shift: true, ..Modifiers::NONE }),
            &mut event_ctx,
        );
        assert_eq!(
            previous_action,
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(1) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(1)));
    }

    #[test]
    fn form_view_cycles_focus_only_across_rows_visible_in_viewport() {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 120.0), &mut layout_ctx);
        view.set_sections(
            vec![FormSection::new(
                fixture_label("General"),
                None,
                vec![
                    checkbox_row(WidgetId(1)),
                    checkbox_row(WidgetId(2)),
                    checkbox_row(WidgetId(3)),
                    checkbox_row(WidgetId(4)),
                ],
                FormSectionStyle::default(),
            )],
            &mut layout_ctx,
        );

        let mut event_ctx = event_ctx(&theme);
        assert_eq!(
            view.on_event(&Event::KeyDown(KeyCode::Tab, Modifiers::NONE), &mut event_ctx),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(1) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(1)));

        assert_eq!(
            view.on_event(&Event::KeyDown(KeyCode::Tab, Modifiers::NONE), &mut event_ctx),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(2) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(2)));

        assert_eq!(
            view.on_event(&Event::KeyDown(KeyCode::Tab, Modifiers::NONE), &mut event_ctx),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(1) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(1)));

        assert_eq!(
            view.on_event(
                &Event::KeyDown(KeyCode::Tab, Modifiers { shift: true, ..Modifiers::NONE }),
                &mut event_ctx,
            ),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(2) }))
        );
        assert_eq!(view.focused_id, Some(WidgetId(2)));
    }

    #[test]
    fn form_view_reports_focused_ime_cursor_rect_with_scroll_translation() {
        let text_box_id = WidgetId(404);
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        let mut view = FormView::new(FormViewStyle::default());
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 160.0), &mut layout_ctx);
        view.set_sections(
            vec![
                FormSection::new(
                    fixture_label("General"),
                    Some(fixture_description("Configure this workspace.")),
                    vec![checkbox_row(WidgetId(1)), checkbox_row(WidgetId(2))],
                    FormSectionStyle::default(),
                ),
                FormSection::new(
                    fixture_label("Profile"),
                    Some(fixture_description("Set your display information.")),
                    vec![FormRow::new(
                        fixture_label("Display name"),
                        None,
                        Box::new(TextBox::with_id(text_box_id)),
                        FormRowStyle::default(),
                    )],
                    FormSectionStyle::default(),
                ),
            ],
            &mut layout_ctx,
        );
        view.scroll_offset = view.section_rects[1].y - 32.0;
        view.set_keyboard_focus(Some(text_box_id));

        let mut event_ctx = event_ctx(&theme);
        let _ = view.on_event(
            &Event::ImePreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut event_ctx,
        );

        let section_local_rect = view.sections[1]
            .focused_ime_cursor_rect()
            .expect("focused section should expose a section-local ime cursor rect");
        let view_local_rect = view
            .focused_ime_cursor_rect()
            .expect("focused view should expose a view-local ime cursor rect");
        assert_eq!(
            view_local_rect,
            Rect::new(
                view.section_rects[1].x + section_local_rect.x,
                view.section_rects[1].y - view.scroll_offset + section_local_rect.y,
                section_local_rect.w,
                section_local_rect.h,
            ),
        );
    }

    #[test]
    fn ime_section_routing_reuses_the_original_text_allocation() {
        let event = Event::ImeCommit("form-view-sensitive-ime-route".to_owned());
        let original_allocation = match &event {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("test event is an IME commit"),
        };

        let local_event = FormView::local_event(&event, Rect::new(8.0, 12.0, 100.0, 60.0));
        let local_allocation = match local_event.as_ref() {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("local event must remain an IME commit"),
        };

        assert_eq!(local_allocation, original_allocation);
    }
}
