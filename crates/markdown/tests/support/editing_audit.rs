extern crate core as doc_core;

use doc_core::document::{DocView, DocViewMut, StringDocView};
use std::borrow::Cow;
use textora_markdown::builder::{BlockNode, MarkdownDoc};
use textora_markdown::parser::{MarkdownEvent, MarkdownTag, parse_markdown};
use textora_markdown::style::MarkdownStyle;
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::{EditIntent, EditPlan, EditPolicy, EditRequest, EditSelection};
use unicode_segmentation::UnicodeSegmentation;

pub const PARAGRAPH: &str = "前段";
pub const RULE: &str = "---";
pub const CODE: &str = "```rust\ncode\n```";
pub const NEXT_PARAGRAPH: &str = "后段";
pub const REPEAT_COUNT: usize = 3;
pub const GEOMETRY_TOLERANCE: f32 = 0.1;
pub const FONT_SIZE: f32 = 16.0;
pub const LINE_HEIGHT: f32 = 24.0;
pub const VIEWPORT_WIDTH: f32 = 800.0;
pub const VIEWPORT_HEIGHT: f32 = 1600.0;

pub fn apply(source: &str, cursor: usize, intent: EditIntent) -> (String, usize) {
    let mut session = EditingSession::new(source, cursor);
    session.apply(intent)
}

pub struct EditingSession {
    pub view: MarkdownEditorView,
    pub source: String,
    pub cursor: usize,
    pub selection: Option<std::ops::Range<usize>>,
    pub generation: u32,
}

impl EditingSession {
    pub fn new(source: &str, cursor: usize) -> Self {
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        Self { view, source: source.to_owned(), cursor, selection: None, generation: 1 }
    }

    pub fn apply(&mut self, intent: EditIntent) -> (String, usize) {
        let request = EditRequest {
            source_generation: self.generation,
            cursor_byte: self.cursor,
            selection: self.selection.clone(),
            intent,
        };
        let (edited, cursor) = apply_plan(&self.source, &request, self.view.plan_edit(&request));
        if self.source != edited {
            self.generation += 1;
            self.view.set_source(edited.clone(), self.generation);
        }
        self.source = edited;
        self.cursor = cursor;
        self.selection = None;
        assert!(self.source.is_char_boundary(cursor));
        assert!(
            cursor == self.source.len()
                || self.source.grapheme_indices(true).any(|(offset, _)| offset == cursor),
            "caret must remain on a grapheme boundary"
        );
        (self.source.clone(), self.cursor)
    }
}

fn apply_plan(source: &str, request: &EditRequest, plan: EditPlan) -> (String, usize) {
    let cursor = request.cursor_byte;
    let mut edited = source.to_owned();
    match plan {
        EditPlan::Apply(transaction) | EditPlan::ApplyDefault(transaction, _) => {
            for replacement in transaction.replacements.iter().rev() {
                edited.replace_range(replacement.range.clone(), &replacement.text);
            }
            let EditSelection::Caret(cursor) = transaction.selection_after else {
                panic!("audit edits must produce a caret");
            };
            assert!(edited.is_char_boundary(cursor));
            (edited, cursor)
        }
        EditPlan::Consume => (edited, cursor),
        EditPlan::MoveCursor(update) => (edited, update.cursor_after),
        EditPlan::SetSelection(_) => panic!("audit edits must produce a caret"),
        EditPlan::UseDefault => {
            let selection = request.selection.clone().filter(|range| !range.is_empty());
            let (range, inserted) = match &request.intent {
                EditIntent::InsertParagraphBreak | EditIntent::InsertLineBreak => {
                    let newline = if source.contains("\r\n") { "\r\n" } else { "\n" };
                    (selection.clone().unwrap_or(cursor..cursor), newline.to_owned())
                }
                EditIntent::DeleteBackward => {
                    let previous = source[..cursor]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(cursor, |(start, _)| start);
                    (selection.clone().unwrap_or(previous..cursor), String::new())
                }
                EditIntent::DeleteForward => {
                    let width = source[cursor..].graphemes(true).next().map_or(0, str::len);
                    (selection.clone().unwrap_or(cursor..cursor + width), String::new())
                }
                EditIntent::InsertText(text) => (selection.unwrap_or(cursor..cursor), text.clone()),
                _ => panic!("unsupported audit intent"),
            };
            let after_cursor = range.start + inserted.len();
            edited.replace_range(range, &inserted);
            (edited, after_cursor)
        }
    }
}

pub fn empty_anchors(source: &str) -> Vec<usize> {
    fn collect(blocks: &[BlockNode], anchors: &mut Vec<usize>) {
        for block in blocks {
            if block.is_editable_empty_paragraph() {
                anchors.push(block.block_range.start);
            }
            collect(&block.children, anchors);
        }
    }
    let document = MarkdownDoc::build_for_editing(
        &parse_markdown(source),
        &MarkdownStyle::from_theme(&ui::theme::test_theme(), FONT_SIZE, LINE_HEIGHT),
        source,
    );
    let mut anchors = Vec::new();
    collect(&document.blocks, &mut anchors);
    anchors
}

pub fn atomic_signature(source: &str) -> Vec<MarkdownEvent> {
    parse_markdown(source)
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                MarkdownEvent::Rule | MarkdownEvent::Start(MarkdownTag::CodeBlock { .. })
            )
        })
        .collect()
}

pub fn content_signature(source: &str) -> Vec<MarkdownEvent> {
    parse_markdown(source)
        .events
        .into_iter()
        .filter(|event| match event {
            MarkdownEvent::Text(text) => !text.trim().is_empty(),
            MarkdownEvent::Code(_)
            | MarkdownEvent::InlineMath(_)
            | MarkdownEvent::DisplayMath(_)
            | MarkdownEvent::InlineHtml(_)
            | MarkdownEvent::TaskListMarker(_) => true,
            _ => false,
        })
        .collect()
}

pub fn caret_is_on_whitespace_only_source_line(source: &str, cursor: usize) -> bool {
    let start = source[..cursor].rfind('\n').map_or(0, |newline| newline + 1);
    let end = source[cursor..].find('\n').map_or(source.len(), |newline| cursor + newline);
    source[start..end].chars().all(char::is_whitespace)
}

pub struct AuditDocument(pub String);
impl DocView for AuditDocument {
    fn line_count(&self) -> usize {
        StringDocView::new(&self.0).line_count()
    }
    fn doc_line_text(&self, line: usize) -> Cow<'_, str> {
        Cow::Owned(StringDocView::new(&self.0).doc_line_text(line).into_owned())
    }
    fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> Cow<'_, str> {
        Cow::Borrowed(&self.0[range])
    }
    fn line_byte_offset(&self, line: usize) -> usize {
        StringDocView::new(&self.0).line_byte_offset(line)
    }
    fn line_byte_length(&self, line: usize) -> usize {
        StringDocView::new(&self.0).line_byte_length(line)
    }
    fn scroll_y(&self) -> f32 {
        0.0
    }
    fn viewport_height(&self) -> f32 {
        VIEWPORT_HEIGHT
    }
}
impl DocViewMut for AuditDocument {
    fn set_scroll_y(&mut self, _: f32) {}
    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.0.replace_range(range, text);
    }
}

pub fn blank_paragraph_count(source: &str) -> usize {
    fn count(blocks: &[BlockNode]) -> usize {
        blocks
            .iter()
            .map(|block| {
                usize::from(
                    matches!(block.kind, textora_markdown::builder::BlockKind::Paragraph)
                        && block.text_lines.len() == 1
                        && block.text_lines[0].chars().all(char::is_whitespace),
                ) + count(&block.children)
            })
            .sum()
    }
    let style = MarkdownStyle::from_theme(&ui::theme::test_theme(), FONT_SIZE, LINE_HEIGHT);
    let document = MarkdownDoc::build_for_editing(&parse_markdown(source), &style, source);
    count(&document.blocks)
}
