//! 正式层级标签的输入体验组件；领域校验由产品/core 层负责。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use std::any::Any;

const TAG_EDITOR_ROW_HEIGHT_LOGICAL: f32 = 26.0;
const TAG_EDITOR_HORIZONTAL_PADDING_LOGICAL: f32 = 8.0;
const TAG_EDITOR_FONT_SIZE_LOGICAL: f32 = 12.0;
const TAG_EDITOR_LABEL: &str = "标签：";
const TAG_EDITOR_ADD_PROMPT: &str = "添加标签";
const TAG_EDITOR_CARET_WIDTH_LOGICAL: f32 = 1.0;
const TAG_EDITOR_CARET_VERTICAL_INSET_LOGICAL: f32 = 6.0;
const TAG_EDITOR_ASCII_TEXT_WIDTH_RATIO: f32 = 0.55;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagEditorAction {
    TextSubmitted(String),
    ChipRemoved(String),
    SuggestionSelected(String),
    Cancelled,
    Dismissed,
}

pub struct TagEditorWidget {
    input: TagEditorInput,
    rect: Rect,
    editing: bool,
}

impl TagEditorWidget {
    pub fn new() -> Self {
        Self { input: TagEditorInput::default(), rect: Rect::ZERO, editing: false }
    }

    pub fn set_input(&mut self, input: TagEditorInput) {
        self.input = input;
    }

    pub fn set_editing(&mut self, editing: bool) {
        self.editing = editing && self.input.enabled;
    }

    pub fn pending_text(&self) -> &str {
        &self.input.pending_text
    }

    pub fn suggestions_open(&self) -> bool {
        self.input.suggestions_open
    }

    pub fn visible_chip_counts(&self, max_visible: usize) -> (usize, usize) {
        let visible = self.input.chips.len().min(max_visible);
        (visible, self.input.chips.len().saturating_sub(visible))
    }

    pub fn event_action(&mut self, event: &Event, dpi: f32) -> Option<TagEditorAction> {
        if !self.input.enabled {
            return None;
        }
        match event {
            Event::KeyDown(crate::core::KeyCode::Escape, _) => {
                self.input.pending_text.clear();
                self.input.suggestions_open = false;
                self.editing = false;
                Some(TagEditorAction::Cancelled)
            }
            Event::KeyDown(crate::core::KeyCode::Backspace, _) => {
                if self.input.pending_text.pop().is_some() {
                    return None;
                }
                self.input
                    .chips
                    .iter()
                    .rev()
                    .find(|chip| chip.removable)
                    .map(|chip| TagEditorAction::ChipRemoved(chip.chip_key.clone()))
            }
            Event::KeyDown(crate::core::KeyCode::Enter, _) => {
                let text = self.input.pending_text.trim();
                if text.is_empty() {
                    return None;
                }
                let submitted = text.to_owned();
                self.input.pending_text.clear();
                self.input.suggestions_open = false;
                Some(TagEditorAction::TextSubmitted(submitted))
            }
            Event::KeyDown(crate::core::KeyCode::Char(character), modifiers)
                if !modifiers.cmd && !modifiers.ctrl && !modifiers.alt =>
            {
                self.input.pending_text.push(*character);
                self.input.suggestions_open = true;
                None
            }
            Event::MouseDown { px, py, button: crate::core::MouseButton::Left } => {
                if !self.rect.contains(*px, *py) {
                    self.editing = false;
                    return Some(TagEditorAction::Dismissed);
                }
                self.editing = true;
                if !self.input.suggestions_open {
                    return None;
                }
                let suggestion_index = self.suggestion_index_at(*py, dpi)?;
                let suggestion = self.input.suggestions.get(suggestion_index)?;
                if !suggestion.enabled {
                    return None;
                }
                self.input.pending_text.clear();
                self.input.suggestions_open = false;
                Some(TagEditorAction::SuggestionSelected(suggestion.option_key.clone()))
            }
            _ => None,
        }
    }

    fn suggestion_index_at(&self, py: f32, dpi: f32) -> Option<usize> {
        self.input.suggestions.iter().enumerate().find_map(|(index, _)| {
            let top = self.rect.y + (index + 1) as f32 * TAG_EDITOR_ROW_HEIGHT_LOGICAL * dpi;
            Rect::new(self.rect.x, top, self.rect.w, TAG_EDITOR_ROW_HEIGHT_LOGICAL * dpi)
                .contains(self.rect.x + 1.0, py)
                .then_some(index)
        })
    }
}

impl Default for TagEditorWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TagEditorWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
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
        let label = if self.input.chips.is_empty() && self.input.pending_text.is_empty() {
            format!("{TAG_EDITOR_LABEL}{TAG_EDITOR_ADD_PROMPT}")
        } else {
            TAG_EDITOR_LABEL.to_owned()
        };
        ctx.text(
            x,
            baseline,
            TAG_EDITOR_FONT_SIZE_LOGICAL * ctx.dpi,
            ctx.theme.palette.text_muted,
            &label,
        );
        x += label.chars().count() as f32
            * TAG_EDITOR_FONT_SIZE_LOGICAL
            * ctx.dpi
            * TAG_EDITOR_ASCII_TEXT_WIDTH_RATIO;
        for chip in self.input.chips.iter().take(visible_count) {
            let label = format!("{}  ", chip.label);
            ctx.text(
                x,
                baseline,
                TAG_EDITOR_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.text_muted,
                &label,
            );
            x += label.chars().count() as f32
                * TAG_EDITOR_FONT_SIZE_LOGICAL
                * ctx.dpi
                * TAG_EDITOR_ASCII_TEXT_WIDTH_RATIO;
        }
        if hidden_count > 0 {
            let folded = format!("+{hidden_count}");
            ctx.text(
                x,
                baseline,
                TAG_EDITOR_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.text_muted,
                &folded,
            );
        }
        if !self.input.pending_text.is_empty() {
            ctx.text(
                x,
                baseline,
                TAG_EDITOR_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.text_main,
                &self.input.pending_text,
            );
            x += self.input.pending_text.chars().count() as f32
                * TAG_EDITOR_FONT_SIZE_LOGICAL
                * ctx.dpi
                * TAG_EDITOR_ASCII_TEXT_WIDTH_RATIO;
        }
        if self.editing {
            let vertical_inset = TAG_EDITOR_CARET_VERTICAL_INSET_LOGICAL * ctx.dpi;
            ctx.list.fill(
                Rect::new(
                    x,
                    self.rect.y + vertical_inset,
                    TAG_EDITOR_CARET_WIDTH_LOGICAL * ctx.dpi,
                    (self.rect.h - vertical_inset * 2.0).max(0.0),
                ),
                ctx.theme.palette.input_fg,
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        self.event_action(event, ctx.dpi).map(|action| match action {
            TagEditorAction::TextSubmitted(text) => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: TAG_EDITOR_SUBMIT_ID,
                    value: TextPayload::Plain(text),
                })
            }
            TagEditorAction::ChipRemoved(chip_key) => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: TAG_EDITOR_REMOVE_ID,
                    value: TextPayload::Plain(chip_key),
                })
            }
            TagEditorAction::SuggestionSelected(option_key) => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: TAG_EDITOR_SUGGESTION_ID,
                    value: TextPayload::Plain(option_key),
                })
            }
            TagEditorAction::Cancelled => {
                WidgetAction::Control(ControlAction::Activated { id: TAG_EDITOR_CANCEL_ID })
            }
            TagEditorAction::Dismissed => {
                WidgetAction::Control(ControlAction::Activated { id: TAG_EDITOR_DISMISS_ID })
            }
        })
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
    use crate::core::{Event, KeyCode, Modifiers};

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

    #[test]
    fn tag_editor_supports_escape_backspace_and_single_line_chip_folding() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(input());

        assert_eq!(
            editor.event_action(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), 1.0),
            Some(TagEditorAction::Cancelled)
        );
        assert_eq!(
            editor.event_action(&Event::KeyDown(KeyCode::Backspace, Modifiers::NONE), 1.0),
            Some(TagEditorAction::ChipRemoved("product/notora".to_owned()))
        );
        assert_eq!(editor.visible_chip_counts(1), (1, 1));
    }

    #[test]
    fn enter_submits_pending_text_and_clicking_a_suggestion_returns_its_option_key() {
        let mut editor = TagEditorWidget::new();
        editor.set_input(input());

        let _ = editor.event_action(&Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE), 1.0);
        assert_eq!(
            editor.event_action(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), 1.0),
            Some(TagEditorAction::TextSubmitted("x".to_owned()))
        );

        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        editor.set_rect(Rect::new(0.0, 0.0, 320.0, 100.0), &mut layout_context);
        editor.set_input(input());
        assert_eq!(
            editor.event_action(
                &Event::MouseDown {
                    px: 80.0,
                    py: TAG_EDITOR_ROW_HEIGHT_LOGICAL * 1.5,
                    button: crate::core::MouseButton::Left,
                },
                1.0,
            ),
            Some(TagEditorAction::SuggestionSelected("product/textora".to_owned()))
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
            |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "标签：添加标签")
        ));
    }

    #[test]
    fn editing_empty_tag_editor_paints_an_input_caret() {
        use crate::core::paint::{DrawCmd, DrawList};

        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut editor = TagEditorWidget::new();
        editor.set_input(TagEditorInput { enabled: true, ..TagEditorInput::default() });
        editor.set_editing(true);
        editor.set_rect(
            Rect::new(0.0, 0.0, 320.0, TAG_EDITOR_ROW_HEIGHT_LOGICAL),
            &mut layout_context,
        );

        let mut draw_list = DrawList::new();
        let mut paint_context = PaintCtx::new(&mut draw_list, &theme, 1.0);
        editor.paint(&mut paint_context);

        assert!(
            draw_list
                .cmds
                .iter()
                .any(|command| matches!(command, DrawCmd::FillRect { rect, .. } if rect.w == 1.0))
        );
    }
}
