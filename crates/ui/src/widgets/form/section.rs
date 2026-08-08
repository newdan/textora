use std::any::Any;
use std::borrow::Cow;

use crate::core::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode, DrawCmd, Event, EventCtx,
    LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
};
use crate::widgets::form::row::FormRow;
use crate::widgets::label::Label;

const DEFAULT_FORM_SECTION_TITLE_GAP_LOGICAL: f32 = 8.0;
const DEFAULT_FORM_SECTION_DESCRIPTION_GAP_LOGICAL: f32 = 12.0;
const DEFAULT_FORM_SECTION_ROW_GAP_LOGICAL: f32 = 0.0;
const DEFAULT_FORM_SECTION_CORNER_RADIUS_LOGICAL: f32 = 12.0;
const DEFAULT_FORM_SECTION_BORDER_WIDTH_LOGICAL: f32 = 1.0;
const DEFAULT_FORM_SECTION_TITLE_HEIGHT_LOGICAL: f32 = 20.0;
const DEFAULT_FORM_SECTION_DESCRIPTION_HEIGHT_LOGICAL: f32 = 18.0;
const DEFAULT_FORM_SECTION_ROW_HEIGHT_LOGICAL: f32 = 56.0;
const DEFAULT_FORM_SECTION_SEPARATOR_WIDTH_LOGICAL: f32 = 1.0;
const DEFAULT_FORM_SECTION_SEPARATOR_HORIZONTAL_INSET_LOGICAL: f32 = 0.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FormSectionStyle {
    pub title_gap_logical: f32,
    pub description_gap_logical: f32,
    pub row_gap_logical: f32,
    pub row_height_logical: f32,
    pub corner_radius_logical: f32,
    pub border_width_logical: f32,
}

impl Default for FormSectionStyle {
    fn default() -> Self {
        Self {
            title_gap_logical: DEFAULT_FORM_SECTION_TITLE_GAP_LOGICAL,
            description_gap_logical: DEFAULT_FORM_SECTION_DESCRIPTION_GAP_LOGICAL,
            row_gap_logical: DEFAULT_FORM_SECTION_ROW_GAP_LOGICAL,
            row_height_logical: DEFAULT_FORM_SECTION_ROW_HEIGHT_LOGICAL,
            corner_radius_logical: DEFAULT_FORM_SECTION_CORNER_RADIUS_LOGICAL,
            border_width_logical: DEFAULT_FORM_SECTION_BORDER_WIDTH_LOGICAL,
        }
    }
}

pub struct FormSection {
    rect: Rect,
    title: Label,
    description: Option<Label>,
    rows: Vec<FormRow>,
    style: FormSectionStyle,
    title_rect: Rect,
    description_rect: Option<Rect>,
    content_rect: Rect,
    row_rects: Vec<Rect>,
    focused_id: Option<WidgetId>,
    pointer_row_index: Option<usize>,
    hover_row_index: Option<usize>,
}

impl FormSection {
    pub fn new(
        title: Label,
        description: Option<Label>,
        rows: Vec<FormRow>,
        style: FormSectionStyle,
    ) -> Self {
        Self {
            rect: Rect::ZERO,
            title,
            description,
            rows,
            style,
            title_rect: Rect::ZERO,
            description_rect: None,
            content_rect: Rect::ZERO,
            row_rects: Vec::new(),
            focused_id: None,
            pointer_row_index: None,
            hover_row_index: None,
        }
    }

    pub fn content_height(&self) -> f32 {
        self.content_rect.h
    }

    pub(crate) fn focused_ime_cursor_rect(&self) -> Option<Rect> {
        let row_index = self.focused_row_index()?;
        let row_rect = *self.row_rects.get(row_index)?;
        let ime_rect = self.rows.get(row_index)?.focused_ime_cursor_rect()?;
        Some(Rect::new(row_rect.x + ime_rect.x, row_rect.y + ime_rect.y, ime_rect.w, ime_rect.h))
    }

    fn logical_to_px(value_logical: f32, dpi: f32) -> f32 {
        value_logical * dpi
    }

    fn title_gap_px(&self, dpi: f32) -> f32 {
        Self::logical_to_px(self.style.title_gap_logical, dpi)
    }

    fn description_gap_px(&self, dpi: f32) -> f32 {
        Self::logical_to_px(self.style.description_gap_logical, dpi)
    }

    fn row_gap_px(&self, dpi: f32) -> f32 {
        Self::logical_to_px(self.style.row_gap_logical, dpi)
    }

    fn row_height_px(&self, dpi: f32) -> f32 {
        Self::logical_to_px(self.style.row_height_logical, dpi)
    }

    fn title_height_px(dpi: f32) -> f32 {
        Self::logical_to_px(DEFAULT_FORM_SECTION_TITLE_HEIGHT_LOGICAL, dpi)
    }

    fn description_height_px(dpi: f32) -> f32 {
        Self::logical_to_px(DEFAULT_FORM_SECTION_DESCRIPTION_HEIGHT_LOGICAL, dpi)
    }

    fn content_origin_y_px(&self, dpi: f32) -> f32 {
        let title_height = Self::title_height_px(dpi);
        let title_gap = self.title_gap_px(dpi);

        if self.description.is_some() {
            title_height
                + title_gap
                + Self::description_height_px(dpi)
                + self.description_gap_px(dpi)
        } else {
            title_height + title_gap
        }
    }

    fn rows_height_px(&self, dpi: f32) -> f32 {
        if self.rows.is_empty() {
            return 0.0;
        }

        let row_height = self.row_height_px(dpi);
        let row_gap = self.row_gap_px(dpi);
        let row_count = self.rows.len() as f32;
        let separator_count = row_count - 1.0;
        row_height * row_count + row_gap * separator_count
    }

    fn rects_intersect(first: Rect, second: Rect) -> bool {
        first.x < second.right()
            && first.right() > second.x
            && first.y < second.bottom()
            && first.bottom() > second.y
    }

    pub(crate) fn preferred_height(&self, dpi: f32) -> f32 {
        self.content_origin_y_px(dpi) + self.rows_height_px(dpi)
    }

    pub(crate) fn collect_visible_focusable_ids(
        &self,
        visible_rect: Rect,
        output: &mut Vec<WidgetId>,
    ) {
        if visible_rect.w <= 0.0 || visible_rect.h <= 0.0 {
            return;
        }

        for (row, row_rect) in self.rows.iter().zip(self.row_rects.iter()) {
            if Self::rects_intersect(*row_rect, visible_rect) {
                row.collect_focusable_ids(output);
            }
        }
    }

    pub(crate) fn collect_accessibility_nodes_in_viewport(
        &self,
        context: &AccessibilityContext,
        visible_rect: Rect,
        output: &mut Vec<AccessibilityNode>,
    ) {
        if visible_rect.w <= 0.0 || visible_rect.h <= 0.0 {
            return;
        }

        if Self::rects_intersect(self.title_rect, visible_rect) {
            self.title.collect_accessibility_nodes(
                &context.offset_by(self.title_rect.x, self.title_rect.y),
                output,
            );
        }
        if let (Some(description), Some(description_rect)) =
            (&self.description, self.description_rect)
            && Self::rects_intersect(description_rect, visible_rect)
        {
            description.collect_accessibility_nodes(
                &context.offset_by(description_rect.x, description_rect.y),
                output,
            );
        }
        for (row, row_rect) in self.rows.iter().zip(&self.row_rects) {
            if Self::rects_intersect(*row_rect, visible_rect) {
                row.collect_accessibility_nodes(&context.offset_by(row_rect.x, row_rect.y), output);
            }
        }
    }

    fn local_event<'a>(event: &'a Event, child_rect: Rect) -> Cow<'a, Event> {
        crate::core::dock::Dock::to_local(event, child_rect.x, child_rect.y)
    }

    fn row_index_at(&self, px: f32, py: f32) -> Option<usize> {
        self.row_rects.iter().position(|rect| rect.contains(px, py))
    }

    fn focused_row_index(&self) -> Option<usize> {
        let focused_id = self.focused_id?;
        self.rows.iter().position(|row| {
            let mut ids = Vec::new();
            row.collect_focusable_ids(&mut ids);
            ids.into_iter().any(|id| id == focused_id)
        })
    }

    fn capturing_row_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.is_capturing())
    }

    fn dispatch_to_row(
        &mut self,
        row_index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let row_rect = *self.row_rects.get(row_index)?;
        let local_event = Self::local_event(event, row_rect);
        self.rows.get_mut(row_index)?.on_event(&local_event, ctx)
    }
}

impl Widget for FormSection {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        self.row_rects.clear();

        let title_height = Self::title_height_px(ctx.dpi).min(self.rect.h);
        self.title_rect = Rect::new(0.0, 0.0, self.rect.w, title_height);
        self.title.set_rect(Rect::new(0.0, 0.0, self.title_rect.w, self.title_rect.h), ctx);

        let mut cursor_y = title_height;
        let title_gap = self.title_gap_px(ctx.dpi);
        let description_gap = self.description_gap_px(ctx.dpi);
        let row_gap = self.row_gap_px(ctx.dpi);

        self.description_rect = self.description.as_mut().map(|description| {
            cursor_y += title_gap;
            let description_height =
                Self::description_height_px(ctx.dpi).min((self.rect.h - cursor_y).max(0.0));
            let rect = Rect::new(0.0, cursor_y, self.rect.w, description_height.max(0.0));
            description.set_rect(Rect::new(0.0, 0.0, rect.w, rect.h), ctx);
            cursor_y += description_height;
            rect
        });

        cursor_y += if self.description_rect.is_some() { description_gap } else { title_gap };
        let row_height = self.row_height_px(ctx.dpi);
        let content_height = self.rows_height_px(ctx.dpi);
        self.content_rect = Rect::new(0.0, cursor_y, self.rect.w, content_height);

        let mut row_y = self.content_rect.y;
        for row in &mut self.rows {
            let row_rect = Rect::new(0.0, row_y, self.content_rect.w, row_height);
            row.set_rect(Rect::new(0.0, 0.0, row_rect.w, row_rect.h), ctx);
            self.row_rects.push(row_rect);
            row_y += row_height + row_gap;
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;

        ctx.list.offset = (saved_offset.0 + self.title_rect.x, saved_offset.1 + self.title_rect.y);
        self.title.paint(ctx);

        if let Some(description) = &self.description
            && let Some(description_rect) = self.description_rect
        {
            ctx.list.offset =
                (saved_offset.0 + description_rect.x, saved_offset.1 + description_rect.y);
            description.paint(ctx);
        }

        let settings = ctx.theme.settings_theme();
        let corner_radius = Self::logical_to_px(self.style.corner_radius_logical, ctx.dpi);
        let border_width = Self::logical_to_px(self.style.border_width_logical, ctx.dpi);
        let separator_width =
            Self::logical_to_px(DEFAULT_FORM_SECTION_SEPARATOR_WIDTH_LOGICAL, ctx.dpi);
        let separator_inset =
            Self::logical_to_px(DEFAULT_FORM_SECTION_SEPARATOR_HORIZONTAL_INSET_LOGICAL, ctx.dpi);

        if self.content_rect.w > 0.0 && self.content_rect.h > 0.0 {
            ctx.list.offset = saved_offset;
            ctx.list.fill_rounded(self.content_rect, settings.section_surface, corner_radius);

            let clip_rect = Rect::new(
                self.content_rect.x + saved_offset.0,
                self.content_rect.y + saved_offset.1,
                self.content_rect.w,
                self.content_rect.h,
            );
            ctx.list.cmds.push(DrawCmd::PushClip(clip_rect));
            for (index, row) in self.rows.iter().enumerate() {
                let row_rect = self.row_rects[index];
                ctx.list.offset = (saved_offset.0 + row_rect.x, saved_offset.1 + row_rect.y);
                row.paint(ctx);

                if index + 1 < self.row_rects.len() {
                    let separator_y = row_rect.h - separator_width * 0.5;
                    let separator_rect = Rect::new(
                        separator_inset,
                        separator_y,
                        (row_rect.w - separator_inset * 2.0).max(0.0),
                        separator_width,
                    );
                    if separator_rect.w > 0.0 {
                        ctx.list.stroke(separator_rect, settings.separator, separator_width);
                    }
                }
            }
            ctx.list.offset = saved_offset;
            ctx.list.cmds.push(DrawCmd::PopClip);

            ctx.list.offset = saved_offset;
            ctx.list.stroke_rounded(
                self.content_rect,
                settings.section_border,
                corner_radius,
                border_width,
            );
        }

        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        for row in &self.rows {
            row.collect_focusable_ids(output);
        }
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        for row in &mut self.rows {
            row.set_keyboard_focus(focused_id);
        }
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        self.rows.iter_mut().find_map(|row| row.on_accessibility_action(request))
    }

    fn is_capturing(&self) -> bool {
        self.pointer_row_index.is_some() || self.capturing_row_index().is_some()
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if matches!(event, Event::PointerLeave | Event::InteractionCancel) {
            let container_changed = if matches!(event, Event::InteractionCancel) {
                self.pointer_row_index.take().is_some() | self.hover_row_index.take().is_some()
            } else {
                self.hover_row_index.take().is_some()
            };
            let mut first_action = None;
            for row_index in 0..self.rows.len() {
                if let Some(action) = self.dispatch_to_row(row_index, event, ctx)
                    && first_action.is_none()
                {
                    first_action = Some(action);
                }
            }
            return first_action.or_else(|| container_changed.then_some(WidgetAction::Consumed));
        }

        if let Some(row_index) = self.capturing_row_index()
            && matches!(
                event,
                Event::MouseMove { .. } | Event::MouseUp { .. } | Event::Wheel { .. }
            )
        {
            let action = self.dispatch_to_row(row_index, event, ctx);
            if matches!(event, Event::MouseUp { .. }) {
                self.pointer_row_index = None;
            }
            return action;
        }

        match event {
            Event::MouseDown { px, py, .. } => {
                let row_index = self.row_index_at(*px, *py)?;
                self.pointer_row_index = Some(row_index);
                self.dispatch_to_row(row_index, event, ctx)
            }
            Event::MouseMove { px, py } => {
                if let Some(row_index) = self.pointer_row_index {
                    return self.dispatch_to_row(row_index, event, ctx);
                }

                let next_hover_row_index = self.row_index_at(*px, *py);
                let previous_hover_action = if self.hover_row_index != next_hover_row_index {
                    self.hover_row_index.and_then(|row_index| {
                        let saved_cursor_hint = ctx.cursor_hint;
                        let action = self.dispatch_to_row(row_index, event, ctx);
                        ctx.cursor_hint = saved_cursor_hint;
                        action
                    })
                } else {
                    None
                };
                self.hover_row_index = next_hover_row_index;

                if let Some(row_index) = next_hover_row_index {
                    return self.dispatch_to_row(row_index, event, ctx).or(previous_hover_action);
                }

                previous_hover_action
            }
            Event::MouseUp { .. } => {
                let row_index = self.pointer_row_index.take()?;
                self.dispatch_to_row(row_index, event, ctx)
            }
            Event::Wheel { px, py, .. } => self
                .row_index_at(*px, *py)
                .and_then(|row_index| self.dispatch_to_row(row_index, event, ctx)),
            _ => self
                .focused_row_index()
                .and_then(|row_index| self.dispatch_to_row(row_index, event, ctx)),
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
    use crate::core::widget::{ControlAction, KeyCode, Modifiers, MouseButton};
    use crate::theme::Theme;
    use crate::widgets::checkbox::Checkbox;
    use crate::widgets::form::row::FormRowStyle;
    use crate::widgets::label::{LabelForeground, LabelStyle};
    use crate::widgets::text_box::TextBox;
    use std::any::Any;
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

    fn laid_out_section(row_count: usize) -> FormSection {
        let mut section = FormSection::new(
            fixture_label("General"),
            Some(fixture_description("Configure this workspace.")),
            (0..row_count).map(|index| checkbox_row(WidgetId(index as u64 + 1))).collect(),
            FormSectionStyle::default(),
        );
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut ctx = layout_ctx(&theme, &mut measure);
        section.set_rect(Rect::new(0.0, 0.0, 720.0, 320.0), &mut ctx);
        section
    }

    fn paint_for_test(section: &FormSection) -> DrawList {
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
        section.paint(&mut paint_ctx);
        draw_list
    }

    fn count_surface_fills(draw_list: &DrawList) -> usize {
        let theme = crate::theme::test_theme();
        let settings = theme.settings_theme();
        draw_list
            .cmds
            .iter()
            .filter(|cmd| {
                matches!(
                    cmd,
                    DrawCmd::FillRect { color, radius, .. }
                    if *color == settings.section_surface && *radius > 0.0
                )
            })
            .count()
    }

    fn count_separator_strokes(draw_list: &DrawList) -> usize {
        draw_list
            .cmds
            .iter()
            .filter(|cmd| matches!(cmd, DrawCmd::StrokeRect { radius, .. } if *radius == 0.0))
            .count()
    }

    fn separator_stroke_rects(draw_list: &DrawList) -> Vec<Rect> {
        draw_list
            .cmds
            .iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::StrokeRect { rect, radius, .. } if *radius == 0.0 => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn form_section_paints_one_surface_and_internal_separators() {
        let section = laid_out_section(3);
        let draw = paint_for_test(&section);
        assert_eq!(count_surface_fills(&draw), 1);
        assert_eq!(count_separator_strokes(&draw), 2);
        let clip_start = draw
            .cmds
            .iter()
            .position(|cmd| matches!(cmd, DrawCmd::PushClip(_)))
            .expect("section should push clip before content");
        let clip_end = draw
            .cmds
            .iter()
            .position(|cmd| matches!(cmd, DrawCmd::PopClip))
            .expect("section should pop clip after content");
        assert!(clip_start < clip_end, "clip commands should be balanced and ordered");
    }

    #[test]
    fn form_section_separator_rects_stay_within_clip_and_align_to_row_bottoms() {
        let section = laid_out_section(3);
        let draw = paint_for_test(&section);
        let clip_rect = draw
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                DrawCmd::PushClip(rect) => Some(*rect),
                _ => None,
            })
            .expect("section should push a clip rect for row content");
        let separator_rects = separator_stroke_rects(&draw);

        assert_eq!(separator_rects.len(), 2, "three rows should emit two separators");

        for (index, separator_rect) in separator_rects.iter().enumerate() {
            let row_rect = section.row_rects[index];
            assert!(
                separator_rect.x >= clip_rect.x,
                "separator {index} should stay inside clip left edge"
            );
            assert!(
                separator_rect.right() <= clip_rect.right(),
                "separator {index} should stay inside clip right edge"
            );
            assert!(
                separator_rect.y >= clip_rect.y,
                "separator {index} should stay inside clip top edge"
            );
            assert!(
                separator_rect.bottom() <= clip_rect.bottom(),
                "separator {index} should stay inside clip bottom edge"
            );
            assert_eq!(
                separator_rect.y + separator_rect.h * 0.5,
                row_rect.bottom(),
                "separator {index} should align to row bottom"
            );
        }
    }

    #[test]
    fn form_section_reports_content_height() {
        let section = laid_out_section(2);
        assert_eq!(section.content_height(), 112.0);
    }

    #[test]
    fn form_section_preferred_height_depends_on_description_presence() {
        let with_description = FormSection::new(
            fixture_label("General"),
            Some(fixture_description("Configure this workspace.")),
            vec![checkbox_row(WidgetId(1))],
            FormSectionStyle::default(),
        );
        let without_description = FormSection::new(
            fixture_label("Editor"),
            None,
            vec![checkbox_row(WidgetId(2))],
            FormSectionStyle::default(),
        );

        assert_eq!(with_description.preferred_height(1.0), 114.0);
        assert_eq!(without_description.preferred_height(1.0), 84.0);
    }

    #[test]
    fn form_section_forwards_row_actions_unchanged() {
        let mut section = laid_out_section(1);
        let theme = crate::theme::test_theme();
        let mut ctx = event_ctx(&theme);
        let row_rect = section.row_rects[0];
        let action = section.on_event(
            &Event::MouseDown {
                px: row_rect.right() - 16.0,
                py: row_rect.y + 16.0,
                button: MouseButton::Left,
            },
            &mut ctx,
        );
        assert_eq!(
            action,
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: WidgetId(1) }))
        );
    }

    #[test]
    fn form_section_delegates_focus_ids_and_local_coordinates_to_rows() {
        let forwarded_action =
            WidgetAction::Control(ControlAction::Toggled { id: WidgetId(77), checked: true });
        let tracking_state = Rc::new(RefCell::new(TrackingState::default()));
        let mut section = FormSection::new(
            fixture_label("Automation"),
            Some(fixture_description("Choose how updates are applied.")),
            vec![FormRow::new(
                fixture_label("Install automatically"),
                None,
                Box::new(TrackingControl::new(
                    WidgetId(77),
                    tracking_state.clone(),
                    Some(forwarded_action.clone()),
                )),
                FormRowStyle::default(),
            )],
            FormSectionStyle::default(),
        );
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        section.set_rect(Rect::new(0.0, 0.0, 720.0, 220.0), &mut layout_ctx);

        let mut ids = Vec::new();
        section.collect_focusable_ids(&mut ids);
        assert_eq!(ids, vec![WidgetId(77)]);

        section.set_keyboard_focus(Some(WidgetId(77)));

        let row_rect = section.row_rects[0];
        let control_rect = section.rows[0].control_rect();
        let control_local_x = 24.0;
        let control_local_y = 14.0;
        let mut event_ctx = event_ctx(&theme);
        let action = section.on_event(
            &Event::MouseDown {
                px: row_rect.x + control_rect.x + control_local_x,
                py: row_rect.y + control_rect.y + control_local_y,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        );
        assert_eq!(action, Some(forwarded_action.clone()));

        let key_action =
            section.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx);
        assert_eq!(key_action, Some(forwarded_action));

        let tracking = tracking_state.borrow();
        assert_eq!(tracking.focused_id, Some(WidgetId(77)));
        assert_eq!(
            tracking.events,
            vec![
                Event::MouseDown {
                    px: control_local_x,
                    py: control_local_y,
                    button: MouseButton::Left,
                },
                Event::KeyDown(KeyCode::Enter, Modifiers::NONE),
            ],
        );
    }

    #[test]
    fn form_section_reports_focused_row_ime_cursor_rect_with_row_offset() {
        let text_box_id = WidgetId(404);
        let mut section = FormSection::new(
            fixture_label("Profile"),
            Some(fixture_description("Set your display information.")),
            vec![
                checkbox_row(WidgetId(1)),
                FormRow::new(
                    fixture_label("Display name"),
                    None,
                    Box::new(TextBox::with_id(text_box_id)),
                    FormRowStyle::default(),
                ),
            ],
            FormSectionStyle::default(),
        );
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx = layout_ctx(&theme, &mut measure);
        section.set_rect(Rect::new(0.0, 0.0, 720.0, 320.0), &mut layout_ctx);

        assert_eq!(section.focused_ime_cursor_rect(), None);

        section.set_keyboard_focus(Some(text_box_id));

        let row_index = 1;
        let row_rect = section.row_rects[row_index];
        let mut event_ctx = event_ctx(&theme);
        let _ = section.on_event(
            &Event::ImePreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut event_ctx,
        );

        let row_local_rect = section.rows[row_index]
            .focused_ime_cursor_rect()
            .expect("focused row should expose a row-local ime cursor rect");
        let section = section;
        let section_local_rect = section
            .focused_ime_cursor_rect()
            .expect("focused row should expose an ime cursor rect");
        assert_eq!(
            section_local_rect,
            Rect::new(
                row_rect.x + row_local_rect.x,
                row_rect.y + row_local_rect.y,
                row_local_rect.w,
                row_local_rect.h,
            ),
        );
    }

    #[test]
    fn form_section_returns_none_when_focus_is_missing_or_not_a_text_box() {
        let checkbox_id = WidgetId(1);
        let mut section = laid_out_section(2);

        let section = section;
        assert_eq!(section.focused_ime_cursor_rect(), None);

        let mut section = section;
        section.set_keyboard_focus(Some(checkbox_id));

        let section = section;
        assert_eq!(section.focused_ime_cursor_rect(), None);
    }

    #[test]
    fn ime_row_routing_reuses_the_original_text_allocation() {
        let event = Event::ImeCommit("form-section-sensitive-ime-route".to_owned());
        let original_allocation = match &event {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("test event is an IME commit"),
        };

        let local_event = FormSection::local_event(&event, Rect::new(8.0, 12.0, 100.0, 60.0));
        let local_allocation = match local_event.as_ref() {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("local event must remain an IME commit"),
        };

        assert_eq!(local_allocation, original_allocation);
    }

    #[test]
    fn form_section_collects_only_focusable_ids_for_rows_intersecting_visible_rect() {
        let section = laid_out_section(4);
        let local_visible_rect = Rect::new(0.0, 100.0, 720.0, 20.0);
        let mut ids = Vec::new();

        section.collect_visible_focusable_ids(local_visible_rect, &mut ids);

        assert_eq!(ids, vec![WidgetId(1), WidgetId(2)]);
    }
}
