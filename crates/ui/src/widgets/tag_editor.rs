//! 正式层级标签的输入体验组件；领域校验由产品/core 层负责。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use crate::widgets::text_box::{TextBox, TextBoxChrome};
use std::any::Any;

const TAG_EDITOR_ROW_HEIGHT_LOGICAL: f32 = 26.0;
const TAG_EDITOR_HORIZONTAL_PADDING_LOGICAL: f32 = 8.0;
const TAG_EDITOR_FONT_SIZE_LOGICAL: f32 = 12.0;
const TAG_EDITOR_LABEL: &str = "标签：";
const TAG_EDITOR_ADD_PROMPT: &str = "添加标签";
const TAG_EDITOR_ASCII_TEXT_WIDTH_RATIO: f32 = 0.55;
const TAG_EDITOR_WIDE_TEXT_WIDTH_RATIO: f32 = 1.0;

pub const TAG_EDITOR_INPUT_ID: WidgetId = WidgetId(10_201);
pub const TAG_EDITOR_SUBMIT_ID: WidgetId = WidgetId(10_202);
pub const TAG_EDITOR_REMOVE_ID: WidgetId = WidgetId(10_203);
pub const TAG_EDITOR_SUGGESTION_ID: WidgetId = WidgetId(10_204);
pub const TAG_EDITOR_CANCEL_ID: WidgetId = WidgetId(10_205);
pub const TAG_EDITOR_DISMISS_ID: WidgetId = WidgetId(10_206);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagChipInput {
    pub chip_key: String,
    pub label: String,
    pub removable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagSuggestionInput {
    pub option_key: String,
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagEditorInput {
    pub chips: Vec<TagChipInput>,
    pub suggestions: Vec<TagSuggestionInput>,
    pub pending_text: String,
    pub suggestions_open: bool,
    pub enabled: bool,
    pub compact: bool,
}

pub struct TagEditorWidget {
    input: TagEditorInput,
    rect: Rect,
    text_box: TextBox,
}

impl TagEditorWidget {
    pub fn new() -> Self {
        let mut text_box = TextBox::with_id(TAG_EDITOR_INPUT_ID);
        text_box.set_chrome(TextBoxChrome::Seamless);
        text_box.set_font_size_logical(TAG_EDITOR_FONT_SIZE_LOGICAL);
        text_box.set_leading_content_inset_logical(0.0);
        text_box.set_placeholder(TAG_EDITOR_ADD_PROMPT);
        Self { input: TagEditorInput::default(), rect: Rect::ZERO, text_box }
    }

    pub fn set_input(&mut self, input: TagEditorInput) {
        if !input.enabled {
            self.set_keyboard_focus(None);
        }
        self.text_box.sync_text(&input.pending_text);
        self.input = input;
    }

    pub fn set_blink_visible(&mut self, visible: bool) {
        self.text_box.set_blink(visible);
    }

    pub fn pending_text(&self) -> &str {
        self.text_box.text()
    }

    pub fn suggestions_open(&self) -> bool {
        self.input.suggestions_open
    }

    pub fn has_keyboard_focus(&self) -> bool {
        self.text_box.is_focused()
    }

    /// Return the local rectangle where the operating system should place IME candidates.
    ///
    /// The parent translates this local rectangle into window coordinates. A tag editor only
    /// owns the IME while it is enabled and actively editing; returning `None` otherwise keeps
    /// stale tag geometry from being used after focus moves to another control.
    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        if !self.input.enabled || !self.text_box.is_focused() {
            return None;
        }
        Some(self.text_box.ime_cursor_rect())
    }

    pub fn visible_chip_counts(&self, max_visible: usize) -> (usize, usize) {
        let visible = self.input.chips.len().min(max_visible);
        (visible, self.input.chips.len().saturating_sub(visible))
    }

    fn layout_text_box(&mut self, context: &mut LayoutCtx<'_>) {
        let font_size = TAG_EDITOR_FONT_SIZE_LOGICAL * context.dpi;
        let measure: &mut dyn crate::core::measure::TextMeasure = match context.ui_measure {
            Some(ref mut measure) => &mut **measure,
            None => context.measure,
        };
        let visible_count =
            self.visible_chip_counts(if self.input.compact { 2 } else { self.input.chips.len() }).0;
        let mut text_box_x = self.rect.x + TAG_EDITOR_HORIZONTAL_PADDING_LOGICAL * context.dpi;
        text_box_x += measure.measure(TAG_EDITOR_LABEL, font_size);
        for chip in self.input.chips.iter().take(visible_count) {
            text_box_x += measure.measure(&format!("{}  ", chip.label), font_size);
        }
        if self.input.chips.len() > visible_count {
            text_box_x +=
                measure.measure(&format!("+{}", self.input.chips.len() - visible_count), font_size);
        }
        self.text_box.set_rect(
            Rect::new(
                text_box_x,
                self.rect.y,
                (self.rect.right() - text_box_x).max(0.0),
                self.rect.h,
            ),
            context,
        );
    }

    fn suggestion_index_at(&self, py: f32, dpi: f32) -> Option<usize> {
        self.input.suggestions.iter().enumerate().find_map(|(index, _)| {
            let top = self.rect.y + (index + 1) as f32 * TAG_EDITOR_ROW_HEIGHT_LOGICAL * dpi;
            Rect::new(self.rect.x, top, self.rect.w, TAG_EDITOR_ROW_HEIGHT_LOGICAL * dpi)
                .contains(self.rect.x + 1.0, py)
                .then_some(index)
        })
    }

    fn map_text_box_action(&mut self, action: Option<WidgetAction>) -> Option<WidgetAction> {
        match action {
            Some(WidgetAction::Control(ControlAction::TextEdited {
                id: TAG_EDITOR_INPUT_ID,
                value: TextPayload::Plain(text),
            })) => {
                self.input.pending_text = text;
                self.input.suggestions_open = true;
                Some(WidgetAction::Consumed)
            }
            Some(WidgetAction::Control(ControlAction::TextCommitted {
                id: TAG_EDITOR_INPUT_ID,
                value: TextPayload::Plain(text),
            })) => {
                let submitted = text.trim().to_owned();
                if submitted.is_empty() {
                    return Some(WidgetAction::Consumed);
                }
                self.text_box.set_text("");
                self.input.pending_text.clear();
                self.input.suggestions_open = false;
                Some(WidgetAction::Control(ControlAction::TextCommitted {
                    id: TAG_EDITOR_SUBMIT_ID,
                    value: TextPayload::Plain(submitted),
                }))
            }
            other => other,
        }
    }

    fn focus_text_box_from_row_click(
        &mut self,
        px: f32,
        py: f32,
        context: &mut EventCtx<'_>,
    ) -> Option<WidgetAction> {
        let text_box_rect = self.text_box.rect();
        let input_x = px.max(text_box_rect.x).min(text_box_rect.right());
        let action = self.text_box.on_event(
            &Event::MouseDown { px: input_x, py, button: crate::core::MouseButton::Left },
            context,
        );
        self.map_text_box_action(action)
    }

    fn route_text_box_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<WidgetAction> {
        let action = self.text_box.on_event(event, context);
        self.map_text_box_action(action)
    }

    fn cancel_editing(&mut self) -> WidgetAction {
        self.text_box.set_text("");
        self.input.pending_text.clear();
        self.input.suggestions_open = false;
        self.set_keyboard_focus(None);
        WidgetAction::Control(ControlAction::Activated { id: TAG_EDITOR_CANCEL_ID })
    }

    fn remove_last_removable_chip(&self) -> WidgetAction {
        self.input.chips.iter().rev().find(|chip| chip.removable).map_or(
            WidgetAction::Consumed,
            |chip| {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: TAG_EDITOR_REMOVE_ID,
                    value: TextPayload::Plain(chip.chip_key.clone()),
                })
            },
        )
    }

    fn handle_focused_key_down(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<WidgetAction> {
        match event {
            Event::KeyDown(crate::core::KeyCode::Escape, _) => Some(self.cancel_editing()),
            Event::KeyDown(crate::core::KeyCode::Backspace, _)
                if self.text_box.text().is_empty() =>
            {
                Some(self.remove_last_removable_chip())
            }
            Event::KeyDown(..) => self.route_text_box_event(event, context),
            _ => None,
        }
    }

    fn select_suggestion_at(&mut self, py: f32, dpi: f32) -> Option<WidgetAction> {
        let suggestion_index = self.suggestion_index_at(py, dpi)?;
        let suggestion = self.input.suggestions.get(suggestion_index)?;
        if !suggestion.enabled {
            return Some(WidgetAction::Consumed);
        }
        let option_key = suggestion.option_key.clone();
        self.text_box.set_text("");
        self.input.pending_text.clear();
        self.input.suggestions_open = false;
        Some(WidgetAction::Control(ControlAction::TextCommitted {
            id: TAG_EDITOR_SUGGESTION_ID,
            value: TextPayload::Plain(option_key),
        }))
    }

    fn handle_left_mouse_down(
        &mut self,
        px: f32,
        py: f32,
        context: &mut EventCtx<'_>,
    ) -> Option<WidgetAction> {
        let suggestions_start_y = self.rect.y + TAG_EDITOR_ROW_HEIGHT_LOGICAL * context.dpi;
        if self.input.suggestions_open
            && py >= suggestions_start_y
            && let Some(action) = self.select_suggestion_at(py, context.dpi)
        {
            return Some(action);
        }
        if self.rect.contains(px, py) {
            return self.focus_text_box_from_row_click(px, py, context);
        }
        if !self.has_keyboard_focus() && !self.input.suggestions_open {
            return None;
        }
        self.input.suggestions_open = false;
        self.set_keyboard_focus(None);
        Some(WidgetAction::Control(ControlAction::Activated { id: TAG_EDITOR_DISMISS_ID }))
    }
}

fn paint_text_and_measure(
    context: &mut PaintCtx<'_>,
    x: f32,
    baseline: f32,
    color: [f32; 4],
    text: &str,
) -> f32 {
    let font_size = TAG_EDITOR_FONT_SIZE_LOGICAL * context.dpi;
    if let Some(shaper) = context.shaper.as_deref_mut() {
        return context.list.text_shaped(x, baseline, font_size, color, text, shaper);
    }
    text.chars()
        .map(|character| {
            if character.is_ascii() {
                TAG_EDITOR_ASCII_TEXT_WIDTH_RATIO
            } else {
                TAG_EDITOR_WIDE_TEXT_WIDTH_RATIO
            }
        })
        .sum::<f32>()
        * font_size
}

impl Default for TagEditorWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TagEditorWidget {
    fn set_rect(&mut self, rect: Rect, context: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        self.layout_text_box(context);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if !self.input.enabled || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (visible_count, hidden_count) =
            self.visible_chip_counts(if self.input.compact { 2 } else { self.input.chips.len() });
        let mut x = self.rect.x + TAG_EDITOR_HORIZONTAL_PADDING_LOGICAL * ctx.dpi;
        let baseline =
            self.rect.y + self.rect.h * 0.5 + TAG_EDITOR_FONT_SIZE_LOGICAL * ctx.dpi * 0.35;
        x += paint_text_and_measure(
            ctx,
            x,
            baseline,
            ctx.theme.palette.text_muted,
            TAG_EDITOR_LABEL,
        );
        for chip in self.input.chips.iter().take(visible_count) {
            let label = format!("{}  ", chip.label);
            x += paint_text_and_measure(ctx, x, baseline, ctx.theme.palette.text_muted, &label);
        }
        if hidden_count > 0 {
            let folded = format!("+{hidden_count}");
            paint_text_and_measure(ctx, x, baseline, ctx.theme.palette.text_muted, &folded);
        }
        self.text_box.paint(ctx);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        Some(TAG_EDITOR_INPUT_ID)
    }

    fn is_focusable(&self) -> bool {
        self.input.enabled
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        let focused = self.input.enabled && focused_id == Some(TAG_EDITOR_INPUT_ID);
        if !focused {
            self.text_box.cancel_transient_interaction();
        }
        self.text_box.set_keyboard_focus(focused.then_some(TAG_EDITOR_INPUT_ID));
        self.text_box.set_placeholder(if focused { "" } else { TAG_EDITOR_ADD_PROMPT });
    }

    fn on_event(&mut self, event: &Event, context: &mut EventCtx) -> Option<WidgetAction> {
        if !self.input.enabled {
            return None;
        }

        match event {
            Event::KeyDown(..) if self.has_keyboard_focus() => {
                self.handle_focused_key_down(event, context)
            }
            Event::MouseDown { px, py, button: crate::core::MouseButton::Left } => {
                self.handle_left_mouse_down(*px, *py, context)
            }
            Event::MouseMove { .. } | Event::MouseUp { .. } if self.text_box.is_capturing() => {
                self.route_text_box_event(event, context)
            }
            Event::ImePreedit { .. }
            | Event::ImeCommit(_)
            | Event::ImeEnable
            | Event::ImeDisable
                if self.has_keyboard_focus() =>
            {
                self.route_text_box_event(event, context)
            }
            Event::InteractionCancel => {
                let had_focus = self.has_keyboard_focus();
                let interaction_changed = self.text_box.on_event(event, context).is_some();
                self.set_keyboard_focus(None);
                (had_focus || interaction_changed).then_some(WidgetAction::Consumed)
            }
            Event::PointerLeave
            | Event::MouseMove { .. }
            | Event::MouseUp { .. }
            | Event::Wheel { .. } => None,
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.text_box.is_capturing()
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
    use crate::core::{Event, EventCtx, KeyCode, Modifiers, MouseButton, WidgetAction};

    fn input() -> TagEditorInput {
        TagEditorInput {
            chips: vec![
                TagChipInput {
                    chip_key: "product".to_owned(),
                    label: "产品".to_owned(),
                    removable: true,
                },
                TagChipInput {
                    chip_key: "product/notora".to_owned(),
                    label: "产品/Notora".to_owned(),
                    removable: true,
                },
            ],
            suggestions: vec![TagSuggestionInput {
                option_key: "product/textora".to_owned(),
                label: "产品/Textora".to_owned(),
                enabled: true,
            }],
            pending_text: String::new(),
            suggestions_open: true,
            enabled: true,
            compact: false,
        }
    }

    fn paint_editor(editor: &TagEditorWidget) -> crate::core::paint::DrawList {
        let theme = crate::theme::test_theme();
        let mut draw_list = crate::core::paint::DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };
        editor.paint(&mut paint_context);
        draw_list
    }

    const TEST_CHARACTER_WIDTH_RATIO: f32 = 0.5;

    struct TestMeasure;

    impl crate::core::measure::TextMeasure for TestMeasure {
        fn measure(&mut self, text: &str, font_size: f32) -> f32 {
            text.chars().count() as f32 * font_size * TEST_CHARACTER_WIDTH_RATIO
        }
    }

    fn layout_editor(editor: &mut TagEditorWidget) {
        let theme = crate::theme::test_theme();
        let mut measure = TestMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        editor.set_rect(
            Rect::new(0.0, 0.0, 320.0, TAG_EDITOR_ROW_HEIGHT_LOGICAL),
            &mut layout_context,
        );
    }

    #[test]
    fn tag_editor_delegates_pointer_and_keyboard_focus_to_its_text_box() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput {
            enabled: true,
            suggestions_open: false,
            ..TagEditorInput::default()
        });
        layout_editor(&mut editor);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);

        assert_eq!(editor.on_event(&Event::MouseMove { px: 40.0, py: 12.0 }, &mut context), None);
        assert_eq!(editor.ime_cursor_rect(), None);
        assert_eq!(
            editor.on_event(
                &Event::MouseDown { px: 40.0, py: 12.0, button: MouseButton::Left },
                &mut context,
            ),
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: TAG_EDITOR_INPUT_ID }))
        );

        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        assert!(editor.ime_cursor_rect().is_some());
        assert_eq!(
            editor.on_event(&Event::ImeCommit("统一焦点".to_owned()), &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(editor.pending_text(), "统一焦点");
    }

    #[test]
    fn tag_editor_supports_escape_backspace_and_single_line_chip_folding() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(input());
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);

        assert_eq!(
            editor.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut context),
            Some(WidgetAction::Control(ControlAction::Activated { id: TAG_EDITOR_CANCEL_ID }))
        );
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        assert_eq!(
            editor.on_event(&Event::KeyDown(KeyCode::Backspace, Modifiers::NONE), &mut context),
            Some(WidgetAction::Control(ControlAction::TextCommitted {
                id: TAG_EDITOR_REMOVE_ID,
                value: TextPayload::Plain("product/notora".to_owned()),
            }))
        );
        assert_eq!(editor.visible_chip_counts(1), (1, 1));
    }

    #[test]
    fn enter_submits_pending_text_and_clicking_a_suggestion_returns_its_option_key() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(input());
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);

        let _ = editor.on_event(&Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE), &mut context);
        assert_eq!(
            editor.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            Some(WidgetAction::Control(ControlAction::TextCommitted {
                id: TAG_EDITOR_SUBMIT_ID,
                value: TextPayload::Plain("x".to_owned()),
            }))
        );

        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        editor.set_rect(Rect::new(0.0, 0.0, 320.0, 100.0), &mut layout_context);
        editor.set_input(input());
        assert_eq!(
            editor.on_event(
                &Event::MouseDown {
                    px: 80.0,
                    py: TAG_EDITOR_ROW_HEIGHT_LOGICAL * 1.5,
                    button: crate::core::MouseButton::Left,
                },
                &mut context,
            ),
            Some(WidgetAction::Control(ControlAction::TextCommitted {
                id: TAG_EDITOR_SUGGESTION_ID,
                value: TextPayload::Plain("product/textora".to_owned()),
            }))
        );
    }

    #[test]
    fn tag_editor_commits_ime_text_only_while_focused() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..input() });

        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            editor.on_event(&Event::ImeCommit("未聚焦".to_owned()), &mut context),
            None,
            "IME must not mutate a tag editor that does not own keyboard focus"
        );
        assert_eq!(editor.pending_text(), "");

        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        assert_eq!(
            editor.on_event(&Event::ImeCommit("你好".to_owned()), &mut context),
            Some(WidgetAction::Consumed),
            "focused tag editor must consume IME commit without submitting a chip"
        );
        assert_eq!(editor.pending_text(), "你好");
    }

    #[test]
    fn tag_editor_consumes_ime_enable_and_disable_only_while_focused() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..input() });

        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(editor.on_event(&Event::ImeEnable, &mut context), None);
        assert_eq!(editor.on_event(&Event::ImeDisable, &mut context), None);

        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        assert_eq!(editor.on_event(&Event::ImeEnable, &mut context), Some(WidgetAction::Consumed));
        assert_eq!(
            editor.on_event(
                &Event::ImePreedit { text: "拼".to_owned(), cursor: Some((0, 3)) },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(editor.on_event(&Event::ImeDisable, &mut context), Some(WidgetAction::Consumed));
        assert_eq!(editor.pending_text(), "");

        layout_editor(&mut editor);
        let draw_list = paint_editor(&editor);
        assert!(draw_list.cmds.iter().all(
            |command| !matches!(command, crate::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "拼")
        ));
    }

    #[test]
    fn tag_editor_cancel_clears_editing_and_preedit_without_losing_draft() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput {
            enabled: true,
            pending_text: "已提交".to_owned(),
            ..input()
        });
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        layout_editor(&mut editor);

        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            editor.on_event(
                &Event::ImePreedit { text: "未完成".to_owned(), cursor: Some((0, 6)) },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(editor.on_event(&Event::PointerLeave, &mut context), None);
        assert!(editor.ime_cursor_rect().is_some());

        assert_eq!(
            editor.on_event(&Event::InteractionCancel, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(editor.pending_text(), "已提交");
        assert_eq!(editor.ime_cursor_rect(), None);
        assert_eq!(editor.on_event(&Event::InteractionCancel, &mut context), None);
    }

    #[test]
    fn disabling_tag_editor_ends_active_editing() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..input() });
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        layout_editor(&mut editor);
        assert!(editor.ime_cursor_rect().is_some());

        editor.set_input(TagEditorInput { enabled: false, ..input() });

        assert_eq!(editor.ime_cursor_rect(), None);
        editor.set_input(TagEditorInput { enabled: true, ..input() });
        assert_eq!(editor.ime_cursor_rect(), None);
    }

    #[test]
    fn tag_editor_reports_an_ime_cursor_rect_only_while_editing() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput {
            enabled: true,
            pending_text: "已提交".to_owned(),
            ..input()
        });
        layout_editor(&mut editor);
        assert_eq!(editor.ime_cursor_rect(), None);

        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        layout_editor(&mut editor);
        let committed_rect = editor.ime_cursor_rect().expect("focused tag should expose IME rect");
        assert!(committed_rect.x > 0.0);
        assert_eq!(committed_rect.w, 2.0);
        assert!(committed_rect.h > 0.0);

        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let _ = editor.on_event(
            &Event::ImePreedit { text: "拼音".to_owned(), cursor: Some((0, 6)) },
            &mut context,
        );
        layout_editor(&mut editor);
        let preedit_rect = editor.ime_cursor_rect().expect("active preedit should keep IME rect");
        assert!(preedit_rect.x > committed_rect.x);

        editor.set_keyboard_focus(None);
        assert_eq!(editor.ime_cursor_rect(), None);
    }

    #[test]
    fn tag_editor_preedit_is_painted_without_submitting_a_tag() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..input() });
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));

        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            editor.on_event(
                &Event::ImePreedit { text: "拼音".to_owned(), cursor: Some((0, 6)) },
                &mut context,
            ),
            Some(WidgetAction::Consumed),
            "focused tag editor must consume preedit"
        );
        assert_eq!(editor.pending_text(), "", "preedit must remain separate from committed draft");

        layout_editor(&mut editor);
        let draw_list = paint_editor(&editor);
        assert!(
            draw_list.cmds.iter().any(
                |command| matches!(command, crate::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "拼音")
            ),
            "active tag editor should paint the current IME preedit"
        );

        assert_eq!(
            editor.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            Some(WidgetAction::Consumed),
            "preedit alone must not submit a tag"
        );
        assert_eq!(editor.pending_text(), "");
    }

    #[test]
    fn leaving_tag_focus_clears_preedit_without_losing_committed_draft() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..input() });
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));

        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            editor.on_event(&Event::ImeCommit("已提交".to_owned()), &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(editor.pending_text(), "已提交");

        assert_eq!(
            editor.on_event(
                &Event::ImePreedit { text: "未完成".to_owned(), cursor: Some((0, 6)) },
                &mut context,
            ),
            Some(WidgetAction::Consumed)
        );

        editor.set_keyboard_focus(None);
        assert_eq!(
            editor.on_event(&Event::ImeDisable, &mut context),
            None,
            "IME events must stop at the tag editor after focus leaves it"
        );
        assert_eq!(editor.pending_text(), "已提交");

        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        layout_editor(&mut editor);
        let draw_list = paint_editor(&editor);
        assert!(
            draw_list.cmds.iter().all(
                |command| !matches!(command, crate::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "未完成")
            ),
            "disabling IME must clear the uncommitted preedit"
        );
    }

    #[test]
    fn empty_tag_editor_shows_an_add_tag_prompt() {
        use crate::core::paint::{DrawCmd, DrawList};

        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..TagEditorInput::default() });
        editor.set_rect(
            Rect::new(0.0, 0.0, 320.0, TAG_EDITOR_ROW_HEIGHT_LOGICAL),
            &mut layout_context,
        );

        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };
        editor.paint(&mut paint_context);

        assert!(draw_list.cmds.iter().any(
            |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == TAG_EDITOR_LABEL)
        ));
        assert!(draw_list.cmds.iter().any(
            |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == TAG_EDITOR_ADD_PROMPT)
        ));
    }

    #[test]
    fn editing_empty_tag_editor_paints_an_input_caret() {
        use crate::core::paint::{DrawCmd, DrawList};

        const TEXT_BOX_CARET_WIDTH: f32 = 2.0;

        let theme = crate::theme::test_theme();
        let mut measure = TestMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..TagEditorInput::default() });
        editor.set_keyboard_focus(Some(TAG_EDITOR_INPUT_ID));
        editor.set_blink_visible(true);
        editor.set_rect(
            Rect::new(0.0, 0.0, 320.0, TAG_EDITOR_ROW_HEIGHT_LOGICAL),
            &mut layout_context,
        );

        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let label_width = TAG_EDITOR_LABEL.chars().count() as f32
            * TAG_EDITOR_FONT_SIZE_LOGICAL
            * TEST_CHARACTER_WIDTH_RATIO;
        let mut paint_context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };
        editor.paint(&mut paint_context);

        let caret_x = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, .. } if rect.w == TEXT_BOX_CARET_WIDTH => Some(rect.x),
                _ => None,
            })
            .expect("editing tag input should paint a caret");
        let expected_x =
            TAG_EDITOR_HORIZONTAL_PADDING_LOGICAL + label_width - TEXT_BOX_CARET_WIDTH * 0.5;
        assert!(
            (caret_x - expected_x).abs() < 0.01,
            "tag caret x {caret_x} should follow shaped label width at {expected_x}"
        );
    }
}
