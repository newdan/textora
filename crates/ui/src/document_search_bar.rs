//! 当前文档普通文本查找栏，所有几何使用屏幕像素坐标。
use crate::button::{Button, ButtonStyle};
use crate::core::widget::{ControlAction, TextPayload};
use crate::text_box::TextBox;
use crate::{Event, EventCtx, KeyCode, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId};
use std::time::Instant;

pub const DOCUMENT_SEARCH_BAR_HEIGHT_LOGICAL: f32 = crate::constants::BAR_HEIGHT;
const QUERY_ID: WidgetId = WidgetId(0x646f_6373_6561_7271);
const PREV_ID: WidgetId = WidgetId(0x646f_6373_6561_7270);
const NEXT_ID: WidgetId = WidgetId(0x646f_6373_6561_726e);
const CLOSE_ID: WidgetId = WidgetId(0x646f_6373_6561_7263);
const CONTROL_WIDTH_LOGICAL: f32 = 24.0;
const COUNT_WIDTH_LOGICAL: f32 = 60.0;
const BASELINE_CENTER_SHIFT: f32 = 0.35;

#[derive(Clone, Debug, Default)]
pub struct DocumentSearchBarInput {
    pub query: String,
    pub match_count: usize,
    pub current_match: usize,
    pub visible: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentSearchBarAction {
    QueryChanged(String),
    Next,
    Prev,
    Close,
    FocusRequested,
    AppearanceChanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentSearchBarEvent {
    pub consumed: bool,
    pub action: Option<DocumentSearchBarAction>,
}

pub struct DocumentSearchBarWidget {
    rect: Rect,
    input: DocumentSearchBarInput,
    query_box: TextBox,
    controls: [Button; 3],
    count_rect: Rect,
}

impl Default for DocumentSearchBarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentSearchBarWidget {
    pub fn new() -> Self {
        let mut query_box = TextBox::with_id(QUERY_ID);
        query_box.set_placeholder("在文档中查找…");
        query_box.set_accessibility_label(Some("文档查找".to_owned()));
        let theme = crate::theme::test_theme();
        let controls = [
            (PREV_ID, "◀", "上一个匹配"),
            (NEXT_ID, "▶", "下一个匹配"),
            (CLOSE_ID, "✕", "关闭查找"),
        ]
        .map(|(id, glyph, label)| {
            let mut button = Button::new(id, ButtonStyle::ghost(theme.settings_theme()));
            button.set_text(Some(glyph.to_owned()));
            button.set_accessibility_label(Some(label.to_owned()));
            button
        });
        Self {
            rect: Rect::ZERO,
            input: DocumentSearchBarInput::default(),
            query_box,
            controls,
            count_rect: Rect::ZERO,
        }
    }

    pub fn set_input(&mut self, input: DocumentSearchBarInput) {
        self.query_box.sync_text(&input.query);
        self.query_box.set_focus(input.visible && input.focused);
        if !input.visible {
            let theme = crate::theme::test_theme();
            let mut context = EventCtx::new(&theme, 1.0);
            for button in &mut self.controls {
                button.on_event(&Event::InteractionCancel, &mut context);
            }
        }
        for button in &mut self.controls[..2] {
            button.set_enabled(input.match_count > 0);
        }
        self.input = input;
    }

    pub fn select_query(&mut self) {
        self.query_box.select_all();
    }
    pub fn ime_allowed(&self) -> bool {
        self.input.visible && self.input.focused && self.query_box.ime_allowed()
    }
    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        self.ime_allowed().then(|| self.query_box.ime_cursor_rect())
    }
    pub fn next_cursor_blink_at(&self) -> Option<Instant> {
        self.query_box.next_cursor_blink_at()
    }
    pub fn advance_cursor_blink(&mut self, now: Instant) -> bool {
        self.query_box.advance_cursor_blink(now)
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        self.input.visible && self.rect.contains(px, py)
    }

    fn count_text(&self) -> String {
        if self.input.query.is_empty() {
            return String::new();
        }
        let current = if self.input.match_count == 0 {
            0
        } else {
            self.input.current_match.saturating_add(1).min(self.input.match_count)
        };
        format!("{current}/{}", self.input.match_count)
    }

    fn translate(&mut self, action: WidgetAction) -> DocumentSearchBarAction {
        match action {
            WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Plain(query),
                ..
            }) => {
                self.input.query = query.clone();
                DocumentSearchBarAction::QueryChanged(query)
            }
            WidgetAction::Control(ControlAction::FocusRequested { .. }) => {
                DocumentSearchBarAction::FocusRequested
            }
            WidgetAction::Control(ControlAction::Activated { id: PREV_ID }) => {
                DocumentSearchBarAction::Prev
            }
            WidgetAction::Control(ControlAction::Activated { id: NEXT_ID }) => {
                DocumentSearchBarAction::Next
            }
            WidgetAction::Control(ControlAction::Activated { id: CLOSE_ID }) => {
                DocumentSearchBarAction::Close
            }
            _ => DocumentSearchBarAction::AppearanceChanged,
        }
    }

    pub fn route_event(&mut self, event: &Event, context: &mut EventCtx) -> DocumentSearchBarEvent {
        if !self.input.visible {
            return DocumentSearchBarEvent::default();
        }
        if matches!(
            event,
            Event::KeyDown(..)
                | Event::ImePreedit { .. }
                | Event::ImeCommit(..)
                | Event::ImeEnable
                | Event::ImeDisable
        ) {
            return self.route_keyboard(event, context);
        }
        if self.query_box.is_capturing()
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            let action = self.query_box.on_event(event, context);
            return DocumentSearchBarEvent {
                consumed: true,
                action: action.map(|action| self.translate(action)),
            };
        }
        let mut child_action = None;
        for button in &mut self.controls {
            if let Some(action) = button.on_event(event, context) {
                child_action = Some(action);
            }
        }
        if child_action.is_none() {
            child_action = self.query_box.on_event(event, context);
        }
        let inside = match event {
            Event::MouseDown { px, py, .. }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => self.contains(*px, *py),
            _ => false,
        };
        DocumentSearchBarEvent {
            consumed: inside || child_action.is_some(),
            action: child_action.map(|action| self.translate(action)),
        }
    }

    fn route_keyboard(&mut self, event: &Event, context: &mut EventCtx) -> DocumentSearchBarEvent {
        if !self.input.focused {
            return DocumentSearchBarEvent::default();
        }
        if let Event::KeyDown(KeyCode::Char(character), modifiers) = event
            && character.eq_ignore_ascii_case(&'f')
            && (modifiers.cmd || modifiers.ctrl)
        {
            return DocumentSearchBarEvent::default();
        }
        let action = match event {
            Event::KeyDown(KeyCode::Escape, _) => Some(DocumentSearchBarAction::Close),
            Event::KeyDown(KeyCode::Enter, modifiers) if !self.query_box.has_preedit() => {
                Some(if modifiers.shift {
                    DocumentSearchBarAction::Prev
                } else {
                    DocumentSearchBarAction::Next
                })
            }
            _ => self.query_box.on_event(event, context).map(|action| self.translate(action)),
        };
        DocumentSearchBarEvent { consumed: true, action }
    }
}

impl Widget for DocumentSearchBarWidget {
    fn set_rect(&mut self, rect: Rect, context: &mut LayoutCtx) {
        self.rect = rect;
        let inset = crate::constants::TINY_GAP * context.dpi;
        let control_width = CONTROL_WIDTH_LOGICAL * context.dpi;
        let count_width = COUNT_WIDTH_LOGICAL * context.dpi;
        let height = (rect.h - inset * 2.0).max(0.0);
        let controls_x = rect.x + rect.w - inset - control_width * self.controls.len() as f32;
        self.count_rect = Rect::new(controls_x - count_width, rect.y + inset, count_width, height);
        let query_rect = Rect::new(
            rect.x + inset,
            rect.y + inset,
            (self.count_rect.x - rect.x - inset * 2.0).max(0.0),
            height,
        );
        self.query_box.layout(query_rect, context);
        for (index, button) in self.controls.iter_mut().enumerate() {
            button.set_style(ButtonStyle::ghost(context.theme.settings_theme()));
            button.set_rect(
                Rect::new(
                    controls_x + control_width * index as f32,
                    rect.y + inset,
                    control_width,
                    height,
                ),
                context,
            );
        }
    }

    fn paint(&self, context: &mut PaintCtx) {
        if !self.input.visible || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        context.list.fill(self.rect, context.theme.palette.input_bg);
        self.query_box.paint(context);
        for button in &self.controls {
            button.paint(context);
        }
        let font_size = crate::constants::CAPTION_FONT_SIZE * context.dpi;
        context.text(
            self.count_rect.x + crate::constants::TINY_GAP * context.dpi,
            self.count_rect.y + self.count_rect.h * 0.5 + font_size * BASELINE_CENTER_SHIFT,
            font_size,
            context.theme.palette.input_fg,
            &self.count_text(),
        );
    }
    fn hit(&self, px: f32, py: f32) -> bool {
        self.contains(px, py)
    }
    fn on_event(&mut self, event: &Event, context: &mut EventCtx) -> Option<WidgetAction> {
        self.route_event(event, context).consumed.then_some(WidgetAction::Consumed)
    }
    fn is_capturing(&self) -> bool {
        self.query_box.is_capturing() || self.controls.iter().any(Widget::is_capturing)
    }
    fn tooltip_at(&self, px: f32, py: f32) -> Option<crate::tooltip::TooltipHint> {
        if !self.input.visible {
            return None;
        }
        self.controls
            .iter()
            .zip(["上一个匹配（Shift+Enter）", "下一个匹配（Enter）", "关闭查找（Esc）"])
            .find(|(button, _)| button.hit(px, py))
            .map(|(button, label)| crate::tooltip::TooltipHint {
                label: label.to_owned(),
                target_rect: button.rect(),
            })
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, EventCtx, KeyCode, Modifiers};

    fn visible_widget(query: &str, focused: bool, count: usize) -> DocumentSearchBarWidget {
        let mut widget = DocumentSearchBarWidget::new();
        widget.set_input(DocumentSearchBarInput {
            query: query.to_owned(),
            visible: true,
            focused,
            match_count: count,
            current_match: 0,
        });
        let theme = crate::theme::test_theme();
        let mut measure = crate::NoopMeasure;
        let mut context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        widget.set_rect(
            Rect::new(100.0, 50.0, 300.0, DOCUMENT_SEARCH_BAR_HEIGHT_LOGICAL),
            &mut context,
        );
        widget
    }

    #[test]
    fn ime_preedit_is_consumed_without_changing_query_and_commit_edits_query() {
        let mut widget = visible_widget("", true, 0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let preedit = widget.route_event(
            &Event::ImePreedit { text: "中文".to_owned(), cursor: Some((0, 6)) },
            &mut context,
        );
        assert!(preedit.consumed);
        assert_eq!(widget.query_box.text(), "");
        assert!(widget.query_box.has_preedit());
        assert_eq!(
            widget.route_event(&Event::ImeCommit("中文".to_owned()), &mut context).action,
            Some(DocumentSearchBarAction::QueryChanged("中文".to_owned()))
        );
        assert!(widget.ime_cursor_rect().expect("focused query has IME rectangle").x >= 100.0);
    }

    #[test]
    fn snapshot_keeps_selection_and_preedit_until_focus_is_lost() {
        let mut widget = visible_widget("中文", true, 1);
        widget.select_query();
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        widget.route_event(
            &Event::ImePreedit { text: "zhong".to_owned(), cursor: None },
            &mut context,
        );
        widget.set_input(widget.input.clone());
        assert_eq!(widget.query_box.selection_text(), Some("中文"));
        assert!(widget.query_box.has_preedit());
        let mut input = widget.input.clone();
        input.focused = false;
        widget.set_input(input);
        assert!(!widget.query_box.has_preedit());
        assert!(!widget.ime_allowed());
        assert!(widget.ime_cursor_rect().is_none());
    }

    #[test]
    fn unfocused_query_does_not_consume_keyboard_or_ime() {
        let mut widget = visible_widget("", false, 0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        for event in [
            Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE),
            Event::KeyDown(KeyCode::Enter, Modifiers::NONE),
            Event::ImeCommit("中文".to_owned()),
            Event::ImeEnable,
            Event::ImeDisable,
        ] {
            assert!(!widget.route_event(&event, &mut context).consumed);
        }
        assert_eq!(widget.query_box.text(), "");
    }

    #[test]
    fn enter_during_ime_composition_does_not_navigate_results() {
        let mut widget = visible_widget("中文", true, 2);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        widget.route_event(
            &Event::ImePreedit { text: "拼".to_owned(), cursor: Some((0, 3)) },
            &mut context,
        );
        let routed =
            widget.route_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context);
        assert!(routed.consumed);
        assert_ne!(routed.action, Some(DocumentSearchBarAction::Next));
    }

    #[test]
    fn narrow_layout_uses_global_coordinates_and_buttons_navigate() {
        let mut widget = visible_widget("中文", true, 2);
        assert!(widget.query_box.rect().w > 100.0);
        assert!(widget.query_box.rect().x >= widget.rect.x);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        for (index, expected) in [
            DocumentSearchBarAction::Prev,
            DocumentSearchBarAction::Next,
            DocumentSearchBarAction::Close,
        ]
        .into_iter()
        .enumerate()
        {
            let rect = widget.controls[index].rect();
            assert!(rect.x >= widget.rect.x && rect.x + rect.w <= widget.rect.x + widget.rect.w);
            let px = rect.x + rect.w * 0.5;
            let py = rect.y + rect.h * 0.5;
            widget.route_event(
                &Event::MouseDown { px, py, button: crate::MouseButton::Left },
                &mut context,
            );
            assert_eq!(
                widget
                    .route_event(
                        &Event::MouseUp { px, py, button: crate::MouseButton::Left },
                        &mut context
                    )
                    .action,
                Some(expected)
            );
        }
    }

    #[test]
    fn no_results_shows_zero_and_disables_navigation() {
        let mut widget = visible_widget("不存在", true, 0);
        assert_eq!(widget.count_text(), "0/0");
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let rect = widget.controls[0].rect();
        let px = rect.x + rect.w * 0.5;
        let py = rect.y + rect.h * 0.5;
        widget.route_event(
            &Event::MouseDown { px, py, button: crate::MouseButton::Left },
            &mut context,
        );
        assert_eq!(
            widget
                .route_event(
                    &Event::MouseUp { px, py, button: crate::MouseButton::Left },
                    &mut context
                )
                .action,
            None
        );
        widget.set_input(DocumentSearchBarInput { visible: true, ..Default::default() });
        assert_eq!(widget.count_text(), "");
    }

    #[test]
    fn dragging_query_over_button_keeps_text_selection_capture() {
        let mut widget = visible_widget("abcdef", true, 1);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let rect = widget.query_box.rect();
        let py = rect.y + rect.h * 0.5;
        widget.route_event(
            &Event::MouseDown { px: rect.x + 1.0, py, button: crate::MouseButton::Left },
            &mut context,
        );
        let button_rect = widget.controls[0].rect();
        widget.route_event(&Event::MouseMove { px: button_rect.x + 1.0, py }, &mut context);
        assert!(widget.query_box.selection_text().is_some());
        widget.route_event(
            &Event::MouseUp { px: button_rect.x + 1.0, py, button: crate::MouseButton::Left },
            &mut context,
        );
        assert!(!widget.is_capturing());
    }

    #[test]
    fn focused_enter_navigates_and_escape_closes() {
        let mut widget = DocumentSearchBarWidget::new();
        widget.set_input(DocumentSearchBarInput {
            visible: true,
            focused: true,
            ..Default::default()
        });
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            widget
                .route_event(&Event::KeyDown(KeyCode::Enter, Modifiers::default()), &mut context)
                .action,
            Some(DocumentSearchBarAction::Next)
        );
        assert_eq!(
            widget
                .route_event(
                    &Event::KeyDown(
                        KeyCode::Enter,
                        Modifiers { shift: true, ..Default::default() }
                    ),
                    &mut context
                )
                .action,
            Some(DocumentSearchBarAction::Prev)
        );
        assert_eq!(
            widget
                .route_event(&Event::KeyDown(KeyCode::Escape, Modifiers::default()), &mut context)
                .action,
            Some(DocumentSearchBarAction::Close)
        );
    }
}
