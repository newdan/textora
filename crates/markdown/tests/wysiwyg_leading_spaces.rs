use textora_markdown::augmenter::augment_edit;
use textora_markdown::parser::{MarkdownEvent, MarkdownTag, parse_markdown};
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::AugmentKind;
use ui::plugin::{EditIntent, EditPlan, EditPolicy, EditRequest, EditSelection};

fn type_text(source: &str, cursor: usize, text: &str) -> (String, usize) {
    let mut edited = source.to_owned();
    if let Some(augmentation) = augment_edit(source, cursor, AugmentKind::InsertText(text.into())) {
        edited.replace_range(
            augmentation.replace_range.unwrap_or(cursor..cursor),
            augmentation.insert_text.as_deref().unwrap_or_default(),
        );
        assert!(edited.is_char_boundary(augmentation.cursor_byte_after));
        (edited, augmentation.cursor_byte_after)
    } else {
        edited.insert_str(cursor, text);
        (edited, cursor + text.len())
    }
}

fn assert_plain_paragraph(source: &str, expected_text: &str) {
    let parsed = parse_markdown(source);
    assert!(parsed.events.contains(&MarkdownEvent::Start(MarkdownTag::Paragraph)));
    assert!(
        !parsed
            .events
            .iter()
            .any(|event| { matches!(event, MarkdownEvent::Start(MarkdownTag::CodeBlock { .. })) })
    );
    let rendered: String = parsed
        .events
        .iter()
        .filter_map(|event| match event {
            MarkdownEvent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(rendered.replace('\u{a0}', " "), expected_text);
}

#[test]
fn wysiwyg_four_spaces_then_chinese_remain_a_paragraph_after_reparse() {
    let mut source = String::new();
    let mut cursor = 0;
    for _ in 0..4 {
        (source, cursor) = type_text(&source, cursor, " ");
    }
    assert_plain_paragraph(&source, "    ");
    (source, cursor) = type_text(&source, cursor, "中文正文");
    assert_plain_paragraph(&source, "    中文正文");
    assert_eq!(cursor, source.len());
}

#[test]
fn wysiwyg_spaces_before_existing_paragraph_do_not_create_code() {
    let mut source = "正文".to_owned();
    let mut cursor = 0;
    for _ in 0..4 {
        (source, cursor) = type_text(&source, cursor, " ");
    }
    assert_plain_paragraph(&source, "    正文");
    assert_eq!(&source[cursor..], "正文");
}

#[test]
fn wysiwyg_preserves_existing_blank_indentation_when_typing_text() {
    let (source, cursor) = type_text("    ", 4, "正文");
    assert_plain_paragraph(&source, "    正文");
    assert_eq!(cursor, source.len());
}

#[test]
fn wysiwyg_spaces_materialize_an_empty_paragraph_between_blocks() {
    for newline in ["\n", "\r\n"] {
        let original = format!("first{newline}{newline}{newline}second");
        let cursor = "first".len() + newline.len() * 2;
        let (source, cursor) = type_text(&original, cursor, "    ");
        let (source, _) = type_text(&source, cursor, "正文");
        assert_plain_paragraph(&source, "first    正文second");
        assert_eq!(source.matches(&format!("{newline}{newline}")).count(), 2);
    }
}

#[test]
fn wysiwyg_preserves_spaces_after_caret_in_an_empty_paragraph() {
    let (source, cursor) = type_text("   ", 1, " 文");
    assert_plain_paragraph(&source, "  文  ");
    assert_eq!(source[cursor..].replace('\u{a0}', " "), "  ");
}

#[test]
fn wysiwyg_existing_code_and_container_indentation_stay_literal() {
    for (source, cursor) in [
        ("    code", 4),
        ("    code", 0),
        ("```rust\n\n```", "```rust\n".len()),
        ("    code\n\n    more", "    code\n".len()),
        ("- item\n  continuation", "- item\n  ".len()),
        ("> quote", 2),
    ] {
        let (edited, _) = type_text(source, cursor, "    ");
        let mut expected = source.to_owned();
        expected.insert_str(cursor, "    ");
        assert_eq!(edited, expected, "fixture: {source:?}");
    }
}

#[test]
fn wysiwyg_fence_and_multiline_markdown_paste_still_create_code() {
    for text in ["```", "    code\n    more"] {
        let (source, _) = type_text("", 0, text);
        assert!(
            parse_markdown(&source).events.iter().any(|event| {
                matches!(event, MarkdownEvent::Start(MarkdownTag::CodeBlock { .. }))
            })
        );
    }
}

#[test]
fn wysiwyg_mid_paragraph_and_trailing_spaces_are_unchanged() {
    for cursor in ["正".len(), "正文".len()] {
        let (source, _) = type_text("正文", cursor, "    ");
        let mut expected = "正文".to_owned();
        expected.insert_str(cursor, "    ");
        assert_eq!(source, expected);
    }
}

#[test]
fn wysiwyg_replacing_selected_paragraph_with_leading_spaces_is_atomic() {
    let original = "旧段落";
    let mut view = MarkdownEditorView::new();
    view.set_source(original.into(), 1);
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: original.len(),
        selection: Some(0..original.len()),
        intent: EditIntent::InsertText("    新段落".into()),
    };
    let EditPlan::Apply(transaction) = view.plan_edit(&request) else {
        panic!("selected paragraph replacement must preserve literal leading spaces");
    };
    assert_eq!(transaction.replacements.len(), 1);
    assert_eq!(transaction.source_generation, 1);
    let replacement = &transaction.replacements[0];
    let mut edited = original.to_owned();
    edited.replace_range(replacement.range.clone(), &replacement.text);
    assert_plain_paragraph(&edited, "    新段落");
    assert_eq!(transaction.selection_after, EditSelection::Caret(edited.len()));
}

#[test]
fn wysiwyg_replacing_selected_code_retains_default_literal_editing() {
    let original = "    code";
    let mut view = MarkdownEditorView::new();
    view.set_source(original.into(), 1);
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: original.len(),
        selection: Some(4..original.len()),
        intent: EditIntent::InsertText("    replacement".into()),
    };
    assert_eq!(view.plan_edit(&request), EditPlan::UseDefault);
}
