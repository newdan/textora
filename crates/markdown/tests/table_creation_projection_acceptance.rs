extern crate core as doc_core;

use std::borrow::Cow;

use doc_core::document::{DocView, DocViewMut, StringDocView};
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::{EditPolicy, PluginMessage, ViewPlugin};

struct TestDocument(String);

impl DocView for TestDocument {
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
        600.0
    }
}

impl DocViewMut for TestDocument {
    fn set_scroll_y(&mut self, _: f32) {}

    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.0.replace_range(range, text);
    }
}

fn create_empty_table(columns: usize, rows: usize) -> (String, usize) {
    let plan = textora_markdown::commands::plan_semantic_edit(
        "",
        1,
        0,
        None,
        ui::plugin::SemanticEditCommand::InsertTable { columns, rows },
    );
    let ui::plugin::SemanticEditPlan::Apply(transaction) = plan else {
        panic!("empty document must accept a table insertion");
    };
    let replacement = transaction.replacements.first().expect("replacement exists");
    let mut source = String::new();
    source.replace_range(replacement.range.clone(), &replacement.text);
    let ui::plugin::EditSelection::Caret(cursor_byte) = transaction.selection_after else {
        panic!("table insertion must place a caret");
    };
    (source, cursor_byte)
}

fn render(view: &mut MarkdownEditorView, document: &TestDocument, shaper: &mut shaping::Shaper) {
    view.render(
        document,
        ui::Rect::new(0.0, 0.0, 800.0, 600.0),
        &ui::theme::test_theme(),
        shaper,
        1.0,
    );
}

#[test]
fn created_blank_table_is_projected_and_each_cell_can_be_hit_and_edited() {
    let (source, _) = create_empty_table(2, 3);
    let parsed = textora_markdown::parser::parse_markdown(&source);
    assert!(parsed.events.iter().any(|event| matches!(
        event,
        textora_markdown::parser::MarkdownEvent::Start(
            textora_markdown::parser::MarkdownTag::Table(alignments)
        ) if alignments.len() == 2
    )));

    let mut document = TestDocument(source.clone());
    let mut view = MarkdownEditorView::new();
    let mut shaper = shaping::Shaper::new().expect("shaper should initialize");
    view.set_source(source.clone(), 1);
    render(&mut view, &document, &mut shaper);
    assert!(view.engine().flat_lines().len() >= 3, "header and two body rows should be visible");

    let table_lines = source.lines().collect::<Vec<_>>();
    for row_index in [0, 2, 3] {
        let row_line = table_lines[row_index];
        let delimiter_bytes = row_line.match_indices('|').map(|(byte, _)| byte).collect::<Vec<_>>();
        assert_eq!(delimiter_bytes.len(), 3);
        for (column_index, delimiter_byte) in delimiter_bytes.iter().take(2).enumerate() {
            let cell_byte = document.line_byte_offset(row_index) + *delimiter_byte + 2;
            view.handle_message(PluginMessage::SetCursorByte(cell_byte), &mut document);
            render(&mut view, &document, &mut shaper);
            let (cursor_x, cursor_y, _, cursor_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("every empty cell must expose a caret position");
            let hit =
                view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0);
            assert_eq!(
                hit,
                Some(cell_byte),
                "empty cell ({row_index}, {column_index}) must hit-test"
            );

            let edit_plan = view.plan_edit(&ui::plugin::EditRequest {
                intent: ui::plugin::EditIntent::InsertText("x".to_owned()),
                source_generation: 1,
                cursor_byte: cell_byte,
                selection: None,
            });
            let mut edited_source = source.clone();
            match edit_plan {
                ui::plugin::EditPlan::Apply(transaction) => {
                    assert_eq!(transaction.replacements.len(), 1);
                    let replacement =
                        transaction.replacements.first().expect("cell replacement exists");
                    edited_source.replace_range(replacement.range.clone(), &replacement.text);
                }
                ui::plugin::EditPlan::UseDefault => edited_source.insert(cell_byte, 'x'),
                other => panic!(
                    "typing into an empty cell must apply or use default insertion, got {other:?}"
                ),
            }
            let edited = textora_markdown::parser::parse_markdown(&edited_source);
            assert!(
                edited.events.iter().any(|event| matches!(
                    event,
                    textora_markdown::parser::MarkdownEvent::Start(
                        textora_markdown::parser::MarkdownTag::Table(alignments)
                    ) if alignments.len() == 2
                )),
                "editing cell ({row_index}, {column_index}) must preserve the table: {edited_source:?}"
            );
            let mut inside_table_cell = false;
            let mut edited_cell_contains_text = false;
            for event in &edited.events {
                match event {
                    textora_markdown::parser::MarkdownEvent::Start(
                        textora_markdown::parser::MarkdownTag::TableCell,
                    ) => inside_table_cell = true,
                    textora_markdown::parser::MarkdownEvent::End(
                        textora_markdown::parser::MarkdownTagEnd::TableCell,
                    ) => inside_table_cell = false,
                    textora_markdown::parser::MarkdownEvent::Text(text)
                        if inside_table_cell && text == "x" =>
                    {
                        edited_cell_contains_text = true;
                    }
                    _ => {}
                }
            }
            assert!(
                edited_cell_contains_text,
                "inserted text must belong to a parsed table cell: {edited_source:?}"
            );
        }
    }
}
