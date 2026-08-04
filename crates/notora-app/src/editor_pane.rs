//! Notora 编辑区 chrome 的组合边界。
//!
//! 这里把产品层的文档状态映射成通用 UI widget 输入，并集中管理头部、属性
//! 弹层和编辑器菜单的命中顺序。正文 runtime 不属于这个组合器。

use ui::core::widget::{ControlAction, TextPayload, WidgetAction};
use ui::editor_header::{EditorHeaderInput, EditorHeaderWidget};
use ui::editor_toolbar::{EditorToolbarInput, EditorToolbarWidget};
use ui::location_picker::{LocationPickerInput, LocationPickerWidget};
use ui::tag_editor::{TagEditorInput, TagEditorWidget};
use ui::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget};

const EDITOR_PANE_TAG_ROW_HEIGHT_LOGICAL: f32 = 28.0;
const EDITOR_PANE_TAG_HORIZONTAL_INSET_LOGICAL: f32 = 16.0;
const EDITOR_PANE_LOCATION_WIDTH_LOGICAL: f32 = 360.0;
const EDITOR_PANE_LOCATION_HEIGHT_LOGICAL: f32 = 220.0;
const EDITOR_PANE_LOCATION_INSET_LOGICAL: f32 = 16.0;
const EDITOR_PANE_COMPACT_DOCUMENT_HEADER_HEIGHT_LOGICAL: f32 = 72.0;
const EDITOR_PANE_COMPACT_HEADER_WIDTH_LOGICAL: f32 = 420.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorPaneMode {
    #[default]
    Empty,
    WorkspaceNote,
    ExternalFile,
    TrashNote,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorPaneInput {
    pub mode: EditorPaneMode,
    pub document_key: String,
    pub header: EditorHeaderInput,
    pub location: LocationPickerInput,
    pub tags: TagEditorInput,
    pub toolbar: EditorToolbarInput,
}

impl EditorPaneInput {
    pub fn effective(&self) -> Self {
        let mut effective = self.clone();
        match self.mode {
            EditorPaneMode::Empty => {
                effective.header = EditorHeaderInput::default();
                effective.location = LocationPickerInput::default();
                effective.tags = TagEditorInput::default();
                effective.toolbar = EditorToolbarInput::default();
            }
            EditorPaneMode::WorkspaceNote => {}
            EditorPaneMode::ExternalFile => {
                effective.header.title_editable = false;
                effective.header.starred = false;
                effective.header.star_enabled = false;
                effective.header.encryption = ui::editor_header::EncryptionStatusInput::Hidden;
                effective.header.delete_visible = false;
                effective.header.delete_enabled = false;
                effective.location = LocationPickerInput::default();
                effective.tags = TagEditorInput::default();
            }
            EditorPaneMode::TrashNote => {
                effective.header.title_editable = false;
                effective.header.starred = false;
                effective.header.star_enabled = false;
                effective.header.delete_visible = false;
                effective.header.delete_enabled = false;
                effective.location = LocationPickerInput::default();
                effective.tags = TagEditorInput::default();
            }
        }
        effective
    }

    pub fn should_render_chrome(&self) -> bool {
        self.mode != EditorPaneMode::Empty
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditorPaneRects {
    pub header: Rect,
    pub toolbar: Rect,
    pub body: Rect,
}

pub struct EditorPaneChrome {
    input: EditorPaneInput,
    header: EditorHeaderWidget,
    location_picker: LocationPickerWidget,
    tag_editor: TagEditorWidget,
    toolbar: EditorToolbarWidget,
    rects: EditorPaneRects,
    document_header_rect: Rect,
    tag_rect: Rect,
    location_rect: Rect,
    tag_editor_active: bool,
}

impl EditorPaneChrome {
    pub fn new() -> Self {
        Self {
            input: EditorPaneInput::default(),
            header: EditorHeaderWidget::new(),
            location_picker: LocationPickerWidget::new(),
            tag_editor: TagEditorWidget::new(),
            toolbar: EditorToolbarWidget::new(),
            rects: EditorPaneRects::default(),
            document_header_rect: Rect::ZERO,
            tag_rect: Rect::ZERO,
            location_rect: Rect::ZERO,
            tag_editor_active: false,
        }
    }

    pub fn set_input(&mut self, input: EditorPaneInput) {
        let effective_input = input.effective();
        if self.input.document_key != effective_input.document_key {
            self.tag_editor_active = false;
        }
        self.input = effective_input;
        self.header.set_input(self.input.header.clone());
        self.location_picker.set_input(self.input.location.clone());
        self.tag_editor.set_input(self.input.tags.clone());
        self.toolbar.set_input(self.input.toolbar.clone());
        if !self.input.should_render_chrome() {
            self.tag_editor_active = false;
        }
    }

    pub fn set_rects(&mut self, rects: EditorPaneRects, context: &mut LayoutCtx<'_>) {
        self.rects = rects;
        self.document_header_rect = document_header_rect(rects.header, context.dpi);
        let compact = rects.header.w / context.dpi <= EDITOR_PANE_COMPACT_HEADER_WIDTH_LOGICAL
            || self.document_header_rect.h / context.dpi
                <= EDITOR_PANE_COMPACT_DOCUMENT_HEADER_HEIGHT_LOGICAL;
        self.input.header.compact = compact;
        self.input.tags.compact = compact;
        self.header.set_input(self.input.header.clone());
        self.tag_editor.set_input(self.input.tags.clone());
        self.tag_rect = tag_rect(rects.header, context.dpi, self.input.tags.enabled);
        self.location_rect = location_rect(rects.header, context.dpi, self.input.location.open);
        self.header.set_rect(local_rect(self.document_header_rect), context);
        self.toolbar.set_rect(local_rect(rects.toolbar), context);
        self.tag_editor.set_rect(local_rect(self.tag_rect), context);
        self.location_picker.set_rect(local_rect(self.location_rect), context);
    }

    pub fn has_open_property_popup(&self) -> bool {
        self.input.location.open || self.input.tags.suggestions_open
    }

    pub fn set_title_focus(&mut self, focused: bool) {
        let focused_id = focused.then_some(ui::editor_header::EDITOR_HEADER_TITLE_ID);
        self.header.set_keyboard_focus(focused_id);
    }

    pub fn set_title_blink_visible(&mut self, visible: bool) {
        self.header.set_title_blink_visible(visible);
    }

    pub fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<WidgetAction> {
        if !self.input.should_render_chrome() {
            return None;
        }

        if self.input.location.open {
            let local_event = translate_event(event, self.location_rect.x, self.location_rect.y);
            if let Some(action) = self.location_picker.on_event(&local_event, context) {
                self.close_location_after_action(&action);
                return Some(action);
            }
        }

        if self.input.tags.suggestions_open || self.tag_editor_active {
            let local_event = translate_event(event, self.tag_rect.x, self.tag_rect.y);
            if let Some(action) = self.tag_editor.on_event(&local_event, context) {
                let dismissed = is_tag_dismiss_action(&action);
                self.close_tag_after_action(&action);
                if !dismissed {
                    return Some(action);
                }
            }
            if self.tag_editor_active || self.input.tags.suggestions_open {
                return Some(WidgetAction::Consumed);
            }
        } else if event_is_inside(event, self.tag_rect) {
            self.tag_editor_active = true;
            let local_event = translate_event(event, self.tag_rect.x, self.tag_rect.y);
            if let Some(action) = self.tag_editor.on_event(&local_event, context) {
                self.close_tag_after_action(&action);
                return Some(action);
            }
            return Some(WidgetAction::Consumed);
        }

        if event_is_inside(event, self.document_header_rect)
            || (self.header.title_is_focused() && event_is_keyboard(event))
        {
            let local_event =
                translate_event(event, self.document_header_rect.x, self.document_header_rect.y);
            if let Some(action) = self.header.on_event(&local_event, context) {
                return Some(action);
            }
        }

        if event_is_inside_or_keyboard(event, self.rects.toolbar) {
            let local_event = translate_event(event, self.rects.toolbar.x, self.rects.toolbar.y);
            return self.toolbar.on_event(&local_event, context);
        }
        None
    }

    pub fn paint_underlay(&self, context: &mut PaintCtx<'_>) {
        if !self.input.should_render_chrome() {
            return;
        }
        paint_at(context, self.document_header_rect, |context| self.header.paint(context));
        if self.input.tags.enabled {
            paint_at(context, self.tag_rect, |context| self.tag_editor.paint(context));
        }
        paint_at(context, self.rects.toolbar, |context| self.toolbar.paint(context));
    }

    pub fn paint_overlay(&self, context: &mut PaintCtx<'_>) {
        if self.input.should_render_chrome() && self.input.location.open {
            paint_at(context, self.location_rect, |context| self.location_picker.paint(context));
        }
    }

    fn close_location_after_action(&mut self, action: &WidgetAction) {
        let WidgetAction::Control(control) = action else {
            return;
        };
        match control {
            ControlAction::Activated { id }
                if *id == ui::location_picker::LOCATION_PICKER_CANCEL_ID
                    || *id == ui::location_picker::LOCATION_PICKER_DISMISS_ID =>
            {
                self.input.location.open = false;
            }
            ControlAction::TextEdited {
                id: ui::location_picker::LOCATION_PICKER_TOGGLE_ID,
                value: TextPayload::Plain(row_key),
            } => {
                if let Some(row) =
                    self.input.location.directories.iter_mut().find(|row| row.row_key == *row_key)
                {
                    row.expanded = !row.expanded;
                    self.location_picker.set_input(self.input.location.clone());
                }
            }
            _ => {}
        }
    }

    fn close_tag_after_action(&mut self, action: &WidgetAction) {
        let WidgetAction::Control(ControlAction::Activated { id }) = action else {
            return;
        };
        if *id == ui::tag_editor::TAG_EDITOR_CANCEL_ID
            || *id == ui::tag_editor::TAG_EDITOR_DISMISS_ID
        {
            self.tag_editor_active = false;
            self.input.tags.suggestions_open = false;
        }
    }
}

impl Default for EditorPaneChrome {
    fn default() -> Self {
        Self::new()
    }
}

fn local_rect(rect: Rect) -> Rect {
    Rect::new(0.0, 0.0, rect.w, rect.h)
}

fn document_header_rect(header: Rect, dpi: f32) -> Rect {
    let tag_row_height = (EDITOR_PANE_TAG_ROW_HEIGHT_LOGICAL * dpi).min(header.h);
    Rect::new(header.x, header.y, header.w, (header.h - tag_row_height).max(0.0))
}

fn tag_rect(header: Rect, dpi: f32, visible: bool) -> Rect {
    if !visible || header.w <= 0.0 || header.h <= 0.0 {
        return Rect::ZERO;
    }
    let height = (EDITOR_PANE_TAG_ROW_HEIGHT_LOGICAL * dpi).min(header.h);
    Rect::new(
        header.x + EDITOR_PANE_TAG_HORIZONTAL_INSET_LOGICAL * dpi,
        header.bottom() - height,
        (header.w - EDITOR_PANE_TAG_HORIZONTAL_INSET_LOGICAL * dpi * 2.0).max(0.0),
        height,
    )
}

fn is_tag_dismiss_action(action: &WidgetAction) -> bool {
    matches!(
        action,
        WidgetAction::Control(ControlAction::Activated {
            id: ui::tag_editor::TAG_EDITOR_DISMISS_ID
        })
    )
}

fn location_rect(header: Rect, dpi: f32, open: bool) -> Rect {
    if !open || header.w <= 0.0 || header.h <= 0.0 {
        return Rect::ZERO;
    }
    Rect::new(
        header.x + EDITOR_PANE_LOCATION_INSET_LOGICAL * dpi,
        header.y + EDITOR_PANE_LOCATION_INSET_LOGICAL * dpi,
        (EDITOR_PANE_LOCATION_WIDTH_LOGICAL * dpi).min(header.w),
        EDITOR_PANE_LOCATION_HEIGHT_LOGICAL * dpi,
    )
}

fn paint_at(context: &mut PaintCtx<'_>, rect: Rect, paint: impl FnOnce(&mut PaintCtx<'_>)) {
    let saved_offset = context.list.offset;
    context.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
    paint(context);
    context.list.offset = saved_offset;
}

fn event_is_inside(event: &Event, rect: Rect) -> bool {
    match event {
        Event::MouseMove { px, py }
        | Event::MouseDown { px, py, .. }
        | Event::MouseUp { px, py, .. }
        | Event::Wheel { px, py, .. } => rect.contains(*px, *py),
        _ => false,
    }
}

fn event_is_inside_or_keyboard(event: &Event, rect: Rect) -> bool {
    event_is_inside(event, rect) || event_is_keyboard(event)
}

fn event_is_keyboard(event: &Event) -> bool {
    matches!(
        event,
        Event::KeyDown(..)
            | Event::ImePreedit { .. }
            | Event::ImeCommit(_)
            | Event::ImeEnable
            | Event::ImeDisable
    )
}

fn translate_event(event: &Event, offset_x: f32, offset_y: f32) -> Event {
    match event {
        Event::MouseMove { px, py } => Event::MouseMove { px: *px - offset_x, py: *py - offset_y },
        Event::MouseDown { px, py, button } => {
            Event::MouseDown { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::MouseUp { px, py, button } => {
            Event::MouseUp { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::Wheel { dx, dy, px, py } => {
            Event::Wheel { dx: *dx, dy: *dy, px: *px - offset_x, py: *py - offset_y }
        }
        Event::KeyDown(key, modifiers) => Event::KeyDown(*key, *modifiers),
        Event::ImePreedit { text, cursor } => {
            Event::ImePreedit { text: text.clone(), cursor: *cursor }
        }
        Event::ImeCommit(text) => Event::ImeCommit(text.clone()),
        Event::ImeEnable => Event::ImeEnable,
        Event::ImeDisable => Event::ImeDisable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::editor_header::{EditorHeaderInput, EncryptionStatusInput};
    use ui::editor_toolbar::EditorToolbarInput;
    use ui::location_picker::LocationPickerInput;
    use ui::tag_editor::TagEditorInput;

    fn input(mode: EditorPaneMode) -> EditorPaneInput {
        EditorPaneInput {
            mode,
            document_key: "test-document".to_owned(),
            header: EditorHeaderInput {
                title: "路线图".to_owned(),
                title_editable: true,
                created_at_text: "创建 08-03".to_owned(),
                modified_at_text: "修改刚刚".to_owned(),
                save_status_text: "已保存".to_owned(),
                starred: true,
                star_enabled: true,
                encryption: EncryptionStatusInput::Unencrypted,
                delete_visible: true,
                delete_enabled: true,
                compact: false,
            },
            location: LocationPickerInput {
                workspace_name: "Notora".to_owned(),
                current_relative_path: "notes".to_owned(),
                ..LocationPickerInput::default()
            },
            tags: TagEditorInput { enabled: true, ..TagEditorInput::default() },
            toolbar: EditorToolbarInput::default(),
        }
    }

    #[test]
    fn pane_modes_expose_only_the_operations_allowed_by_document_kind() {
        let empty = input(EditorPaneMode::Empty).effective();
        assert_eq!(empty.mode, EditorPaneMode::Empty);
        assert!(!empty.should_render_chrome());

        let note = input(EditorPaneMode::WorkspaceNote).effective();
        assert!(note.should_render_chrome());
        assert!(note.header.title_editable);
        assert!(note.header.star_enabled);
        assert!(note.tags.enabled);

        let external = input(EditorPaneMode::ExternalFile).effective();
        assert!(external.should_render_chrome());
        assert!(!external.header.title_editable);
        assert!(!external.header.star_enabled);
        assert_eq!(external.header.encryption, EncryptionStatusInput::Hidden);
        assert!(!external.header.delete_visible);
        assert!(!external.tags.enabled);
        assert!(external.location.current_relative_path.is_empty());

        let trash = input(EditorPaneMode::TrashNote).effective();
        assert!(trash.should_render_chrome());
        assert!(!trash.header.title_editable);
        assert!(!trash.header.star_enabled);
        assert!(!trash.tags.enabled);
    }

    #[test]
    fn an_empty_pane_does_not_leave_popup_hit_targets() {
        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(input(EditorPaneMode::Empty));
        let theme = ui::theme::test_theme();
        let mut event_context = ui::EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert!(!chrome.has_open_property_popup());
        assert_eq!(
            chrome.route_event(
                &ui::Event::MouseDown { px: 40.0, py: 40.0, button: ui::MouseButton::Left },
                &mut event_context,
            ),
            None
        );
    }

    #[test]
    fn closing_a_location_popup_removes_its_next_event_hit_target() {
        let mut pane_input = input(EditorPaneMode::WorkspaceNote);
        pane_input.location.open = true;
        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(pane_input);
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        chrome.set_rects(
            EditorPaneRects {
                header: Rect::new(100.0, 20.0, 640.0, 128.0),
                toolbar: Rect::new(100.0, 148.0, 640.0, 40.0),
                body: Rect::new(100.0, 188.0, 640.0, 400.0),
            },
            &mut layout_context,
        );
        let mut event_context = ui::EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
        let outside_click =
            ui::Event::MouseDown { px: 900.0, py: 500.0, button: ui::MouseButton::Left };

        assert!(chrome.route_event(&outside_click, &mut event_context).is_some());
        assert!(!chrome.has_open_property_popup());
        assert!(chrome.route_event(&outside_click, &mut event_context).is_none());
    }

    #[test]
    fn compact_geometry_is_forwarded_to_header_and_tag_widgets() {
        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(input(EditorPaneMode::WorkspaceNote));
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };

        chrome.set_rects(
            EditorPaneRects {
                header: Rect::new(0.0, 0.0, 320.0, 56.0),
                toolbar: Rect::new(0.0, 56.0, 320.0, 40.0),
                body: Rect::new(0.0, 96.0, 320.0, 200.0),
            },
            &mut layout_context,
        );

        assert!(chrome.input.header.compact);
        assert!(chrome.input.tags.compact);
    }

    #[test]
    fn tag_editor_uses_a_dedicated_row_below_the_document_header() {
        let tags = tag_rect(Rect::new(0.0, 0.0, 640.0, 108.0), 1.0, true);

        assert_eq!(tags.y, 80.0);
        assert_eq!(tags.h, EDITOR_PANE_TAG_ROW_HEIGHT_LOGICAL);
    }

    #[test]
    fn dismissing_the_tag_editor_does_not_consume_the_body_click() {
        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(input(EditorPaneMode::WorkspaceNote));
        chrome.tag_editor_active = true;
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        chrome.set_rects(
            EditorPaneRects {
                header: Rect::new(0.0, 0.0, 640.0, 108.0),
                toolbar: Rect::new(0.0, 108.0, 640.0, 40.0),
                body: Rect::new(0.0, 148.0, 640.0, 400.0),
            },
            &mut layout_context,
        );
        let mut event_context = ui::EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
        let body_click =
            ui::Event::MouseDown { px: 320.0, py: 240.0, button: ui::MouseButton::Left };

        assert_eq!(chrome.route_event(&body_click, &mut event_context), None);
        assert!(!chrome.tag_editor_active);
    }

    #[test]
    fn switching_document_keys_clears_pending_tag_input_state() {
        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(input(EditorPaneMode::WorkspaceNote));
        chrome.tag_editor_active = true;

        let mut next_input = input(EditorPaneMode::WorkspaceNote);
        next_input.document_key = "next-document".to_owned();
        chrome.set_input(next_input);

        assert!(!chrome.tag_editor_active);
    }

    #[test]
    fn product_focus_request_can_enter_and_leave_title_editing() {
        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(input(EditorPaneMode::WorkspaceNote));

        chrome.set_title_focus(true);
        assert!(chrome.header.title_is_focused());

        chrome.set_title_focus(false);
        assert!(!chrome.header.title_is_focused());
    }

    #[test]
    fn pane_forwards_title_blink_visibility_to_the_header() {
        use ui::core::paint::{DrawCmd, DrawList};

        let mut chrome = EditorPaneChrome::new();
        chrome.set_input(input(EditorPaneMode::WorkspaceNote));
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        chrome.set_rects(
            EditorPaneRects {
                header: Rect::new(0.0, 0.0, 640.0, 108.0),
                toolbar: Rect::new(0.0, 108.0, 640.0, 40.0),
                body: Rect::new(0.0, 148.0, 640.0, 400.0),
            },
            &mut layout_context,
        );
        chrome.set_title_focus(true);

        chrome.set_title_blink_visible(true);
        let mut visible_draw_list = DrawList::new();
        let mut visible_paint_context = ui::PaintCtx::new(&mut visible_draw_list, &theme, 1.0);
        chrome.paint_underlay(&mut visible_paint_context);
        assert!(visible_draw_list.cmds.iter().any(|command| is_caret_command(command, &theme)));

        chrome.set_title_blink_visible(false);
        let mut hidden_draw_list = DrawList::new();
        let mut hidden_paint_context = ui::PaintCtx::new(&mut hidden_draw_list, &theme, 1.0);
        chrome.paint_underlay(&mut hidden_paint_context);
        assert!(!hidden_draw_list.cmds.iter().any(|command| is_caret_command(command, &theme)));

        fn is_caret_command(command: &DrawCmd, theme: &ui::Theme) -> bool {
            matches!(
                command,
                DrawCmd::FillRect { rect, color, radius }
                    if *radius == 0.0
                        && rect.w == 2.0
                        && *color == theme.palette.input_fg
            )
        }
    }
}
