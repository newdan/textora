use super::*;
use ui::plugin::EditPolicy;

#[test]
fn pipe_header_enter_creates_a_parseable_table_and_places_caret_in_first_body_cell() {
    for (header, expected_columns) in
        [("| 名称 | 说明 | 备注 |", 3), ("| 名称 | 说明 \\| 备注 |", 2), ("| 名称 | `a|b` |", 2)]
    {
        let cursor = header.len();
        let augmentation = augment_edit(header, cursor, AugmentKind::Enter)
            .expect("pipe header Enter must be augmented");
        let result = apply_augmentation_at(header, cursor, &augmentation);
        let parsed = crate::parser::parse_markdown(&result);
        let parsed_columns = parsed.events.iter().find_map(|event| match event {
            crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(columns)) => {
                Some(columns.len())
            }
            _ => None,
        });
        let table_cell_count = parsed
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::TableCell)
                )
            })
            .count();

        assert_eq!(parsed_columns, Some(expected_columns), "{result:?}");
        assert_eq!(table_cell_count, expected_columns * 2, "{result:?}");
        assert!(result.starts_with(&escape_inline_code_pipes(header)));
        if header.contains("`a|b`") {
            assert!(parsed.events.iter().any(|event| {
                matches!(event, crate::parser::MarkdownEvent::Code(text) if text == "a|b")
            }));
        }
        assert_eq!(
            &result[augmentation.cursor_byte_after..augmentation.cursor_byte_after + 1],
            " "
        );
    }
    assert_eq!(pipe_header_column_count("| 名称 | `a|b` |"), Some(2));
    assert_eq!(pipe_header_column_count("| 名称 | 说明 \\| 备注 |"), Some(2));
}

#[test]
fn escaped_pipe_inside_inline_code_keeps_parser_table_columns() {
    let source = "| 名称 | `a\\|b` |\n| --- | --- |\n|  |  |";
    let parsed = crate::parser::parse_markdown(source);
    let column_count = parsed.events.iter().find_map(|event| match event {
        crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(columns)) => {
            Some(columns.len())
        }
        _ => None,
    });
    let code_text = parsed.events.iter().find_map(|event| match event {
        crate::parser::MarkdownEvent::Code(text) => Some(text.as_str()),
        _ => None,
    });

    assert_eq!(column_count, Some(2), "{parsed:?}");
    assert_eq!(code_text, Some("a|b"));
}

#[test]
fn pipe_header_recognition_requires_unescaped_outer_boundaries() {
    assert_eq!(pipe_header_column_count("| A | B \\"), None);
    assert_eq!(pipe_header_column_count("| `A|B` |"), Some(1));
    assert_eq!(pipe_header_column_count("| `A\\|B` |"), Some(1));
}

#[test]
fn pipe_header_enter_preserves_adjacent_blocks_and_crlf_newlines() {
    for (source, expected_newline) in
        [("before\n\n| A | B |\n\nafter", "\n"), ("before\r\n\r\n| A | B |\r\n\r\nafter", "\r\n")]
    {
        let cursor = source.find("| A | B |").expect("header exists") + "| A | B |".len();
        assert!(has_pipe_header_shape_at_cursor(source, cursor));
        let context = classify_enter_context(source, cursor);
        assert!(matches!(context, EnterContext::TopLevelParagraphEnd), "{context:?}");
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .expect("pipe header Enter must create a table");
        let result = apply_augmentation_at(source, cursor, &augmentation);
        assert!(result.starts_with("before"));
        assert!(result.ends_with(if expected_newline == "\n" {
            "\n\nafter"
        } else {
            "\r\n\r\nafter"
        }));
        assert!(
            result.contains(&format!("{expected_newline}| --- | --- |{expected_newline}|  |  |")),
            "{result:?}"
        );
        assert_eq!(parsed_table_count(&result, 2), 1);
    }
}

#[test]
fn pipe_header_enter_preserves_blockquote_and_list_prefixes() {
    for (source, cursor, expected) in [
        ("> | A | B |", "> | A | B |".len(), "> | A | B |\n> | --- | --- |\n> |  |  |"),
        ("> | A | B |\r\n", "> | A | B |".len(), "> | A | B |\r\n> | --- | --- |\r\n> |  |  |\r\n"),
        ("- | A | B |", "- | A | B |".len(), "- | A | B |\n  | --- | --- |\n  |  |  |"),
        ("- | A | B |\r\n", "- | A | B |".len(), "- | A | B |\r\n  | --- | --- |\r\n  |  |  |\r\n"),
    ] {
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .expect("container pipe header Enter should be augmented");
        let result = apply_augmentation_at(source, cursor, &augmentation);
        let parsed_columns =
            crate::parser::parse_markdown(&result).events.iter().find_map(|event| match event {
                crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(columns)) => {
                    Some(columns.len())
                }
                _ => None,
            });

        assert_eq!(result, expected);
        assert_eq!(parsed_columns, Some(2), "{result:?}");
        assert_eq!(parsed_table_count(&result, 2), 1);
        assert!(result[augmentation.cursor_byte_after..].starts_with(' '));
    }
}

#[test]
fn pipe_header_enter_rejects_plain_text_nonterminal_cursor_and_code_contexts() {
    for (source, cursor) in [
        ("甲 | 乙", "甲 | 乙".len()),
        ("| 甲 | 乙 |", 2),
        ("```\n| 甲 | 乙 |\n```", "```\n| 甲 | 乙 |".len()),
        ("| 甲 | 乙 |\n| --- | --- |\n| x | y |", "| 甲 | 乙 |\n| --- | --- |\n| x | y |".len()),
    ] {
        let result = augment_edit(source, cursor, AugmentKind::Enter).map_or_else(
            || source.to_owned(),
            |augmentation| apply_augmentation_at(source, cursor, &augmentation),
        );
        let parsed = crate::parser::parse_markdown(&result);
        let table_count = parsed
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(_))
                )
            })
            .count();
        let original_table_count = crate::parser::parse_markdown(source)
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(_))
                )
            })
            .count();
        assert_eq!(table_count, original_table_count, "must not create a table for {source:?}");
    }
}

#[test]
fn pipe_header_enter_adds_table_beside_an_existing_table_with_same_width() {
    let source = "| 新表 | `a|b` |\n\n| 现有 | 表格 |\n| --- | --- |\n| x | y |";
    let cursor = "| 新表 | `a|b` |".len();
    let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
        .expect("pipe header before an existing table must be recognized");
    let result = apply_augmentation_at(source, cursor, &augmentation);

    assert_eq!(parsed_table_count(&result, 2), 2, "{result:?}");
    assert!(result.starts_with("| 新表 | `a\\|b` |"));
}

#[test]
fn table_tab_moves_to_next_cell_in_visual_row_order() {
    let source = "| H1 | H2 |\n| --- | --- |\n| A | B |\n| C | D |";
    for (current_text, next_text) in [("H1", "H2"), ("H2", "A"), ("A", "B"), ("B", "C"), ("C", "D")]
    {
        let current_byte = source.find(current_text).expect("current cell exists");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Tab)
            .expect("Tab in a table cell must navigate");

        assert_eq!(
            augmentation.cursor_byte_after,
            source.find(next_text).expect("next cell exists")
        );
        assert!(augmentation.insert_text.as_deref().unwrap_or_default().is_empty());
    }
}

#[test]
fn table_shift_tab_moves_to_previous_cell_across_table_rows() {
    let source = "| H1 | H2 |\n| --- | --- |\n| A | B |";
    let mut view = crate::view::MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    let request = ui::plugin::EditRequest {
        source_generation: 1,
        cursor_byte: source.find("A").expect("body cell exists"),
        selection: None,
        intent: ui::plugin::EditIntent::Outdent,
    };

    assert!(matches!(view.plan_edit(&request), ui::plugin::EditPlan::MoveCursor(_)));
}

#[test]
fn table_last_cell_tab_appends_a_parseable_row_and_places_caret_in_its_first_cell() {
    let source = "| H1 | H2 |\n| --- | --- |\n| A | B |";
    let current_byte = source.find("B").expect("last cell exists");
    let augmentation = augment_edit(source, current_byte, AugmentKind::Tab)
        .expect("Tab in the last cell must append a body row");
    let result = apply_augmentation_at(source, current_byte, &augmentation);
    let parsed = crate::parser::parse_markdown(&result);

    assert_eq!(parsed_table_count(&result, 2), 1);
    assert_eq!(
        parsed
            .events
            .iter()
            .filter(|event| matches!(
                event,
                crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::TableRow)
            ))
            .count(),
        2,
        "{result:?}"
    );
    assert_eq!(result[augmentation.cursor_byte_after..].chars().next(), Some(' '));
}

#[test]
fn tab_on_header_only_table_appends_the_first_body_row() {
    let source = "| H1 | H2 |\n| :--- | ---: |";
    let current_byte = source.find("H2").expect("last header cell exists");
    let augmentation = augment_edit(source, current_byte, AugmentKind::Tab)
        .expect("Tab on the last header cell must create the first body row");
    let result = apply_augmentation_at(source, current_byte, &augmentation);
    let parsed = crate::parser::parse_markdown(&result);

    assert_eq!(parsed_table_count(&result, 2), 1);
    assert_eq!(
        parsed
            .events
            .iter()
            .filter(|event| matches!(
                event,
                crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::TableRow)
            ))
            .count(),
        1,
        "{result:?}"
    );
    assert!(result.contains("| :--- | ---: |"), "{result:?}");
}

#[test]
fn table_text_insertion_escapes_pipes_and_flattens_cell_newlines() {
    let source = "| H1 | H2 |\n| --- | --- |\n| A | B |";
    let current_byte = source.find("A").expect("body cell exists") + 1;
    let augmentation =
        augment_edit(source, current_byte, AugmentKind::InsertText("x|y\nz".to_owned()))
            .expect("table-cell input must be sanitized");
    let result = apply_augmentation_at(source, current_byte, &augmentation);

    assert!(result.contains("Ax\\|y z |"), "{result:?}");
    assert_eq!(parsed_table_count(&result, 2), 1);
}

#[test]
fn table_header_input_preserves_the_divider_row() {
    let source = "| H1 | H2 |\n| :--- | ---: |\n| A | B |";
    let current_byte = source.find("H1").expect("header cell exists") + 2;
    let augmentation =
        augment_edit(source, current_byte, AugmentKind::InsertText("x|y\r\nz".to_owned()))
            .expect("table header input must be sanitized");
    let result = apply_augmentation_at(source, current_byte, &augmentation);

    assert!(result.starts_with("| H1x\\|y z | H2 |\n| :--- | ---: |"), "{result:?}");
    assert_eq!(parsed_table_count(&result, 2), 1);
}

#[test]
fn table_paste_replacing_selected_cell_text_flattens_newlines_and_escapes_pipes() {
    let source = "| H1 | H2 |\n| --- | --- |\n| A | B |";
    let selected_cell_text = source.find('A').expect("body cell exists");
    let selected_range = selected_cell_text..selected_cell_text + 1;
    let pasted_text = "left|right\nnext";
    let augmentation = augment_selected_text(source, selected_range.clone(), pasted_text)
        .expect("pasting into a selected table cell must be sanitized");
    let mut result = source.to_owned();
    result.replace_range(selected_range, augmentation.insert_text.as_deref().unwrap_or_default());

    assert!(result.contains("left\\|right next"), "{result:?}");
    assert_eq!(parsed_table_count(&result, 2), 1);
}

#[test]
fn tab_outside_a_table_uses_the_existing_editor_fallback() {
    let source = "ordinary paragraph";

    assert!(augment_edit(source, source.len(), AugmentKind::Tab).is_none());
    assert!(augment_edit(source, source.len(), AugmentKind::ShiftTab).is_none());
}

fn reset_classify_parse_count() {
    CLASSIFY_PARSE_COUNT.with(|parse_count| parse_count.set(0));
}

fn classify_parse_count() -> usize {
    CLASSIFY_PARSE_COUNT.with(Cell::get)
}

fn apply_augmentation_at(
    source: &str,
    current_byte: usize,
    augmentation: &EditAugmentation,
) -> String {
    let replacement_range =
        augmentation.replace_range.clone().unwrap_or(current_byte..current_byte);
    let mut edited_source = source.to_owned();
    edited_source
        .replace_range(replacement_range, augmentation.insert_text.as_deref().unwrap_or(""));
    edited_source
}

fn apply_backspace_with_default(source: &str, current_byte: usize) -> (String, usize) {
    if let Some(augmentation) = augment_edit(source, current_byte, AugmentKind::Backspace) {
        return (
            apply_augmentation_at(source, current_byte, &augmentation),
            augmentation.cursor_byte_after,
        );
    }

    let (grapheme_start, _) = UnicodeSegmentation::grapheme_indices(&source[..current_byte], true)
        .next_back()
        .expect("Backspace fixture must have a preceding grapheme");
    let mut edited_source = source.to_owned();
    edited_source.replace_range(grapheme_start..current_byte, "");
    (edited_source, grapheme_start)
}

fn apply_delete_forward_with_default(source: &str, current_byte: usize) -> (String, usize) {
    if let Some(augmentation) = augment_delete_forward(source, current_byte) {
        return (
            apply_augmentation_at(source, current_byte, &augmentation),
            augmentation.cursor_byte_after,
        );
    }

    let grapheme = UnicodeSegmentation::graphemes(&source[current_byte..], true)
        .next()
        .expect("Delete fixture must have a following grapheme");
    let mut edited_source = source.to_owned();
    edited_source.replace_range(current_byte..current_byte + grapheme.len(), "");
    (edited_source, current_byte)
}

#[test]
fn code_html_literal_backspace_removes_one_character() {
    for (source, current_byte, expected) in [
        ("```\n<br>\n```", "```\n<br>".len(), "```\n<br\n```"),
        ("`<br>`", "`<br>".len(), "`<br`"),
        (r"\<br>", r"\<br>".len(), r"\<br"),
    ] {
        let (edited, _) = apply_backspace_with_default(source, current_byte);
        assert_eq!(edited, expected, "failed for {source:?}");
    }
}

#[test]
fn code_html_literal_delete_removes_one_character() {
    for (source, current_byte, expected) in [
        ("```\n<br>\n```", "```\n".len(), "```\nbr>\n```"),
        ("`<br>`", 1, "`br>`"),
        (r"\<br>", 1, r"\br>"),
    ] {
        let (edited, _) = apply_delete_forward_with_default(source, current_byte);
        assert_eq!(edited, expected, "failed for {source:?}");
    }
}

#[cfg(feature = "rich-markdown")]
#[test]
fn inline_html_break_deletion_removes_the_complete_element() {
    for html_break in ["<br>", "<br/>", "<br />"] {
        let source = format!("before{html_break}after");
        let break_start = "before".len();
        let break_end = break_start + html_break.len();

        let (backspaced, backspace_cursor) = apply_backspace_with_default(&source, break_end);
        assert_eq!(backspaced, "beforeafter");
        assert_eq!(backspace_cursor, break_start);

        let (deleted, delete_cursor) = apply_delete_forward_with_default(&source, break_start);
        assert_eq!(deleted, "beforeafter");
        assert_eq!(delete_cursor, break_start);
    }
}

fn assert_empty_paragraph_text_roundtrip(
    source: &str,
    current_byte: usize,
    expected_source: &str,
    expected_cursor: usize,
) {
    let insertion = augment_edit(source, current_byte, AugmentKind::InsertText(String::from("新")))
        .expect("typing in an empty paragraph must preserve its sibling slots");
    let typed_source = apply_augmentation_at(source, current_byte, &insertion);

    assert_eq!(typed_source, expected_source, "insertion mismatch for {source:?}");
    assert_eq!(insertion.cursor_byte_after, expected_cursor);

    let (restored_source, restored_cursor) =
        apply_backspace_with_default(&typed_source, insertion.cursor_byte_after);
    assert_eq!(restored_source, source, "deletion mismatch for {source:?}");
    assert_eq!(restored_cursor, current_byte);
}

fn editable_paragraph_slots(source: &str) -> Vec<(Vec<usize>, usize)> {
    let parsed = crate::parser::parse_markdown(source);
    let document = crate::builder::MarkdownDoc::build_structure(&parsed);
    let paragraphs = crate::builder::EditableParagraphMap::from_blocks(&document.blocks, source);
    paragraphs
        .runs()
        .iter()
        .flat_map(|run| {
            run.lines[run.hidden_separator_count..]
                .iter()
                .map(|line| (run.owner_path.clone(), line.source_byte))
        })
        .collect()
}

fn assert_empty_paragraph_enter_backspace_sequence(
    source: &str,
    current_byte: usize,
    expected_first_enter: &str,
    expected_second_enter: &str,
    expected_owner_path: &[usize],
) {
    let initial_slots = editable_paragraph_slots(source);
    assert_eq!(initial_slots, [(expected_owner_path.to_vec(), current_byte)]);

    let first_enter = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a real empty paragraph must add one sibling slot");
    let after_first = apply_augmentation_at(source, current_byte, &first_enter);
    let first_slots = editable_paragraph_slots(&after_first);
    assert_eq!(after_first, expected_first_enter);
    assert_eq!(first_slots.len(), 2);
    assert_eq!(first_enter.cursor_byte_after, first_slots[1].1);
    assert!(first_slots.iter().all(|(owner, _)| owner == expected_owner_path));

    let second_enter =
        augment_edit(&after_first, first_enter.cursor_byte_after, AugmentKind::Enter)
            .expect("repeated Enter must add one more sibling slot");
    let after_second =
        apply_augmentation_at(&after_first, first_enter.cursor_byte_after, &second_enter);
    let second_slots = editable_paragraph_slots(&after_second);
    assert_eq!(after_second, expected_second_enter);
    assert_eq!(second_slots.len(), 3);
    assert_eq!(second_enter.cursor_byte_after, second_slots[2].1);
    assert!(second_slots.iter().all(|(owner, _)| owner == expected_owner_path));

    let (restored_first, restored_first_cursor) =
        apply_backspace_with_default(&after_second, second_enter.cursor_byte_after);
    assert_eq!(
        (restored_first, restored_first_cursor),
        (after_first.clone(), first_enter.cursor_byte_after)
    );
    let (restored_source, restored_cursor) =
        apply_backspace_with_default(&after_first, first_enter.cursor_byte_after);
    assert_eq!((restored_source, restored_cursor), (source.to_owned(), current_byte));
}

#[test]
fn regular_text_insertion_skips_document_classification_parse() {
    reset_classify_parse_count();
    let source = "first paragraph\n\nsecond paragraph";

    let augmentation =
        augment_edit(source, "first".len(), AugmentKind::InsertText(String::from("x")));

    assert!(augmentation.is_none());
    assert_eq!(classify_parse_count(), 0);
}

#[test]
fn padded_top_level_empty_paragraph_enter_backspace_preserves_source_and_slots() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("head{newline} {newline} {newline}tail");
        let current_byte = format!("head{newline} {newline}").len();
        let after_first = format!("head{newline} {newline} {newline}{newline}tail");
        let after_second = format!("head{newline} {newline} {newline}{newline}{newline}tail");

        assert_empty_paragraph_enter_backspace_sequence(
            &source,
            current_byte,
            &after_first,
            &after_second,
            &[],
        );
    }
}

#[test]
fn trailing_single_empty_paragraph_enter_accounts_for_new_hidden_separator() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("head{newline}");
        let current_byte = source.len();
        let initial_slots = editable_paragraph_slots(&source);
        assert_eq!(initial_slots, [(Vec::new(), current_byte)]);

        let enter = augment_edit(&source, current_byte, AugmentKind::Enter)
            .expect("Enter must add a trailing empty paragraph slot");
        let edited_source = apply_augmentation_at(&source, current_byte, &enter);
        let expected_source = format!("head{newline}{newline}{newline}");
        let entered_slots = editable_paragraph_slots(&edited_source);
        assert_eq!(edited_source, expected_source);
        assert_eq!(entered_slots.len(), initial_slots.len() + 1);
        assert_eq!(enter.cursor_byte_after, entered_slots[1].1);

        let (after_backspace, backspace_cursor) =
            apply_backspace_with_default(&edited_source, enter.cursor_byte_after);
        let remaining_slots = editable_paragraph_slots(&after_backspace);
        assert_eq!(remaining_slots.len(), initial_slots.len());
        assert_eq!(backspace_cursor, remaining_slots[0].1);
    }
}

#[test]
fn blockquote_empty_paragraph_enter_backspace_stays_in_owner() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("> first{newline}>{newline}>{newline}> second");
        let current_byte = format!("> first{newline}>{newline}>").len();
        let after_first = format!("> first{newline}>{newline}>{newline}>{newline}> second");
        let after_second =
            format!("> first{newline}>{newline}>{newline}>{newline}>{newline}> second");

        assert_empty_paragraph_enter_backspace_sequence(
            &source,
            current_byte,
            &after_first,
            &after_second,
            &[0],
        );
    }
}

#[test]
fn all_empty_blockquote_with_multiple_slots_keeps_navigating_in_its_owner() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!(">{newline}>{newline}>");
        let current_byte = source.len();
        let initial_slots = editable_paragraph_slots(&source);
        assert_eq!(initial_slots.len(), 3);
        assert!(initial_slots.iter().all(|(owner, _)| owner == &[0]));

        let enter = augment_edit(&source, current_byte, AugmentKind::Enter)
            .expect("a multi-slot empty quote must keep Enter inside its owner");
        let edited_source = apply_augmentation_at(&source, current_byte, &enter);
        let expected_source = format!(">{newline}>{newline}>{newline}>");
        let entered_slots = editable_paragraph_slots(&edited_source);
        assert_eq!(edited_source, expected_source);
        assert_eq!(entered_slots.len(), initial_slots.len() + 1);
        assert_eq!(enter.cursor_byte_after, entered_slots[3].1);

        let (restored_source, restored_cursor) =
            apply_backspace_with_default(&edited_source, enter.cursor_byte_after);
        assert_eq!((restored_source, restored_cursor), (source, current_byte));
    }
}

#[test]
fn list_empty_paragraph_enter_backspace_stays_in_owner() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("- first{newline}{newline}{newline}  second");
        let current_byte = format!("- first{newline}{newline}").len();
        let after_first = format!("- first{newline}{newline}{newline}{newline}  second");
        let after_second = format!("- first{newline}{newline}{newline}{newline}{newline}  second");

        assert_empty_paragraph_enter_backspace_sequence(
            &source,
            current_byte,
            &after_first,
            &after_second,
            &[0],
        );
    }
}

#[test]
fn regular_mid_line_backspace_skips_document_classification_parse() {
    reset_classify_parse_count();
    let source = "first paragraph\n\nsecond paragraph";

    let augmentation = augment_edit(source, "first".len(), AugmentKind::Backspace);

    assert!(augmentation.is_none());
    assert_eq!(classify_parse_count(), 0);
}

#[test]
fn classification_guards_allow_special_edit_candidates() {
    reset_classify_parse_count();
    let separator_source = "first\n\nsecond";
    let separator_byte = "first\n".len();

    let insertion =
        augment_edit(separator_source, separator_byte, AugmentKind::InsertText(String::from("x")));

    assert!(insertion.is_some());
    assert_eq!(classify_parse_count(), 1);

    reset_classify_parse_count();
    let interblock_source = "first\n\n中\n\nsecond";
    let paragraph_end = "first\n\n中".len();

    let backspace = augment_edit(interblock_source, paragraph_end, AugmentKind::Backspace);

    assert!(backspace.is_some());
    assert_eq!(classify_parse_count(), 1);
}

#[test]
fn parses_supported_list_marker_boundaries() {
    assert_eq!(parse_list_marker("42. item", 0), Some((ListBullet::Ordered(42), 4)));
    assert_eq!(parse_list_marker("-\titem", 0), Some((ListBullet::Bullet, 2)));
    assert_eq!(parse_list_marker("-   item", 0), Some((ListBullet::Bullet, 4)));
    assert_eq!(parse_list_marker("10.\titem", 0), Some((ListBullet::Ordered(10), 4)));
    assert_eq!(parse_list_marker("- [X] done", 0), Some((ListBullet::TaskList(true), 6)));
    assert_eq!(parse_list_marker("-   [ ]\titem", 0), Some((ListBullet::TaskList(false), 8)));
    assert_eq!(parse_list_marker("+ item", 0), Some((ListBullet::Bullet, 2)));
    assert_eq!(parse_list_marker("* item", 0), Some((ListBullet::Bullet, 2)));
    assert_eq!(parse_list_marker("7)\titem", 0), Some((ListBullet::Ordered(7), 3)));
    assert_eq!(parse_list_marker("- [ ] item", 0), Some((ListBullet::TaskList(false), 6)));
    assert_eq!(parse_list_marker("1234567890. item", 0), None);
}

#[test]
fn list_enter_normalizes_task_markers_without_a_content_separator() {
    let cases = ["- [ ]todo", "- [x]done", "- [X]done"];

    for source in cases {
        let augmentation = augment_edit(source, source.len(), AugmentKind::Enter)
            .expect("Enter should continue a task list item");

        assert_eq!(
            augmentation.insert_text.as_deref(),
            Some("\n- [ ] "),
            "continued task marker must remain valid for {source:?}"
        );
    }
}

#[test]
fn enter_after_top_level_horizontal_rule_creates_editable_paragraph() {
    for source in ["---", "***", "___", "before\n\n---", "---\n"] {
        let rule_end = source.rfind(['-', '*', '_']).expect("fixture has a rule") + 1;
        let augmentation = augment_edit(source, rule_end, AugmentKind::Enter)
            .expect("Enter after a horizontal rule must create a paragraph");
        let edited_source = apply_augmentation_at(source, rule_end, &augmentation);

        assert_eq!(augmentation.cursor_byte_after, rule_end + 2, "{source:?}");
        assert_eq!(&edited_source[rule_end..rule_end + 2], "\n\n", "{source:?}");
    }
}

#[test]
fn paragraph_enter_before_soft_break_preserves_following_source_line() {
    let source = "first line\nsecond line";
    let current_byte = "first line".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split the paragraph");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n\n"));
    assert_eq!(augmentation.cursor_byte_after, current_byte + 2);
    assert_eq!(edited_source, "first line\n\n\nsecond line");
}

#[test]
fn paragraph_enter_before_crlf_soft_break_preserves_crlf_line_endings() {
    let source = "first line\r\nsecond line";
    let current_byte = "first line".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split before a CRLF soft break");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\r\n\r\n"));
    assert_eq!(augmentation.cursor_byte_after, current_byte + 4);
    assert_eq!(edited_source, "first line\r\n\r\n\r\nsecond line");
}

#[test]
fn enter_at_double_space_hard_break_promotes_it_to_a_block_boundary() {
    let source = "first  \nsecond";
    let current_byte = "first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a hard break should split the paragraph in two");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\n\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\n\n".len());
}

#[test]
fn enter_at_backslash_hard_break_promotes_it_to_a_block_boundary() {
    let source = "first\\\nsecond";
    let current_byte = "first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a hard break should split the paragraph in two");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\n\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\n\n".len());
}

#[test]
fn enter_at_odd_backslash_hard_break_preserves_escaped_backslashes() {
    let source = "first\\\\\\\nsecond";
    let current_byte = "first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should consume only the final unescaped backslash");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\\\\\n\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\\\\\n\n".len());
}

#[test]
fn enter_at_crlf_hard_break_keeps_crlf_line_endings() {
    let source = "first  \r\nsecond";
    let current_byte = "first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a CRLF hard break should split the paragraph in two");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\r\n\r\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\r\n\r\n".len());
}

#[test]
fn enter_before_even_backslashes_keeps_soft_break_semantics() {
    let source = "first\\\\\nsecond";
    let current_byte = "first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter before escaped backslashes should use paragraph splitting");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\n\n\\\\\nsecond");
}

#[test]
fn heading_enter_before_single_newline_creates_editable_empty_paragraph() {
    let source = "# Heading\nparagraph";
    let current_byte = "# Heading".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("heading-end Enter should create an empty paragraph");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n\n"));
    assert_eq!(augmentation.cursor_byte_after, current_byte + 2);
    assert_eq!(edited_source, "# Heading\n\n\nparagraph");
}

#[test]
fn backspace_at_start_of_space_split_paragraph_restores_original_space() {
    let original_source = "hello world";
    let split_byte = "hello ".len();
    let enter_augmentation = augment_edit(original_source, split_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split at the cursor");
    let split_source = apply_augmentation_at(original_source, split_byte, &enter_augmentation);

    let backspace_augmentation =
        augment_edit(&split_source, enter_augmentation.cursor_byte_after, AugmentKind::Backspace)
            .expect("Backspace at the second paragraph start should join both paragraphs");
    let restored_source = apply_augmentation_at(
        &split_source,
        enter_augmentation.cursor_byte_after,
        &backspace_augmentation,
    );

    assert_eq!(restored_source, original_source);
    assert_eq!(backspace_augmentation.cursor_byte_after, split_byte);
}

#[test]
fn backspace_at_paragraph_start_after_heading_joins_heading() {
    let source = "# Heading\n\nparagraph";
    let current_byte = "# Heading\n\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace at a block start should remove the complete source boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "# Headingparagraph");
    assert_eq!(augmentation.cursor_byte_after, "# Heading".len());
}

#[test]
fn backspace_at_atx_heading_marker_start_is_consumed() {
    let source = "previous\n# Heading";
    let marker_start = "previous\n".len();

    let augmentation = augment_edit(source, marker_start, AugmentKind::Backspace)
        .expect("heading marker start must guard the preceding source boundary");

    assert_eq!(augmentation.insert_text.as_deref(), Some(""));
    assert_eq!(augmentation.replace_range, None);
    assert_eq!(augmentation.cursor_byte_after, marker_start);
}

#[test]
fn enter_then_backspace_restores_plain_paragraph_and_heading_cases() {
    let cases = [
        ("hello", "hello".len()),
        ("# hello world", "# he".len()),
        ("# Heading", "# Heading".len()),
        ("first\n\nsecond", "first".len()),
        ("# Heading\n\nparagraph", "# Heading".len()),
    ];

    for (source, current_byte) in cases {
        let enter_augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter must handle fixture {source:?}"));
        let edited_source = apply_augmentation_at(source, current_byte, &enter_augmentation);
        let backspace_augmentation = augment_edit(
            &edited_source,
            enter_augmentation.cursor_byte_after,
            AugmentKind::Backspace,
        )
        .unwrap_or_else(|| panic!("Backspace must reverse Enter for {source:?}"));
        let restored_source = apply_augmentation_at(
            &edited_source,
            enter_augmentation.cursor_byte_after,
            &backspace_augmentation,
        );

        assert_eq!(restored_source, source, "Enter/Backspace mismatch for {source:?}");
        assert_eq!(backspace_augmentation.cursor_byte_after, current_byte);
    }
}

#[test]
fn backspace_after_repeated_trailing_enter_removes_only_latest_empty_line() {
    let source = "first";
    let first_enter = augment_edit(source, source.len(), AugmentKind::Enter)
        .expect("first Enter should create a trailing paragraph");
    let after_first_enter = apply_augmentation_at(source, source.len(), &first_enter);
    let second_enter =
        augment_edit(&after_first_enter, first_enter.cursor_byte_after, AugmentKind::Enter)
            .expect("second Enter should add one trailing empty line");
    let after_second_enter =
        apply_augmentation_at(&after_first_enter, first_enter.cursor_byte_after, &second_enter);

    let backspace =
        augment_edit(&after_second_enter, second_enter.cursor_byte_after, AugmentKind::Backspace)
            .expect("Backspace should remove the latest trailing empty line");
    let restored =
        apply_augmentation_at(&after_second_enter, second_enter.cursor_byte_after, &backspace);

    assert_eq!(restored, after_first_enter);
    assert_eq!(backspace.cursor_byte_after, first_enter.cursor_byte_after);
}

#[test]
fn backspace_between_blocks_reverses_three_enters_one_at_a_time() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let initial_source = format!("前段{newline}{newline}后段");
        let initial_cursor = "前段".len();
        let mut edit_states = vec![(initial_source, initial_cursor)];

        for _ in 0..3 {
            let (source, cursor) = edit_states.last().expect("the initial edit state must exist");
            let enter = augment_edit(source, *cursor, AugmentKind::Enter)
                .expect("Enter between blocks must create an editable empty paragraph");
            edit_states
                .push((apply_augmentation_at(source, *cursor, &enter), enter.cursor_byte_after));
        }

        for expected_state_index in (0..3).rev() {
            let (source, cursor) = edit_states.last().expect("the latest edit state must exist");
            let backspace = augment_edit(source, *cursor, AugmentKind::Backspace)
                .expect("Backspace must reverse the nearest Enter");
            let edited_source = apply_augmentation_at(source, *cursor, &backspace);
            let expected_state = &edit_states[expected_state_index];

            assert_eq!(edited_source, expected_state.0, "source mismatch for {newline:?}");
            assert_eq!(
                backspace.cursor_byte_after, expected_state.1,
                "cursor mismatch for {newline:?}"
            );
            edit_states.pop();
        }
    }
}

#[test]
fn backspace_in_blank_run_before_atomic_block_keeps_nearest_empty_line() {
    let cases = [
        ("前段\n\n\n\n---", "前段\n\n\n".len(), "前段\n\n\n---", "前段\n\n".len()),
        (
            "# 标题\r\n\r\n\r\n\r\n```",
            "# 标题\r\n\r\n\r\n".len(),
            "# 标题\r\n\r\n\r\n```",
            "# 标题\r\n\r\n".len(),
        ),
    ];

    for (source, current_byte, expected_source, expected_cursor) in cases {
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace in the blank run must remove one empty line");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, expected_source);
        assert_eq!(augmentation.cursor_byte_after, expected_cursor);
    }
}

#[test]
fn backspace_on_fenced_code_blank_line_keeps_default_physical_newline_edit() {
    let source = "```\nfirst\n\n\nsecond\n```";
    let current_byte = "```\nfirst\n\n".len();

    assert!(matches!(classify_enter_context(source, current_byte), EnterContext::CodeBlock));
    assert!(
        augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
        "Backspace inside fenced code must delete the preceding physical newline by default"
    );
}

#[test]
fn backspace_without_preceding_block_removes_only_one_empty_slot() {
    let cases = [
        ("\n\n", 2, "\n", 1),
        ("\n\n\nhead", 2, "\n\nhead", 1),
        ("\r\n\r\n", 4, "\r\n", 2),
        ("\r\n\r\n\r\nhead", 4, "\r\n\r\nhead", 2),
    ];

    for (source, current_byte, expected_source, expected_cursor) in cases {
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace must remove only the nearest leading empty slot");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, expected_source, "source mismatch for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, expected_cursor);
    }
}

#[test]
fn list_enter_preserves_marker_style_and_resets_completed_tasks() {
    let cases = [
        ("* item", "\n* "),
        ("+ item", "\n+ "),
        ("7) item", "\n8) "),
        ("-\titem", "\n-\t"),
        ("- [x] done", "\n- [ ] "),
        ("- [ ]\ttodo", "\n- [ ]\t"),
    ];

    for (source, expected_insert) in cases {
        let augmentation = augment_edit(source, source.len(), AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter should continue list item {source:?}"));

        assert_eq!(
            augmentation.insert_text.as_deref(),
            Some(expected_insert),
            "continuation marker mismatch for {source:?}"
        );
    }
}

#[test]
fn backspace_removes_markers_that_use_whitespace_separators() {
    for source in ["-\t", ">\t", "1.\t", "1.  ", "1. \t"] {
        let augmentation = augment_edit(source, source.len(), AugmentKind::Backspace)
            .unwrap_or_else(|| panic!("Backspace should remove marker {source:?}"));

        assert_eq!(augmentation.replace_range, Some(0..source.len()));
        assert_eq!(augmentation.cursor_byte_after, 0);
    }
}

#[test]
fn backspace_at_soft_line_start_removes_single_source_newline() {
    let source = "first line\nsecond line";
    let current_byte = "first line\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace at a soft line start should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first linesecond line");
    assert_eq!(augmentation.replace_range, Some("first line".len()..current_byte));
    assert_eq!(augmentation.cursor_byte_after, "first line".len());
}

#[test]
fn backspace_after_backslash_hard_break_removes_the_marker() {
    let source = "first\\\nsecond";
    let current_byte = "first\\\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a backslash hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "first".len());
}

#[test]
fn backspace_after_double_space_hard_break_removes_the_marker() {
    let source = "first  \nsecond";
    let current_byte = "first  \n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a double-space hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "first".len());
}

#[test]
fn backspace_after_crlf_hard_break_removes_the_complete_boundary() {
    let source = "first  \r\nsecond";
    let current_byte = "first  \r\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a CRLF hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "first".len());
}

#[test]
fn backspace_after_odd_backslash_hard_break_preserves_escaped_backslashes() {
    let source = "first\\\\\\\nsecond";
    let current_byte = "first\\\\\\\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace should remove only the final unescaped backslash");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\\\\second");
    assert_eq!(augmentation.cursor_byte_after, "first\\\\".len());
}

#[test]
fn backspace_keeps_non_hard_break_trailing_characters() {
    let cases = [("first \nsecond", "first second"), ("first\\\\\nsecond", "first\\\\second")];

    for (source, expected_source) in cases {
        let current_byte = source.find("second").expect("fixture must contain second");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace at a soft line start should join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, expected_source);
    }
}

#[test]
fn backspace_at_crlf_paragraph_start_removes_complete_boundary() {
    let source = "first\r\n\r\nsecond";
    let current_byte = "first\r\n\r\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace should treat CRLF sequences as complete source newlines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "firstsecond");
    assert_eq!(augmentation.replace_range, Some("first".len()..current_byte));
    assert_eq!(augmentation.cursor_byte_after, "first".len());
}

#[test]
fn backspace_on_crlf_empty_line_removes_complete_newline_sequence() {
    let source = "first\r\n\r\nsecond";
    let current_byte = "first\r\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace on a CRLF empty line should use the empty-line edit");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\r\nsecond");
    assert_eq!(augmentation.replace_range, Some(current_byte..current_byte + 2));
    assert_eq!(augmentation.cursor_byte_after, "first".len());
}

#[test]
fn insert_text_in_crlf_empty_separator_preserves_block_boundaries() {
    let source = "first\r\n\r\nsecond";
    let current_byte = "first\r\n".len();

    let augmentation =
        augment_edit(source, current_byte, AugmentKind::InsertText(String::from("中")))
            .expect("typing on a CRLF separator line should create an editable paragraph");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "first\r\n\r\n中\r\n\r\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\r\n\r\n中".len());
}

#[test]
fn insert_text_in_leading_empty_lines_preserves_every_slot_and_is_reversible() {
    let cases = [
        ("\n\nhead", 0, "新\n\n\nhead", "新".len()),
        ("\n\nhead", 1, "\n新\n\nhead", "\n新".len()),
        ("\r\n\r\nhead", 0, "新\r\n\r\n\r\nhead", "新".len()),
        ("\r\n\r\nhead", 2, "\r\n新\r\n\r\nhead", "\r\n新".len()),
    ];

    for (source, current_byte, expected_source, expected_cursor) in cases {
        assert_empty_paragraph_text_roundtrip(
            source,
            current_byte,
            expected_source,
            expected_cursor,
        );
    }
}

#[test]
fn insert_text_in_each_pure_empty_document_line_preserves_all_lines() {
    let cases = [
        ("\n\n", 0, "新\n\n\n", "新".len()),
        ("\n\n", 1, "\n新\n\n", "\n新".len()),
        ("\n\n", 2, "\n\n新", "\n\n新".len()),
        ("\r\n\r\n", 0, "新\r\n\r\n\r\n", "新".len()),
        ("\r\n\r\n", 2, "\r\n新\r\n\r\n", "\r\n新".len()),
        ("\r\n\r\n", 4, "\r\n\r\n新", "\r\n\r\n新".len()),
    ];

    for (source, current_byte, expected_source, expected_cursor) in cases {
        assert_empty_paragraph_text_roundtrip(
            source,
            current_byte,
            expected_source,
            expected_cursor,
        );
    }
}

#[test]
fn insert_text_in_each_interblock_empty_slot_preserves_the_complete_run() {
    let cases = [
        ("head\n\n\n\nfoot", "head\n\n".len(), "head\n\n新\n\n\nfoot"),
        ("head\n\n\n\nfoot", "head\n\n\n".len(), "head\n\n\n新\n\nfoot"),
        ("head\r\n\r\n\r\n\r\nfoot", "head\r\n\r\n".len(), "head\r\n\r\n新\r\n\r\n\r\nfoot"),
        ("head\r\n\r\n\r\n\r\nfoot", "head\r\n\r\n\r\n".len(), "head\r\n\r\n\r\n新\r\n\r\nfoot"),
    ];

    for (source, current_byte, expected_source) in cases {
        assert_empty_paragraph_text_roundtrip(
            source,
            current_byte,
            expected_source,
            current_byte + "新".len(),
        );
    }
}

#[test]
fn insert_text_in_first_trailing_empty_line_preserves_every_slot_and_is_reversible() {
    let cases = [
        ("head\n\n\n\n", "head\n\n".len(), "head\n\n新\n\n\n"),
        ("head\r\n\r\n\r\n\r\n", "head\r\n\r\n".len(), "head\r\n\r\n新\r\n\r\n\r\n"),
    ];

    for (source, current_byte, expected_source) in cases {
        assert_empty_paragraph_text_roundtrip(
            source,
            current_byte,
            expected_source,
            current_byte + "新".len(),
        );
    }
}

#[test]
fn insert_text_with_source_newline_keeps_default_semantics_on_empty_lines() {
    for text in ["\n", "\r\n", "新\n行"] {
        assert!(
            augment_edit("\n\nhead", 0, AugmentKind::InsertText(String::from(text))).is_none(),
            "newline insertion must not use empty-paragraph normalization: {text:?}"
        );
    }
}

#[test]
fn list_enter_in_crlf_document_continues_with_crlf() {
    let source = "- first\r\n- second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("list Enter should continue the current CRLF list item");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\r\n- "));
    assert_eq!(edited_source, "- first\r\n- \r\n- second");
}

#[test]
fn empty_list_enter_in_crlf_document_preserves_following_newline() {
    let source = "- \r\nnext";
    let current_byte = "- ".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should exit an empty CRLF list item");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "\r\nnext");
    assert_eq!(augmentation.cursor_byte_after, 0);
}

#[test]
fn empty_blockquote_enter_in_crlf_document_exits_quote() {
    let source = "> \r\nnext";
    let current_byte = "> ".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should exit an empty CRLF blockquote line");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "\r\nnext");
    assert_eq!(augmentation.cursor_byte_after, 0);
}

#[test]
fn indented_blockquote_enter_preserves_indent_and_marker() {
    let source = "  > quote";
    let current_byte = source.len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should continue a blockquote indented by up to three spaces");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n  > "));
    assert_eq!(edited_source, "  > quote\n  > ");
}

#[test]
fn nested_blockquote_enter_preserves_complete_marker_prefix() {
    let source = "> > quote";
    let current_byte = source.len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should continue every active blockquote level");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n> > "));
    assert_eq!(edited_source, "> > quote\n> > ");
}

#[test]
fn backspace_after_non_text_block_uses_existing_specialized_or_fallback_behavior() {
    // 列表边界已由显式并入测试覆盖（M4），此处只保留仍回退默认计划的叶块。
    let cases = ["---\n\nparagraph", "```\ncode\n```\n\nparagraph"];

    for source in cases {
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

        assert!(
            augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
            "generic paragraph joining must not consume a {source:?} boundary"
        );
    }
}

#[test]
fn pulldown_cmark_emits_heading_event_for_setext_heading() {
    use pulldown_cmark::{Event, Parser, Tag};

    let source = "Title\n===\n";
    let heading_ranges: Vec<std::ops::Range<usize>> =
        Parser::new_ext(source, pulldown_cmark::Options::all())
            .into_offset_iter()
            .filter_map(|(event, range)| {
                matches!(event, Event::Start(Tag::Heading { .. })).then_some(range)
            })
            .collect();

    assert_eq!(heading_ranges.len(), 1, "setext heading must surface as a heading event");
    let heading_range = &heading_ranges[0];
    assert!(
        !source[heading_range.start..].starts_with('#'),
        "setext heading source does not start with an ATX marker: {:?}",
        &source[heading_range.clone()]
    );
}

#[test]
fn setext_heading_is_classified_as_setext_not_atx() {
    let source = "Title\n===";
    let underline_end = source.len();

    assert!(
        matches!(
            classify_enter_context(source, "Title".len()),
            EnterContext::SetextHeading { underline_end: end, .. } if end == underline_end
        ),
        "setext title text must not use ATX heading semantics"
    );
    assert!(
        matches!(
            classify_enter_context(source, source.len()),
            EnterContext::SetextHeading { underline_end: end, .. } if end == underline_end
        ),
        "setext underline must not use ATX heading semantics"
    );
}

#[test]
fn setext_heading_enter_creates_editable_paragraph_after_the_underline() {
    let source = "Title\n=====\npara";
    let current_byte = "Title".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside a setext heading must preserve the heading source");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "Title\n=====\n\n\npara");
    assert_eq!(augmentation.cursor_byte_after, "Title\n=====\n\n".len());
}

#[test]
fn setext_heading_enter_from_the_underline_line_appends_after_it() {
    let source = "Title\n-----\npara";
    let current_byte = "Title\n-----".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a setext underline must preserve the heading source");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "Title\n-----\n\n\npara");
    assert_eq!(augmentation.cursor_byte_after, "Title\n-----\n\n".len());
}

#[test]
fn setext_heading_enter_at_document_end_preserves_existing_empty_paragraph() {
    let source = "Title\n=====\n";
    let current_byte = "Title".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside a trailing setext heading must preserve the heading source");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "Title\n=====\n\n\n");
    assert_eq!(augmentation.cursor_byte_after, "Title\n=====\n\n".len());
}

#[test]
fn definition_list_is_classified_the_way_the_renderer_lays_it_out() {
    let source = "term\n: definition";

    assert!(
        matches!(classify_enter_context(source, source.len()), EnterContext::TopLevelParagraphEnd),
        "editing classifier and renderer must share parser options"
    );
}

#[test]
fn indented_code_block_enter_continues_the_current_line_indent() {
    let source = "    let x = 1;";
    let current_byte = source.len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside an indented code block should continue the indent");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n    "));
    assert_eq!(edited_source, "    let x = 1;\n    ");
    assert_eq!(augmentation.cursor_byte_after, edited_source.len());
}

#[test]
fn indented_code_block_enter_preserves_tab_indent() {
    let source = "\tlet x = 1;";
    let current_byte = source.len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside a tab-indented code block should continue the indent");

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n\t"));
}

#[test]
fn indented_code_block_blank_line_inherits_nearest_code_indent() {
    let source = "    first\n\n    second";
    let current_byte = "    first\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on an internal blank code line should inherit code indentation");
    let after_enter = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n    "));
    assert!(
        augment_edit(
            &after_enter,
            augmentation.cursor_byte_after,
            AugmentKind::InsertText(String::from("y")),
        )
        .is_none(),
        "typing after Enter on a blank code line must stay in the code block"
    );
}

#[test]
fn fenced_code_block_enter_still_uses_the_default_plan() {
    let source = "```\nlet x = 1;\n```";
    let current_byte = "```\nlet x = 1;".len();

    assert!(
        augment_edit(source, current_byte, AugmentKind::Enter).is_none(),
        "fenced code blocks keep using the default single-newline plan"
    );
}

#[test]
fn table_enter_moves_to_the_next_cell_content_start() {
    let source = "| a |\n|---|\n| b |";
    let current_byte = source.find('a').expect("fixture must contain the first cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside a table cell should move to the cell below");

    assert_eq!(augmentation.replace_range, None);
    assert_eq!(augmentation.insert_text.as_deref(), Some(""));
    assert_eq!(
        augmentation.cursor_byte_after,
        source.rfind('b').expect("fixture must contain the cell below")
    );
}

#[test]
fn table_enter_into_an_empty_cell_stops_at_the_cell_end() {
    let source = "| a |\n|---|\n|   |";
    let current_byte = source.find('a').expect("fixture must contain the first cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside a table cell should move to the cell below");

    assert_eq!(
        augmentation.cursor_byte_after,
        source.rfind('|').expect("fixture must contain a closing pipe")
    );
}

#[test]
fn backspace_merging_paragraph_into_unmergeable_leaf_block_is_noop() {
    let cases = [
        "```\ncode\n```\nparagraph",
        "    code\nparagraph",
        "---\nparagraph",
        "Title\n===\nparagraph",
    ];

    for source in cases {
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .unwrap_or_else(|| {
                panic!("leaf-block boundary in {source:?} must be guarded, not fall back")
            });
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, source, "guarded backspace must not change {source:?}");
        assert_eq!(
            augmentation.cursor_byte_after, current_byte,
            "guarded backspace must keep the cursor in {source:?}"
        );
    }
}

#[test]
fn backspace_guard_for_leaf_block_boundary_preserves_crlf_source() {
    let source = "```\r\ncode\r\n```\r\nparagraph";
    let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("CRLF leaf-block boundary must be guarded");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, source);
    assert_eq!(augmentation.cursor_byte_after, current_byte);
}

#[test]
fn backspace_across_blank_line_before_leaf_block_keeps_default_fallback() {
    let cases = ["```\ncode\n```\n\nparagraph", "    code\n\nparagraph", "---\n\nparagraph"];

    for source in cases {
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

        assert!(
            augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
            "removing one newline of a blank separator before a leaf block is safe: {source:?}"
        );
    }
}

#[test]
fn backspace_at_paragraph_start_after_blockquote_merges_with_explicit_marker() {
    let cases = [
        ("> quote\n\nparagraph", "> quote\n> paragraph", "> quote\n> "),
        ("> > quote\n\nparagraph", "> > quote\n> > paragraph", "> > quote\n> > "),
        ("  > quote\n\nparagraph", "  > quote\n  > paragraph", "  > quote\n  > "),
    ];

    for (source, expected_source, expected_cursor_prefix) in cases {
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .unwrap_or_else(|| {
                panic!("blockquote boundary in {source:?} must merge with an explicit marker")
            });
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, expected_source, "merge mismatch for {source:?}");
        assert_eq!(
            augmentation.cursor_byte_after,
            expected_cursor_prefix.len(),
            "cursor must land before the merged paragraph content for {source:?}"
        );
    }
}

#[test]
fn backspace_at_paragraph_start_after_list_item_merges_as_continuation_line() {
    let cases = [
        ("- item\n\nparagraph", "- item\n  paragraph", "- item\n  "),
        ("1. item\n\nparagraph", "1. item\n   paragraph", "1. item\n   "),
        ("- [x] done\n\nparagraph", "- [x] done\n      paragraph", "- [x] done\n      "),
        (
            "- outer\n  - inner\n\nparagraph",
            "- outer\n  - inner\n    paragraph",
            "- outer\n  - inner\n    ",
        ),
    ];

    for (source, expected_source, expected_cursor_prefix) in cases {
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .unwrap_or_else(|| {
                panic!("list boundary in {source:?} must merge as a continuation line")
            });
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, expected_source, "merge mismatch for {source:?}");
        assert_eq!(
            augmentation.cursor_byte_after,
            expected_cursor_prefix.len(),
            "cursor must land at the content column for {source:?}"
        );
    }
}

#[test]
fn backspace_merging_into_blockquote_or_list_preserves_crlf_line_endings() {
    let cases = [
        ("> quote\r\n\r\nparagraph", "> quote\r\n> paragraph", "> quote\r\n> "),
        ("- item\r\n\r\nparagraph", "- item\r\n  paragraph", "- item\r\n  "),
    ];

    for (source, expected_source, expected_cursor_prefix) in cases {
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .unwrap_or_else(|| panic!("CRLF boundary in {source:?} must merge explicitly"));
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, expected_source, "merge mismatch for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, expected_cursor_prefix.len());
    }
}

#[test]
fn deleting_last_interblock_paragraph_grapheme_restores_one_editable_blank_line() {
    let source = "first\n\n中\n\nsecond";
    let cursor_byte = "first\n\n中".len();

    let augmentation = augment_edit(source, cursor_byte, AugmentKind::Backspace)
        .expect("deleting the final paragraph grapheme should preserve one editable blank line");
    let replacement = augmentation.replace_range.expect("final grapheme deletion needs a range");
    let mut result = source.to_string();
    result.replace_range(replacement, augmentation.insert_text.as_deref().unwrap_or(""));

    assert_eq!(result, "first\n\n\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\n\n".len());
}

#[test]
fn deleting_last_interblock_paragraph_grapheme_preserves_crlf_boundaries() {
    let source = "first\r\n\r\n中\r\n\r\nsecond";
    let cursor_byte = "first\r\n\r\n中".len();

    let augmentation = augment_edit(source, cursor_byte, AugmentKind::Backspace)
        .expect("CRLF paragraphs should normalize the final grapheme deletion");
    let replacement = augmentation.replace_range.expect("final grapheme deletion needs a range");
    let mut result = source.to_string();
    result.replace_range(replacement, augmentation.insert_text.as_deref().unwrap_or(""));

    assert_eq!(result, "first\r\n\r\n\r\nsecond");
    assert_eq!(augmentation.cursor_byte_after, "first\r\n\r\n".len());
}

#[test]
fn enter_on_blank_line_between_loose_list_items_inserts_plain_newline() {
    // 松散列表 item 的 range 含块间空行；命中测试用去尾换行的 content_end，
    // 因此空行不再属于前一 item，Enter 只加宽块间距而非续 `- ` marker。
    let source = "- a\n\n- b";
    let current_byte = "- a\n".len();

    assert!(
        matches!(
            classify_enter_context(source, current_byte),
            EnterContext::EmptyBlockSeparatorLine
        ),
        "blank separator line must not be classified as part of the preceding list item"
    );
    assert!(
        matches!(classify_enter_context(source, "- a".len()), EnterContext::ListItem { .. }),
        "the item content end must still continue the list marker"
    );

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a blank separator line should emit a plain newline");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n"));
    assert_eq!(augmentation.replace_range, None);
    assert_eq!(augmentation.cursor_byte_after, current_byte + 1);
    assert_eq!(edited_source, "- a\n\n\n- b");
}

#[test]
fn enter_on_blank_line_after_code_block_inserts_plain_newline() {
    // 代码块 range 的结尾换行统一按去尾换行归一化；其后的空行是块分隔行，
    // Enter 只加宽块间距，不落入 CodeBlock（无增强）语义。
    let source = "```\ncode\n```\n\npara";
    let current_byte = "```\ncode\n```\n".len();

    assert!(
        matches!(
            classify_enter_context(source, current_byte),
            EnterContext::EmptyBlockSeparatorLine
        ),
        "blank line after a code block must not be classified as CodeBlock"
    );

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a blank separator line should emit a plain newline");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n"));
    assert_eq!(augmentation.replace_range, None);
    assert_eq!(augmentation.cursor_byte_after, current_byte + 1);
    assert_eq!(edited_source, "```\ncode\n```\n\n\npara");
}

#[test]
fn list_enter_at_backslash_hard_break_promotes_it_to_a_new_item() {
    let source = "- first\\\n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a list hard break should split into two items");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- first\n- second");
    assert_eq!(augmentation.cursor_byte_after, "- first\n- ".len());
}

#[test]
fn list_enter_before_blank_separator_preserves_source_for_each_marker() {
    for newline in ["\n", "\r\n"] {
        for separator in ["", " ", "\t"] {
            for (first, marker) in
                [("- first", "- "), ("3. first", "4. "), ("- [x] first", "- [ ] ")]
            {
                let suffix = format!("{newline}{separator}{newline}  following");
                let source = format!("{first}{suffix}");
                let augmentation = augment_edit(&source, first.len(), AugmentKind::Enter)
                    .expect("Enter must create a new list item before the separator");
                let edited = apply_augmentation_at(&source, first.len(), &augmentation);
                assert_eq!(edited, format!("{first}{newline}{marker}{suffix}"));
                assert_eq!(
                    augmentation.cursor_byte_after,
                    first.len() + newline.len() + marker.len()
                );
            }
        }
    }
}

#[test]
fn list_enter_at_odd_backslash_hard_break_preserves_escaped_backslashes() {
    let source = "- first\\\\\\\n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at an odd backslash run should keep escaped backslashes");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- first\\\\\n- second");
}

#[test]
fn list_enter_at_double_space_hard_break_promotes_it_to_a_new_item() {
    let source = "- first  \n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a double-space list hard break should split into two items");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- first\n- second");
}

#[test]
fn quote_enter_at_backslash_hard_break_promotes_it_to_an_explicit_line() {
    let source = "> first\\\n> second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a quote hard break should continue with an explicit marker");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> second");
    assert_eq!(augmentation.cursor_byte_after, "> first\n> ".len());
}

#[test]
fn quote_enter_at_crlf_hard_break_keeps_crlf_line_endings() {
    let source = "> first\\\r\n> second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a CRLF quote hard break should keep CRLF");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\r\n> second");
}

#[test]
fn list_enter_before_even_backslashes_does_not_treat_them_as_hard_break() {
    let source = "- first\\\\\n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("even backslashes are not a hard break");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_ne!(edited_source, "- first\n- second");
    assert!(edited_source.contains('\\'), "escaped backslashes must remain: {edited_source:?}");
}

#[test]
fn list_backspace_after_backslash_hard_break_joins_visual_lines() {
    let source = "- first\\\n  second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a list hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "- first".len());
}

#[test]
fn list_backspace_after_double_space_hard_break_joins_visual_lines() {
    let source = "- first  \n  second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a double-space list hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- firstsecond");
}

#[test]
fn quote_backspace_after_backslash_hard_break_joins_visual_lines() {
    let source = "> first\\\n> second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a quote hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "> first".len());
}

#[test]
fn quote_backspace_after_crlf_hard_break_removes_the_complete_boundary() {
    let source = "> first\\\r\n> second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("CRLF quote hard break backspace should drop the whole boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> firstsecond");
}

#[test]
fn backspace_does_not_join_hard_break_across_a_new_list_item() {
    let source = "- first\\\n- second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace);
    let edited_source = augmentation
        .as_ref()
        .map(|aug| apply_augmentation_at(source, current_byte, aug))
        .unwrap_or_else(|| source.to_owned());

    assert_ne!(
        edited_source, "- firstsecond",
        "a following sibling item must not be glued onto the previous item"
    );
}

#[test]
fn backspace_does_not_strip_a_new_nested_container_after_a_hard_break() {
    for source in ["plain\\\n> quote", "- item\\\n  > quote"] {
        let current_byte = source.find("quote").expect("fixture contains quote content");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace);
        let edited_source = augmentation
            .as_ref()
            .map(|augmentation| apply_augmentation_at(source, current_byte, augmentation))
            .unwrap_or_else(|| source.to_owned());

        assert!(
            edited_source.contains('>'),
            "Backspace must not erase a structurally new quote marker in {source:?}"
        );
    }
}

#[test]
fn backspace_joins_a_hard_break_when_the_nested_container_path_matches() {
    let source = "- > first\\\n  > second";
    let current_byte = source.find("second").expect("fixture contains continuation content");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("matching nested container paths should be joinable");

    assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), "- > firstsecond");
}

#[test]
fn list_enter_at_lazy_continuation_newline_becomes_a_new_item() {
    let source = "- item\npara";
    let current_byte = "- item".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a lazy continuation newline should start a new item");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- item\n- para");
    assert_eq!(augmentation.cursor_byte_after, "- item\n- ".len());
}

#[test]
fn list_enter_after_lazy_continuation_newline_prefixes_the_following_line() {
    let source = "- item\npara";
    let current_byte = "- item\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter after a lazy continuation newline should mark the following line");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- item\n- para");
    assert_eq!(augmentation.cursor_byte_after, "- item\n- ".len());
}

#[test]
fn quote_enter_at_lazy_continuation_newline_inserts_an_explicit_marker() {
    let source = "> first\nsecond";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a lazy quote newline should add an explicit marker");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> second");
    assert_eq!(augmentation.cursor_byte_after, "> first\n> ".len());
}

#[test]
fn quote_enter_between_explicit_lines_still_inserts_a_quote_line() {
    let source = "> first\n> second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter between explicit quote lines should insert a quoted line");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> \n> second");
    assert_eq!(augmentation.cursor_byte_after, "> first\n> ".len());
}

#[test]
fn paragraph_enter_before_a_single_space_preserves_a_reversible_separator() {
    let source = "left right";
    let current_byte = "left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split at the word boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "left \n\nright");
    assert_eq!(augmentation.cursor_byte_after, "left \n\n".len());
}

#[test]
fn paragraph_enter_after_a_single_space_preserves_a_reversible_separator() {
    let source = "left right";
    let current_byte = "left ".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split at the word boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "left \n\nright");
}

#[test]
fn heading_enter_before_a_single_space_preserves_a_reversible_separator() {
    let source = "# left right";
    let current_byte = "# left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("heading interior Enter should split at the word boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "# left \nright");
}

#[test]
fn paragraph_enter_inside_bold_closes_and_reopens_the_inline_element() {
    let source = "**left right**";
    let current_byte = "**left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside bold text should preserve both styled fragments");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "**left** \n\n**right**");
    assert_eq!(augmentation.cursor_byte_after, "**left** \n\n**".len());
}

#[test]
fn paragraph_enter_inside_nested_inline_elements_preserves_marker_order() {
    let source = "**left *nested text* right**";
    let current_byte = "**left *nested".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside nested inline styles should close and reopen every frame");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "**left *nested*** \n\n***text* right**");
}

#[test]
fn paragraph_enter_inside_link_duplicates_the_link_wrapper() {
    let source = "[left right](https://example.com)";
    let current_byte = "[left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside link text should preserve a valid link on both sides");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "[left](https://example.com) \n\n[right](https://example.com)");
}

#[test]
fn paragraph_enter_inside_inline_code_duplicates_the_code_delimiters() {
    let source = "before `code text` after";
    let current_byte = "before `code".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside inline code should keep two valid code spans");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "before `code` \n\n`text` after");
}

#[test]
fn paragraph_enter_in_the_middle_of_a_word_keeps_letters_together() {
    let source = "left right";
    let current_byte = "le".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("mid-word Enter should keep the split letters");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "le\n\nft right");
}

#[test]
fn table_enter_on_the_last_row_appends_a_new_row() {
    let source = "| a |\n|---|\n| b |";
    let current_byte = source.rfind('b').expect("fixture contains the body cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on the last table row should insert a new row");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a |\n|---|\n| b |\n|  |");
    assert_eq!(
        augmentation.cursor_byte_after,
        edited_source.rfind("|  |").expect("new row starts with an empty cell") + 2
    );
}

#[test]
fn table_enter_on_a_multi_column_last_row_copies_column_count() {
    let source = "| a | b |\n|---|---|\n| c | d |";
    let current_byte = source.rfind('c').expect("fixture contains the first body cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on the last table row should copy the column count");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a | b |\n|---|---|\n| c | d |\n|  |  |");
}

#[test]
fn table_enter_on_an_empty_last_body_row_exits_the_table() {
    let source = "| a |\n|---|\n|  |";
    let current_byte = source.rfind('|').expect("fixture contains the last pipe") - 1;

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on an empty last body row should leave the table");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a |\n|---|\n\n");
    assert_eq!(augmentation.cursor_byte_after, edited_source.len());
}

#[test]
fn table_enter_on_the_header_without_a_body_appends_a_body_row() {
    let source = "| a |\n|---|";
    let current_byte = source.find('a').expect("fixture contains the header cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a header with no body should create a body row");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a |\n|---|\n|  |");
}

#[test]
fn list_enter_at_indented_lazy_continuation_consumes_the_old_indent() {
    let source = "- item\n  continuation";
    let current_byte = "- item".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should turn the continuation into a sibling item");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- item\n- continuation");
}

#[test]
fn blockquote_enter_at_indented_lazy_continuation_consumes_the_old_indent() {
    let source = "> first\n  second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should make the lazy quote continuation explicit");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> second");
}

#[test]
fn heading_enter_inside_blockquote_preserves_the_quote_container() {
    let source = "> # left right";
    let current_byte = "> # left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("heading Enter should create a paragraph inside the quote");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> # left \n> right");
}

#[test]
fn heading_enter_inside_list_preserves_the_list_item_container() {
    let source = "- # left right";
    let current_byte = "- # left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("heading Enter should create a paragraph inside the item");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- # left \n  right");
}

#[test]
fn blockquote_inside_list_takes_precedence_over_the_outer_item() {
    let source = "- > left right";
    let current_byte = "- > left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("the inner quote should own Enter");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- > left \n  >\n  > right");
}

#[test]
fn table_row_insert_inside_blockquote_preserves_the_quote_prefix() {
    let source = "> | a |\n> |---|\n> | b |";
    let current_byte = source.rfind('b').expect("fixture contains body cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter should append a quoted table row");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> | a |\n> |---|\n> | b |\n> |  |");
}

#[test]
fn table_inside_list_takes_precedence_and_preserves_item_indent() {
    let source = "- table\n\n  | a |\n  |---|\n  | b |";
    let current_byte = source.rfind('b').expect("fixture contains body cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("the nested table should own Enter");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- table\n\n  | a |\n  |---|\n  | b |\n  |  |");
}

#[test]
fn line_break_preserves_the_word_separator_and_is_backspace_reversible() {
    let source = "left right";
    let split_byte = "left".len();

    let line_break = augment_edit(source, split_byte, AugmentKind::LineBreak)
        .expect("Shift+Enter should create a Markdown hard break");
    let source_with_break = apply_augmentation_at(source, split_byte, &line_break);
    assert_eq!(source_with_break, "left \\\nright");

    let backspace =
        augment_edit(&source_with_break, line_break.cursor_byte_after, AugmentKind::Backspace)
            .expect("Backspace at the hard-break continuation should restore the source");
    assert_eq!(
        apply_augmentation_at(&source_with_break, line_break.cursor_byte_after, &backspace),
        source
    );
}

#[test]
fn line_break_preserves_crlf_and_inline_element_boundaries() {
    let source = "**left right**\r\nnext";
    let current_byte = "**left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
        .expect("Shift+Enter should preserve styled fragments and CRLF");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "**left** \\\r\n**right**\r\nnext");
}

#[test]
fn inline_element_splits_are_backspace_reversible() {
    for (source, current_byte) in [
        ("**left right**", "**left".len()),
        ("**left *nested text* right**", "**left *nested".len()),
        ("[left right](https://example.com)", "[left".len()),
        ("before `code text` after", "before `code".len()),
    ] {
        let enter = augment_edit(source, current_byte, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter must split inline fixture {source:?}"));
        let split_source = apply_augmentation_at(source, current_byte, &enter);
        let backspace =
            augment_edit(&split_source, enter.cursor_byte_after, AugmentKind::Backspace)
                .unwrap_or_else(|| panic!("Backspace must reverse inline fixture {source:?}"));

        assert_eq!(
            apply_augmentation_at(&split_source, enter.cursor_byte_after, &backspace),
            source,
            "inline Enter/Backspace must roundtrip {source:?}"
        );
        assert_eq!(backspace.cursor_byte_after, reversible_split_point(source, current_byte));
    }
}

#[test]
fn styled_line_break_is_backspace_reversible() {
    let source = "**left right**";
    let current_byte = "**left".len();
    let line_break = augment_edit(source, current_byte, AugmentKind::LineBreak)
        .expect("Shift+Enter must split styled text");
    let split_source = apply_augmentation_at(source, current_byte, &line_break);
    let backspace =
        augment_edit(&split_source, line_break.cursor_byte_after, AugmentKind::Backspace)
            .expect("Backspace must reverse a styled hard break");

    assert_eq!(
        apply_augmentation_at(&split_source, line_break.cursor_byte_after, &backspace),
        source
    );
    assert_eq!(backspace.cursor_byte_after, reversible_split_point(source, current_byte));
}

#[cfg(feature = "rich-markdown")]
#[test]
fn inline_html_line_break_is_backspace_reversible() {
    let source = "# left right";
    let current_byte = "# left".len();
    let line_break = augment_edit(source, current_byte, AugmentKind::LineBreak)
        .expect("Shift+Enter must insert a heading line break");
    let split_source = apply_augmentation_at(source, current_byte, &line_break);
    let backspace =
        augment_edit(&split_source, line_break.cursor_byte_after, AugmentKind::Backspace)
            .expect("Backspace must remove the complete inline HTML break");

    assert_eq!(
        apply_augmentation_at(&split_source, line_break.cursor_byte_after, &backspace),
        source
    );
    assert_eq!(backspace.cursor_byte_after, current_byte);
}

#[test]
fn inline_element_splits_are_delete_forward_reversible() {
    for (source, current_byte) in [
        ("**left right**", "**left".len()),
        ("**left *nested text* right**", "**left *nested".len()),
        ("[left right](https://example.com)", "[left".len()),
        ("before `code text` after", "before `code".len()),
    ] {
        let enter = augment_edit(source, current_byte, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter must split inline fixture {source:?}"));
        let split_source = apply_augmentation_at(source, current_byte, &enter);
        let delete_byte =
            inline_safe_split_point(source, reversible_split_point(source, current_byte));
        let delete = augment_delete_forward(&split_source, delete_byte)
            .unwrap_or_else(|| panic!("DeleteForward must reverse inline fixture {source:?}"));

        assert_eq!(
            apply_augmentation_at(&split_source, delete_byte, &delete),
            source,
            "inline Enter/DeleteForward must roundtrip {source:?}"
        );
        assert_eq!(delete.cursor_byte_after, delete_byte);
    }
}

#[test]
fn inline_html_line_break_is_delete_forward_reversible() {
    let source = "# left right";
    let current_byte = "# left".len();
    let line_break = augment_edit(source, current_byte, AugmentKind::LineBreak)
        .expect("Shift+Enter must insert a heading line break");
    let split_source = apply_augmentation_at(source, current_byte, &line_break);
    let delete = augment_delete_forward(&split_source, current_byte)
        .expect("DeleteForward must remove the complete inline HTML break");

    assert_eq!(apply_augmentation_at(&split_source, current_byte, &delete), source);
    assert_eq!(delete.cursor_byte_after, current_byte);
}

#[test]
fn line_break_preserves_list_and_quote_container_paths() {
    let list_source = "- left right";
    let list_cursor = "- left".len();
    let list_augmentation = augment_edit(list_source, list_cursor, AugmentKind::LineBreak)
        .expect("Shift+Enter should stay inside the list item");
    assert_eq!(
        apply_augmentation_at(list_source, list_cursor, &list_augmentation),
        "- left \\\n  right"
    );

    let quote_source = "- > left right";
    let quote_cursor = "- > left".len();
    let quote_augmentation = augment_edit(quote_source, quote_cursor, AugmentKind::LineBreak)
        .expect("Shift+Enter should stay inside the nested quote");
    assert_eq!(
        apply_augmentation_at(quote_source, quote_cursor, &quote_augmentation),
        "- > left \\\n  > right"
    );
}

#[test]
fn list_line_break_uses_markdown_content_columns() {
    for (source, expected_source) in [
        ("-\tleft right", "-\tleft \\\n    right"),
        ("-   left right", "-   left \\\n    right"),
        ("10.\tleft right", "10.\tleft \\\n    right"),
        ("- parent\n  - left right", "- parent\n  - left \\\n    right"),
    ] {
        let current_byte = source.find("left").expect("fixture contains left") + "left".len();
        let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("list line break must be augmented");

        assert_eq!(
            apply_augmentation_at(source, current_byte, &augmentation),
            expected_source,
            "wrong Markdown content column for {source:?}"
        );
    }
}

#[test]
fn list_enter_preserves_multi_space_marker_separator() {
    let source = "-   left right";
    let current_byte = source.find("left").expect("fixture contains left") + "left".len();
    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter must continue the list marker");

    assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), "-   left \n-   right");
}

#[test]
fn paragraph_backspace_into_tabbed_list_aligns_to_content_column() {
    let source = "-\titem\n\nparagraph";
    let current_byte = source.find("paragraph").expect("fixture contains paragraph");
    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace must merge the paragraph into the list");

    assert_eq!(
        apply_augmentation_at(source, current_byte, &augmentation),
        "-\titem\n    paragraph"
    );
}

#[cfg(feature = "rich-markdown")]
#[test]
fn line_break_uses_inline_html_where_a_physical_line_would_end_the_leaf() {
    for (source, current_byte) in [
        ("# left right", "# left".len()),
        ("| a |\n|---|\n| left right |", "| a |\n|---|\n| left".len()),
    ] {
        let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("Shift+Enter should preserve the heading or table-cell leaf");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);
        assert_eq!(&edited_source[current_byte..current_byte + 4], "<br>");
    }
}

#[test]
fn metadata_enter_and_line_break_insert_indented_literal_newlines() {
    let source = "---\nsection:\n  title: hello\n---";
    let current_byte = source.find("hello").expect("fixture contains metadata value") + 2;

    for kind in [AugmentKind::Enter, AugmentKind::LineBreak] {
        let augmentation = augment_edit(source, current_byte, kind.clone())
            .expect("metadata editing must emit a literal source newline");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "---\nsection:\n  title: he\n  llo\n---");
        assert!(!edited_source.contains("\\\n"));
    }
}

#[test]
fn html_block_enter_and_line_break_insert_indented_literal_newlines() {
    let source = "<div>\n  hello world\n</div>";
    let current_byte = source.find("hello").expect("fixture contains HTML text") + 5;

    for kind in [AugmentKind::Enter, AugmentKind::LineBreak] {
        let augmentation = augment_edit(source, current_byte, kind.clone())
            .expect("HTML block editing must emit a literal source newline");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "<div>\n  hello\n   world\n</div>");
        assert!(!edited_source.contains("\\\n"));
    }
}

#[test]
fn line_break_on_empty_or_unknown_structural_line_never_inserts_hard_break_marker() {
    for (source, current_byte) in [("left\n\nright", "left\n".len()), ("---", 0)] {
        let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("literal structural line break must produce an augmentation");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert!(!edited_source.contains("\\\n"), "unexpected hard break in {edited_source:?}");
    }
}

#[test]
fn enter_on_an_empty_nested_container_exits_exactly_one_level() {
    for (source, expected_source) in [("- > ", "- "), ("> - ", "> "), ("> > ", "> ")] {
        let augmentation = augment_edit(source, source.len(), AugmentKind::Enter)
            .expect("Enter should exit the innermost empty container");
        let edited_source = apply_augmentation_at(source, source.len(), &augmentation);

        assert_eq!(edited_source, expected_source, "wrong exit level for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, expected_source.len());
    }
}

#[test]
fn setext_heading_enter_preserves_outer_container_path() {
    for (source, expected_source) in
        [("> Title\n> =====", "> Title\n> =====\n> "), ("- Title\n  =====", "- Title\n  =====\n  ")]
    {
        let current_byte = source.find("Title").expect("fixture contains title") + "Title".len();
        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should append a paragraph inside the same container");

        assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), expected_source);
    }
}

#[test]
fn table_empty_row_exit_preserves_outer_container_path() {
    for (source, expected_source) in [
        ("> | a |\n> |---|\n> |  |", "> | a |\n> |---|\n> "),
        ("- table\n\n  | a |\n  |---|\n  |  |", "- table\n\n  | a |\n  |---|\n  "),
    ] {
        let current_byte = source.rfind('|').expect("fixture contains empty row") - 1;
        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should leave the table but keep its parent container");

        assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), expected_source);
    }
}

#[test]
fn delete_forward_before_a_sibling_block_line_is_consumed() {
    // 段末 Delete 若会把下一行并上来自成 setext 标题/破坏 ATX/围栏/列表/引用,
    // 必须拦截为消费型空操作(不删任何字节)。
    for (source, cursor) in [
        ("文字\n\n---", "文字".len()),
        ("文字\r\n\r\n---", "文字".len()),
        ("文字\n\n===", "文字".len()),
        ("文字\n\n# 标题", "文字".len()),
        ("文字\n\n```", "文字".len()),
        ("文字\n\n- item", "文字".len()),
        ("文字\n\n> 引用", "文字".len()),
        ("- a\n\n- b", "- a".len()),
        ("| a |\n|---|---|\n| b |", "| a |".len()),
    ] {
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward at {source:?} must be guarded"));
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);

        assert_eq!(edited_source, source, "DeleteForward must not alter {source:?}");
        assert_eq!(augmentation.cursor_byte_after, cursor);
    }
}

#[test]
fn line_column_position_preserves_only_container_tab_overshoot() {
    for (required_columns, expected_residual) in [(4, 0), (3, 1), (2, 2), (1, 3)] {
        let source = "\t=";
        let path = ContainerPath(vec![ContainerSegment::IndentColumns(required_columns)]);
        let container_position = matching_container_position(source, 0, source.len(), &path)
            .expect("tab must reach the container target");
        assert_eq!(container_position.leaf_indent_columns(), Some(expected_residual));
        assert!(
            marker_position_after_optional_indent(source, source.len(), container_position)
                .is_some(),
            "container overshoot {expected_residual} must be a valid leaf indent"
        );
    }

    let source = "\t=";
    let root_position = LineColumnPosition::at_line_start(0);
    assert!(
        marker_position_after_optional_indent(source, source.len(), root_position).is_none(),
        "a root-level tab is four leaf-indent columns"
    );

    let source = "  \t=";
    let path = ContainerPath(vec![ContainerSegment::IndentColumns(2)]);
    let exact_container_position = matching_container_position(source, 0, source.len(), &path)
        .expect("spaces exactly reach the container target");
    assert!(
        marker_position_after_optional_indent(source, source.len(), exact_container_position)
            .is_none(),
        "a tab beginning after the container target is four leaf-indent columns"
    );
}

#[test]
fn block_quote_segment_consumes_container_tab_residual_as_optional_indent() {
    for (required_columns, explicit_spaces) in [(3, 2), (2, 1), (1, 0)] {
        let source = format!("\t{}> marker", " ".repeat(explicit_spaces));
        let path = ContainerPath(vec![
            ContainerSegment::IndentColumns(required_columns),
            ContainerSegment::BlockQuote,
        ]);
        let position = matching_container_position(&source, 0, source.len(), &path)
            .unwrap_or_else(|| panic!("container path must match {source:?}"));

        assert_eq!(source.as_bytes().get(position.byte_offset), Some(&b'm'));
        assert_eq!(position.absolute_column, position.container_target_column);
    }

    let source = "\t  > marker";
    let path =
        ContainerPath(vec![ContainerSegment::IndentColumns(2), ContainerSegment::BlockQuote]);
    assert!(
        matching_container_position(source, 0, source.len(), &path).is_none(),
        "two residual columns plus two physical spaces exceed quote optional indent"
    );
}

#[test]
fn delete_forward_at_paragraph_end_merges_the_following_paragraph() {
    for (source, cursor, expected_source) in [
        ("a\n\nb", 1, "ab"),
        ("a\r\n\r\nb", 1, "ab"),
        ("first\nsecond", "first".len(), "firstsecond"),
        ("# 标题\n\n正文", "# 标题".len(), "# 标题正文"),
    ] {
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward should merge paragraphs in {source:?}"));
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);

        assert_eq!(edited_source, expected_source, "wrong merge result for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, cursor);
    }
}

#[test]
fn delete_forward_inside_code_block_body_falls_back_to_default() {
    // 代码体内的下一行只是代码文本(即使以 `#` 开头),不做块边界拦截。
    let source = "```\nlet a\n# b\n```";
    let cursor = "```\nlet a".len();

    assert!(augment_delete_forward(source, cursor).is_none());
}

#[test]
fn delete_forward_before_code_block_closing_fence_is_consumed() {
    let source = "```\ncode\n```\n\npara";
    let cursor = "```\ncode".len();

    let augmentation = augment_delete_forward(source, cursor)
        .expect("DeleteForward must not merge the closing fence into code");
    let edited_source = apply_augmentation_at(source, cursor, &augmentation);

    assert_eq!(edited_source, source);
    assert_eq!(augmentation.cursor_byte_after, cursor);
}

#[test]
fn delete_forward_protects_short_setext_underlines() {
    for source in ["title\n=", "title\n==", "title\n--"] {
        let cursor = "title".len();
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None, "must not merge {source:?}");
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_allows_marker_like_paragraphs() {
    for (source, expected_source) in [
        ("first\n---not-a-rule", "first---not-a-rule"),
        ("first\n===not-setext", "first===not-setext"),
        ("first\n***plain", "first***plain"),
    ] {
        let cursor = "first".len();
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must merge {source:?}"));

        assert_eq!(augmentation.replace_range, Some(cursor..cursor + 1));
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), expected_source);
    }
}

#[test]
fn delete_forward_inside_fenced_code_only_protects_matching_closer() {
    for source in ["````\ncode\n```\n````", "~~~~\ncode\n```\n~~~~", "```\ncode\n```not-close\n```"]
    {
        let cursor = source.find("\ncode").expect("fixture contains code") + "\ncode".len();

        assert!(
            augment_delete_forward(source, cursor).is_none(),
            "non-closing fence line must stay default for {source:?}"
        );
    }

    let source = "```\ncode\n``` \t\npara";
    let cursor = "```\ncode".len();
    let augmentation = augment_delete_forward(source, cursor)
        .expect("matching closing fence with trailing whitespace must be protected");

    assert_eq!(augmentation.replace_range, None);
    assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
}

#[test]
fn delete_forward_inside_container_fenced_code_matches_the_real_closer() {
    for source in [
        "> ```\n> code\n>````\n> paragraph",
        "> ```\n> code\n> ````\n> paragraph",
        " > ```\n> code\n  >````\n> paragraph",
        "- ```\n   code\n   ````\n\nparagraph",
        "- ```\n  code\n     ````\n\nparagraph",
        "- ```\n  code\n  ````\n\nparagraph",
        "-\t```\n\tcode\n\t````\n\nparagraph",
        "- item\n\n  ```\n  code\n     ````\n\nparagraph",
        "- item\n\n  ```\n  code\n\t```\n\nparagraph",
        "> 12. ```\n>     code\n> \t\t``` \n\nparagraph",
    ] {
        let cursor = source.find("code").expect("fixture contains code") + "code".len();
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_matches_list_quote_fences_across_tab_residual() {
    for source in
        ["- > ```\n  > code\n\t> ```\n", "- item\n\n\t> ```\n\t> code\n  > ```\n\nparagraph"]
    {
        let cursor = source.find("code").expect("fixture contains code") + "code".len();
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_container_code_distinguishes_literal_pseudo_fences_from_neighboring_blocks() {
    for (source, expected) in [
        ("> ```\n> code\n> ```not-close\n> ```", "> ```\n> code```not-close\n> ```"),
        ("- ```\n  code\n  ```not-close\n  ```", "- ```\n  code```not-close\n  ```"),
    ] {
        let cursor = source.find("code").expect("fixture contains code") + "code".len();
        let join = augment_delete_forward(source, cursor)
            .expect("literal rows join without structural prefixes");
        assert_eq!(apply_augmentation_at(source, cursor, &join), expected);
    }
    for source in ["> ```\n> code\n```\nparagraph", "- ```\n  code\n```\nparagraph"] {
        let cursor = source.find("code").expect("fixture contains code") + "code".len();
        let protected = augment_delete_forward(source, cursor)
            .expect("neighboring outer code block stays separate");
        assert_eq!(apply_augmentation_at(source, cursor, &protected), source);
    }
}

#[test]
fn delete_forward_inside_fenced_code_joins_over_indented_pseudo_fences_as_literal_text() {
    for (source, expected) in [
        ("```\ncode\n    ```\n```", "```\ncode    ```\n```"),
        ("```\ncode\n\t```\n```", "```\ncode\t```\n```"),
        ("> ```\n> code\n>     ```\n> ```", "> ```\n> code    ```\n> ```"),
        ("> ```\n> code\n> \t```\n> ```", "> ```\n> code\t```\n> ```"),
        ("- ```\n  code\n      ```\n  ```", "- ```\n  code    ```\n  ```"),
        ("- ```\n  code\n  \t```\n  ```", "- ```\n  code\t```\n  ```"),
        ("- item\n\n  ```\n  code\n      ```\n  ```", "- item\n\n  ```\n  code    ```\n  ```"),
    ] {
        let cursor = source.find("code").expect("fixture contains code") + "code".len();
        let edited = augment_delete_forward(source, cursor).map_or_else(
            || {
                let mut edited = source.to_owned();
                let width = newline_sequence_width_at(source, cursor)
                    .expect("fixture joins a source newline");
                edited.replace_range(cursor..cursor + width, "");
                edited
            },
            |augmentation| apply_augmentation_at(source, cursor, &augmentation),
        );
        assert_eq!(edited, expected, "pseudo fence is literal text in {source:?}");
    }
}

#[test]
fn delete_forward_inside_code_before_blank_line_keeps_default() {
    let source = "```\ncode\n\n```";
    let cursor = "```\ncode".len();

    assert!(
        augment_delete_forward(source, cursor).is_none(),
        "DeleteForward must remove only the first newline before a blank code line"
    );
}

#[test]
fn fenced_code_start_range_points_to_the_opening_marker() {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};

    for source in ["```\ncode\n```", "> ```\n> code\n> ```", "- ```\n  code\n  ```"] {
        let marker_start = Parser::new_ext(source, crate::parser::markdown_options())
            .into_offset_iter()
            .find_map(|(event, range)| {
                matches!(event, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))))
                    .then_some(range.start)
            })
            .expect("fixture contains fenced code");

        assert_eq!(marker_start, source.find("```").expect("fixture contains opener"));
    }
}

#[test]
fn delete_forward_rejects_indented_code_as_thematic_break() {
    for source in ["first\n\t***", "first\n    ***"] {
        let cursor = "first".len();
        let stars = source.find("***").expect("fixture contains stars");
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must merge {source:?}"));

        assert_eq!(augmentation.replace_range, Some(cursor..stars));
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), "first***");
    }
}

#[test]
fn delete_forward_accepts_valid_thematic_break_indentation() {
    for source in ["first\n***", "first\n *\t*\t*", "first\n  ***", "first\n   ***"] {
        let cursor = "first".len();
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_marker_boundaries_support_crlf() {
    for source in ["title\r\n=", "title\r\n  ==\t", "first\r\n *\t*\t*"] {
        let cursor = source.find("\r\n").expect("fixture contains CRLF");
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
    }

    let source = "first\r\n===not-setext";
    let cursor = "first".len();
    let augmentation =
        augment_delete_forward(source, cursor).expect("marker-like CRLF paragraph must merge");
    assert_eq!(augmentation.replace_range, Some(cursor..cursor + 2));
    assert_eq!(apply_augmentation_at(source, cursor, &augmentation), "first===not-setext");

    let source = "````\r\ncode\r\n````` \t\r\nparagraph";
    let cursor = source.find("code").expect("fixture contains code") + "code".len();
    let augmentation = augment_delete_forward(source, cursor)
        .expect("longer CRLF closing fence must be protected");
    assert_eq!(augmentation.replace_range, None);
}

#[test]
fn delete_forward_protects_markers_after_wide_list_continuation() {
    for source in ["123. title\n     =", "123. para\n     ***"] {
        let cursor = source.find('\n').expect("fixture contains newline");
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_protects_markers_after_nested_container_continuation() {
    for source in [
        "123. > title\n     > =",
        "123. > para\n     > ***",
        "> 123. title\n>      =",
        "> 123. para\n>      ***",
        "- title\n\t=",
        "- para\n\t***",
        "> - title\n>\t=",
    ] {
        let cursor = source.find('\n').expect("fixture contains newline");
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_protects_list_quote_markers_across_tab_residual() {
    for source in ["- > title\n\t> =", "- > para\n\t> ***"] {
        let cursor = source.find('\n').expect("fixture contains newline");
        let augmentation = augment_delete_forward(source, cursor)
            .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }
}

#[test]
fn delete_forward_rejects_over_indented_or_marker_like_container_lines() {
    for source in [
        "123. para\n         ***",
        "123. para\n     \t***",
        "123. para\n     ***plain",
        "123. title\n     ===not-setext",
        "123. > para\n     >     ***",
        "123. > para\n     > \t***",
    ] {
        let cursor = source.find('\n').expect("fixture contains newline");
        let augmentation = augment_delete_forward(source, cursor);

        assert!(
            augmentation.as_ref().is_none_or(|value| value.replace_range.is_some()),
            "DeleteForward must not guard {source:?}"
        );
    }
}

#[test]
fn delete_forward_after_hard_break_marker_keeps_default_behaviour() {
    // 行尾硬换行标记属于段内结构,Delete 仍逐字符削弱标记。
    let source = "first  \n\nsecond";
    let cursor = "first".len();

    assert!(augment_delete_forward(source, cursor).is_none());
}

#[test]
fn enter_on_opening_fence_line_moves_cursor_into_code_body() {
    // 开头围栏行(含 info string)任意位置回车:在该行行尾插入单个换行,
    // 光标进入代码体第一行,围栏本身保持完整。
    for (source, cursor) in [
        ("```rust\ncode\n```", 0),
        ("```rust\ncode\n```", 5),
        ("```rust\ncode\n```", "```rust".len()),
        ("~~~\ncode\n~~~", "~~~".len()),
        // 已有代码内容的未闭合围栏不应在开头插入闭合围栏。
        ("```rust\ncode", 5),
    ] {
        let fence_line_end = source.find('\n').unwrap_or(source.len());
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter on opening fence line of {source:?}"));
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);
        let mut expected_source = source.to_owned();
        expected_source.insert(fence_line_end, '\n');

        assert_eq!(edited_source, expected_source, "wrong opening fence enter for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, fence_line_end + 1);
    }
}

#[test]
fn enter_on_new_unclosed_fence_inserts_matching_closer() {
    for (source, expected) in [
        ("```", "```\n\n```"),
        ("```rust", "```rust\n\n```"),
        ("````", "````\n\n````"),
        ("~~~~", "~~~~\n\n~~~~"),
        ("  ```", "  ```\n  \n  ```"),
        ("> ```", "> ```\n> \n> ```"),
        ("- ```", "- ```\n  \n  ```"),
        ("```\n", "```\n\n```"),
        ("```rust\r\n", "```rust\r\n\r\n```"),
    ] {
        let cursor = source.find(['\r', '\n']).unwrap_or(source.len());
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("new fence should close: {source:?}"));
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), expected);
        let body_line_start = expected.find(['\r', '\n']).expect("fixture has body")
            + if expected.contains("\r\n") { CRLF_SEQUENCE.len() } else { LF_SEQUENCE.len() };
        let body_prefix_length =
            expected[body_line_start..].find(['\r', '\n']).expect("fixture has closing fence line");
        assert_eq!(augmentation.cursor_byte_after, body_line_start + body_prefix_length);
    }
}

#[test]
fn enter_on_closing_fence_line_exits_code_block() {
    // 闭合围栏行任意位置回车:在闭合围栏行尾建立块边界,光标落到新空段。
    for (source, cursor, expected_source) in [
        ("```\ncode\n```", 9, "```\ncode\n```\n\n"),
        ("```\ncode\n```", "```\ncode\n```".len(), "```\ncode\n```\n\n"),
        ("~~~\ncode\n~~~", 11, "~~~\ncode\n~~~\n\n"),
        ("```\ncode\n```\npara", 9, "```\ncode\n```\n\n\npara"),
    ] {
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter on closing fence line of {source:?}"));
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);

        assert_eq!(edited_source, expected_source, "wrong closing fence enter for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, "```\ncode\n```\n\n".len());
    }
}

#[test]
fn enter_inside_code_block_body_keeps_default_newline() {
    // 代码体内部(含形似短围栏的代码行)回车仍回落默认裸换行。
    for (source, cursor) in [
        ("```\ncode\n```", 6),
        ("````\n```\n````", 7),
        ("````\ncode\n```", "````\ncode\n```".len()),
    ] {
        assert!(
            augment_edit(source, cursor, AugmentKind::Enter).is_none(),
            "code body Enter must stay default for {source:?}"
        );
    }
}

#[test]
fn delete_forward_after_closing_fence_preserves_hidden_separators_and_deletes_visible_slots() {
    let cursor = "```\ncode\n```".len();
    for source in ["```\ncode\n```\npara", "```\ncode\n```\n\npara"] {
        let protected =
            augment_delete_forward(source, cursor).expect("structural separator is protected");
        assert_eq!(apply_augmentation_at(source, cursor, &protected), source);
        assert_eq!(protected.cursor_byte_after, cursor);
    }
    let editable_source = "```\ncode\n```\n\n\npara";
    let deletion = augment_delete_forward(editable_source, cursor)
        .expect("visible empty paragraph can be removed");
    assert_eq!(apply_augmentation_at(editable_source, cursor, &deletion), "```\ncode\n```\n\npara");
}

#[test]
fn list_item_hosting_only_a_nested_list_is_not_empty() {
    // `- ` 下只挂子列表时父 item 不算空:父行回车不得删掉 `- ` 把子列表提升为顶层。
    let source = "- \n  - x";
    let cursor = "- ".len();

    let context = classify_enter_context(source, cursor);
    let EnterContext::ListItem { empty, .. } = context else {
        panic!("expected ListItem context, got {context:?}");
    };
    assert!(!empty, "parent item hosting a nested list must not be empty");

    let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
        .expect("Enter on the parent line should produce an augmentation");
    let edited_source = apply_augmentation_at(source, cursor, &augmentation);
    assert!(
        edited_source.starts_with("- \n"),
        "parent marker must survive Enter, got {edited_source:?}"
    );
}

#[test]
fn enter_on_empty_nested_list_item_exits_one_level_at_a_time() {
    // 嵌套空 item 回车退一层(行替换为父级前缀);残留行再回车继续退出,
    // 最终落在干净的空行,而不是意外创建同级 item。
    // 注意:空嵌套 item 必须跟在非空嵌套 item 之后,否则 `- ` 会被
    // pulldown-cmark 解析成上一行文本的 setext 下划线。
    let source = "- a\n  - b\n    - c\n    - ";
    let mut current_source = source.to_owned();
    let mut cursor = current_source.len();
    for expected_source in
        ["- a\n  - b\n    - c\n    ", "- a\n  - b\n    - c\n  ", "- a\n  - b\n    - c\n"]
    {
        let augmentation = augment_edit(&current_source, cursor, AugmentKind::Enter)
            .expect("Enter should exit one list level");
        current_source = apply_augmentation_at(&current_source, cursor, &augmentation);
        cursor = augmentation.cursor_byte_after;

        assert_eq!(current_source, expected_source);
        assert_eq!(cursor, expected_source.len());
    }
}

#[test]
fn enter_inside_list_marker_behaves_like_enter_at_content_start() {
    // 光标在 marker 内部(`-| item`)回车 = 在内容起点回车,
    // 不得产出 `-\n item` 这样的懒延续残留。
    for (source, cursor, expected_source, expected_cursor) in [
        ("- item", "-".len(), "- \n- item", "- \n- ".len()),
        ("1. item", "1.".len(), "1. \n2. item", "1. \n2. ".len()),
    ] {
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter).unwrap_or_else(|| {
            panic!("Enter inside the marker of {source:?} should continue the list")
        });
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);

        assert_eq!(edited_source, expected_source, "wrong marker-interior enter for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, expected_cursor);
    }
}

#[test]
fn heading_content_start_scans_actual_whitespace_after_hashes() {
    // `#` 后的多余空白不属于标题内容:光标落在空白内时 Enter 回落默认。
    for (source, cursor) in [("#  Title", 2), ("#\t\tTitle", 2), ("##   Title", 3)] {
        assert!(
            augment_edit(source, cursor, AugmentKind::Enter).is_none(),
            "Enter inside heading marker whitespace must stay default for {source:?}"
        );
    }

    // 内容起点回车前插普通空段，原标题保持完整。
    let source = "#  Title";
    let cursor = "#  ".len();
    let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
        .expect("Enter at heading content start should insert a preceding paragraph");
    assert_eq!(apply_augmentation_at(source, cursor, &augmentation), "\n#  Title");
    assert_eq!(augmentation.cursor_byte_after, cursor + LF_SEQUENCE.len());

    // 空标题(`#` 后无空白)行尾回车 = 退出到新段落。
    let empty_heading = "#";
    let augmentation = augment_edit(empty_heading, 1, AugmentKind::Enter)
        .expect("Enter on an empty heading should create a paragraph break");
    assert_eq!(apply_augmentation_at(empty_heading, 1, &augmentation), "#\n\n");
    assert_eq!(augmentation.cursor_byte_after, 3);
}

#[test]
fn blockquote_mid_line_enter_splits_into_two_paragraphs() {
    let source = "> hello world";
    let current_byte = "> hel".len();
    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("mid-line Enter in blockquote should split paragraph");
    assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), "> hel\n>\n> lo world");
    assert_eq!(augmentation.cursor_byte_after, "> hel\n>\n> ".len());

    let crlf_source = "> hello world\r\n> next";
    let crlf_byte = "> hel".len();
    let crlf_aug = augment_edit(crlf_source, crlf_byte, AugmentKind::Enter)
        .expect("CRLF blockquote mid-line Enter should preserve CRLF");
    assert_eq!(
        apply_augmentation_at(crlf_source, crlf_byte, &crlf_aug),
        "> hel\r\n>\r\n> lo world\r\n> next"
    );
    assert_eq!(crlf_aug.cursor_byte_after, "> hel\r\n>\r\n> ".len());

    let bold_source = "> **left right**";
    let bold_byte = "> **left".len();
    let bold_aug = augment_edit(bold_source, bold_byte, AugmentKind::Enter)
        .expect("mid-line Enter inside bold span in blockquote should split cleanly");
    assert_eq!(
        apply_augmentation_at(bold_source, bold_byte, &bold_aug),
        "> **left** \n>\n> **right**"
    );
}

#[test]
fn blockquote_lazy_continuation_mid_line_enter_splits_inside_quote() {
    let source = "> hello\nworld";
    let current_byte = "> hello\nwor".len();
    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("lazy continuation line inside blockquote should split inside quote");
    assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), "> hello\nwor\n>\n> ld");
    assert_eq!(augmentation.cursor_byte_after, "> hello\nwor\n>\n> ".len());
}

#[test]
fn blockquote_nested_list_continuation_preserves_list_indent() {
    let source = "> - para 1\n>\n>   para 2";
    let current_byte = source.len();
    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("list continuation inside blockquote should resolve to list item");
    assert_eq!(
        apply_augmentation_at(source, current_byte, &augmentation),
        "> - para 1\n>\n>   para 2\n> - "
    );
    assert_eq!(augmentation.cursor_byte_after, "> - para 1\n>\n>   para 2\n> - ".len());
}

#[test]
fn backspace_at_soft_line_start_strips_folded_leading_whitespace() {
    for (source, needle, expected) in [
        ("left\n   right", "right", "left|right"),
        ("left\r\n  \tright", "right", "left|right"),
        ("left \n   right", "right", "left |right"),
        ("left\\\n   right", "right", "left|right"),
        ("left  \r\n  right", "right", "left|right"),
    ] {
        let current_byte = source.find(needle).expect("fixture must contain needle");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .unwrap_or_else(|| panic!("expected backspace augmentation for {source:?}"));
        let edited = apply_augmentation_at(source, current_byte, &augmentation);
        let expected_cursor = expected.find('|').expect("expected must contain caret marker");
        let expected_source = expected.replace('|', "");
        assert_eq!(edited, expected_source, "wrong result for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, expected_cursor, "wrong caret for {source:?}");
    }
}

#[test]
fn delete_forward_before_soft_break_strips_folded_leading_whitespace() {
    for (source, expected) in [
        ("left\n   right", "leftright"),
        ("left\r\n  \tright", "leftright"),
        ("left\n\n   right", "leftright"),
    ] {
        let current_byte = "left".len();
        let augmentation = augment_delete_forward(source, current_byte)
            .unwrap_or_else(|| panic!("expected delete-forward augmentation for {source:?}"));
        let edited = apply_augmentation_at(source, current_byte, &augmentation);
        assert_eq!(edited, expected, "wrong forward-delete result for {source:?}");
        assert_eq!(augmentation.cursor_byte_after, current_byte, "wrong caret for {source:?}");
    }
}

#[test]
fn empty_fenced_code_insert_at_closing_fence_writes_inside_code_body() {
    for (source, cursor, insertion, expected, expected_cursor) in [
        ("```\n```", "```\n".len(), "x", "```\nx\n```", "```\nx".len()),
        ("```\r\n```", "```\r\n".len(), "x", "```\r\nx\r\n```", "```\r\nx".len()),
        (
            "```rust\n```",
            "```rust\n".len(),
            "let a = 1;",
            "```rust\nlet a = 1;\n```",
            "```rust\nlet a = 1;".len(),
        ),
        ("> ```\n> ```", "> ```\n".len(), "x", "> ```\n> x\n> ```", "> ```\n> x".len()),
        ("> ```\n> ```", "> ```\n> ".len(), "x", "> ```\n> x\n> ```", "> ```\n> x".len()),
        ("- ```\n  ```", "- ```\n".len(), "x", "- ```\n  x\n  ```", "- ```\n  x".len()),
    ] {
        let augmentation =
            augment_edit(source, cursor, AugmentKind::InsertText(insertion.to_owned()))
                .unwrap_or_else(|| {
                    panic!("empty fenced code insert should augment for {source:?} at {cursor}")
                });
        let edited = apply_augmentation_at(source, cursor, &augmentation);
        assert_eq!(edited, expected, "wrong edited source for {source:?} at {cursor}");
        assert_eq!(
            augmentation.cursor_byte_after, expected_cursor,
            "wrong cursor for {source:?} at {cursor}"
        );
    }

    assert!(
        augment_edit("```\na\n```", "```\na\n".len(), AugmentKind::InsertText(String::from("x")))
            .is_none(),
        "non-empty fenced code closing fence must not synthesize extra empty body line"
    );
}

#[test]
fn opening_fence_allows_editing_info_string_and_protects_fence_markers() {
    assert_eq!(
        apply_backspace_with_default("```rust\ncode\n```", "```rust".len()),
        (String::from("```rus\ncode\n```"), "```rus".len())
    );
    assert_eq!(
        apply_backspace_with_default("```rust\ncode\n```", "```r".len()),
        (String::from("```ust\ncode\n```"), "```".len())
    );
    assert_eq!(
        apply_delete_forward_with_default("```rust\ncode\n```", "```".len()),
        (String::from("```ust\ncode\n```"), "```".len())
    );
    assert_eq!(
        apply_backspace_with_default("> ```rust\n> code\n> ```", "> ```rust".len()),
        (String::from("> ```rus\n> code\n> ```"), "> ```rus".len())
    );
    assert_eq!(
        apply_delete_forward_with_default("~~~~rust\ncode\n~~~~", "~~~~".len()),
        (String::from("~~~~ust\ncode\n~~~~"), "~~~~".len())
    );

    let fenced = "```rust\ncode\n```";
    for cursor in [0usize, 1, 2, 3] {
        assert_eq!(
            apply_backspace_with_default(fenced, cursor),
            (String::from(fenced), cursor),
            "Backspace at opening fence byte {cursor} must preserve fence"
        );
    }
    for cursor in [0usize, 1, 2] {
        assert_eq!(
            apply_delete_forward_with_default(fenced, cursor),
            (String::from(fenced), cursor),
            "DeleteForward at opening fence byte {cursor} must preserve fence"
        );
    }
    assert_eq!(
        apply_backspace_with_default("```\ncode\n```", "```".len()),
        (String::from("```\ncode\n```"), "```".len())
    );
    assert_eq!(
        apply_backspace_with_default("~~~rust\ncode\n~~~", "~~~".len()),
        (String::from("~~~rust\ncode\n~~~"), "~~~".len())
    );

    let quoted = "> ```rust\n> code\n> ```";
    for cursor in ["> ".len(), "> ```".len()] {
        assert_eq!(
            apply_backspace_with_default(quoted, cursor),
            (String::from(quoted), cursor),
            "quoted opening fence Backspace at {cursor} must preserve prefix and marker"
        );
    }
    assert_eq!(
        apply_delete_forward_with_default(quoted, "> ".len()),
        (String::from(quoted), "> ".len())
    );

    let closing_source = "```\ncode\n```";
    let closing_start = "```\ncode\n".len();
    for offset in 0..=3 {
        let cursor = closing_start + offset;
        assert_eq!(
            apply_backspace_with_default(closing_source, cursor),
            (String::from(closing_source), cursor),
            "Backspace on closing fence byte {cursor} must preserve closing fence"
        );
    }
    for offset in 0..3 {
        let cursor = closing_start + offset;
        assert_eq!(
            apply_delete_forward_with_default(closing_source, cursor),
            (String::from(closing_source), cursor),
            "DeleteForward on closing fence byte {cursor} must preserve closing fence"
        );
    }

    let quoted_closing_start = "> ```rust\n> code\n".len();
    for offset in 0..="> ```".len() {
        let cursor = quoted_closing_start + offset;
        assert_eq!(
            apply_backspace_with_default(quoted, cursor),
            (String::from(quoted), cursor),
            "quoted closing fence Backspace at {cursor} must preserve closing fence"
        );
    }
}

fn list_item_nesting_depths(source: &str) -> Vec<(usize, String)> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut depth = 0usize;
    let mut item_texts: Vec<(usize, String)> = Vec::new();
    let mut active_items: Vec<Option<usize>> = Vec::new();

    for event in Parser::new_ext(source, crate::parser::markdown_options()) {
        match event {
            Event::Start(Tag::List(_)) => {
                if let Some(slot) = active_items.last_mut() {
                    *slot = None;
                }
                depth += 1;
            }
            Event::End(TagEnd::List(_)) => depth = depth.saturating_sub(1),
            Event::Start(Tag::Item) => {
                let index = item_texts.len();
                item_texts.push((depth, String::new()));
                active_items.push(Some(index));
            }
            Event::Text(text) => {
                if let Some(Some(index)) = active_items.last().copied() {
                    item_texts[index].1.push_str(&text);
                }
            }
            Event::SoftBreak => {
                if let Some(Some(index)) = active_items.last().copied()
                    && !item_texts[index].1.is_empty()
                {
                    item_texts[index].1.push(' ');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(slot) = active_items.last_mut()
                    && slot.is_some_and(|index| !item_texts[index].1.is_empty())
                {
                    *slot = None;
                }
            }
            Event::End(TagEnd::Item) => {
                active_items.pop();
            }
            _ => {}
        }
    }
    item_texts
}

#[test]
fn list_item_tab_and_shift_tab_move_complete_subtree_and_preserve_ast_hierarchy() {
    // 1. 首项无前驱兄弟按 Tab 消费（不变为缩进代码块），顶层 0 缩进 Shift+Tab 消费，带前导空格 Shift+Tab 去缩进。
    let first_only = "- first\n- second";
    let first_tab = augment_edit(first_only, "- fir".len(), AugmentKind::Tab)
        .expect("first item Tab must consume instead of falling back");
    assert_eq!(apply_augmentation_at(first_only, "- fir".len(), &first_tab), first_only);
    assert_eq!(first_tab.cursor_byte_after, "- fir".len());

    let first_shift = augment_edit(first_only, "- fir".len(), AugmentKind::ShiftTab)
        .expect("top-level zero-indent item ShiftTab must consume");
    assert_eq!(apply_augmentation_at(first_only, "- fir".len(), &first_shift), first_only);

    let leading_indented = "  - solo";
    let unindent = augment_edit(leading_indented, leading_indented.len(), AugmentKind::ShiftTab)
        .expect("leading indented top-level item ShiftTab should outdent");
    assert_eq!(
        apply_augmentation_at(leading_indented, leading_indented.len(), &unindent),
        "- solo"
    );
    assert_eq!(unindent.cursor_byte_after, "- solo".len());

    // 2. 普通无序列表、引用内列表与任务列表第二项 Tab / Shift+Tab 往返与 AST 父子关系。
    for (source, cursor_needle, expected_indented) in [
        ("- first\n- second", "sec", "- first\n  - second"),
        ("> - first\n> - second", "sec", "> - first\n>   - second"),
        ("- [ ] first\n- [ ] second", "sec", "- [ ] first\n  - [ ] second"),
    ] {
        let cursor = source.find(cursor_needle).expect("needle exists") + 1;
        let indent_aug = augment_edit(source, cursor, AugmentKind::Tab)
            .unwrap_or_else(|| panic!("Tab should indent item in {source:?}"));
        let indented = apply_augmentation_at(source, cursor, &indent_aug);
        assert_eq!(indented, expected_indented);
        assert_eq!(
            indented.find(cursor_needle).expect("needle survives") + 1,
            indent_aug.cursor_byte_after
        );
        assert_eq!(
            list_item_nesting_depths(&indented),
            vec![(1, String::from("first")), (2, String::from("second"))],
            "AST must nest second under first for {indented:?}"
        );

        let outdent_aug =
            augment_edit(&indented, indent_aug.cursor_byte_after, AugmentKind::ShiftTab)
                .expect("ShiftTab should outdent item back to parent level");
        let restored = apply_augmentation_at(&indented, indent_aug.cursor_byte_after, &outdent_aug);
        assert_eq!(restored, source);
        assert_eq!(outdent_aug.cursor_byte_after, cursor);
    }

    // 3. 多位有序列表按前驱内容起始列（4 空格）对齐并保持嵌套 AST 结构
    // （注：CommonMark 仅允许 `1.` 或空行后的有序子列表打断紧邻段落，故用空行或 `1.` 验证 AST 嵌套）。
    let multi_tight = "10. first\n11. second";
    let multi_tight_cursor = "10. first\n11. sec".len();
    let multi_tight_tab = augment_edit(multi_tight, multi_tight_cursor, AugmentKind::Tab)
        .expect("multi-digit ordered item Tab should indent to 4 columns");
    assert_eq!(
        apply_augmentation_at(multi_tight, multi_tight_cursor, &multi_tight_tab),
        "10. first\n    11. second"
    );
    assert_eq!(multi_tight_tab.cursor_byte_after, "10. first\n    11. sec".len());

    for (source, cursor_needle, expected_indented) in [
        ("10. first\n\n11. second", "sec", "10. first\n\n    11. second"),
        ("10. first\n1. second", "sec", "10. first\n    1. second"),
    ] {
        let cursor = source.find(cursor_needle).expect("needle exists") + 1;
        let indent_aug = augment_edit(source, cursor, AugmentKind::Tab)
            .expect("multi-digit ordered item Tab should indent to 4 columns");
        let indented = apply_augmentation_at(source, cursor, &indent_aug);
        assert_eq!(indented, expected_indented);
        assert_eq!(
            indented.find(cursor_needle).expect("needle survives") + 1,
            indent_aug.cursor_byte_after
        );
        assert_eq!(
            list_item_nesting_depths(&indented),
            vec![(1, String::from("first")), (2, String::from("second"))]
        );
        let outdent_aug =
            augment_edit(&indented, indent_aug.cursor_byte_after, AugmentKind::ShiftTab)
                .expect("ShiftTab restores multi-digit ordered item");
        assert_eq!(
            apply_augmentation_at(&indented, indent_aug.cursor_byte_after, &outdent_aug),
            source
        );
        assert_eq!(outdent_aug.cursor_byte_after, cursor);
    }

    // 4. 完整子树（含续段、空行续段与子列表）同步移动，重解析 AST 校验 3 层级保留。
    let subtree = "- parent\n- target\n  continuation\n\n  more\n  - nested";
    let target_cursor = "- parent\n- tar".len();
    let subtree_tab = augment_edit(subtree, target_cursor, AugmentKind::Tab)
        .expect("subtree Tab must indent item, continuations, and child list");
    let subtree_indented = apply_augmentation_at(subtree, target_cursor, &subtree_tab);
    assert_eq!(
        subtree_indented,
        "- parent\n  - target\n    continuation\n\n    more\n    - nested"
    );
    assert_eq!(subtree_tab.cursor_byte_after, "- parent\n  - tar".len());
    assert_eq!(
        list_item_nesting_depths(&subtree_indented),
        vec![
            (1, String::from("parent")),
            (2, String::from("target continuation")),
            (3, String::from("nested")),
        ]
    );

    let subtree_shift =
        augment_edit(&subtree_indented, subtree_tab.cursor_byte_after, AugmentKind::ShiftTab)
            .expect("subtree ShiftTab must outdent complete subtree");
    assert_eq!(
        apply_augmentation_at(&subtree_indented, subtree_tab.cursor_byte_after, &subtree_shift),
        subtree
    );
    assert_eq!(subtree_shift.cursor_byte_after, target_cursor);
}

#[test]
fn ordered_list_renumbers_following_siblings_on_enter_split_and_exit() {
    // 1. 段尾 Enter 新增有序列表项，后续同级项在单次替换内顺延重编号（保留 `.` 与 `)` 分隔符）。
    for (source, cursor, expected_source, expected_cursor) in [
        (
            "1. first\n2. second\n3. third",
            "1. first".len(),
            "1. first\n2. \n3. second\n4. third",
            "1. first\n2. ".len(),
        ),
        (
            "1) first\n2) second",
            "1) first".len(),
            "1) first\n2) \n3) second",
            "1) first\n2) ".len(),
        ),
        ("9. nine\n10. ten", "9. nine".len(), "9. nine\n10. \n11. ten", "9. nine\n10. ".len()),
        // CRLF 与 blockquote 容器前缀兼容
        (
            "> 1. first\r\n> 2. second",
            "> 1. first".len(),
            "> 1. first\r\n> 2. \r\n> 3. second",
            "> 1. first\r\n> 2. ".len(),
        ),
        // 列表项中部拆分 + 保留嵌套子列表不受外层编号改动影响
        (
            "1. ab\n   1. sub1\n   2. sub2\n2. next",
            "1. a".len(),
            "1. a\n2. b\n   1. sub1\n   2. sub2\n3. next",
            "1. a\n2. ".len(),
        ),
        // 空有序列表项 Enter 退出时，后续同级项前移重编号
        (
            "1. first\n2. \n3. third\n4. fourth",
            "1. first\n2. ".len(),
            "1. first\n\n2. third\n3. fourth",
            "1. first\n".len(),
        ),
        // 引用容器内空有序列表项退出时保留 `> ` 且重编号后续同级项
        (
            "> 1. first\n> 2. \n> 3. third",
            "> 1. first\n> 2. ".len(),
            "> 1. first\n> \n> 2. third",
            "> 1. first\n> ".len(),
        ),
    ] {
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .unwrap_or_else(|| panic!("Enter must augment {source:?} at {cursor}"));
        let edited = apply_augmentation_at(source, cursor, &augmentation);
        assert_eq!(edited, expected_source, "renumber mismatch for {source:?} at {cursor}");
        assert_eq!(
            augmentation.cursor_byte_after, expected_cursor,
            "cursor mismatch for {source:?} at {cursor}"
        );
    }
}

#[test]
fn multiline_insert_in_containers_preserves_blockquote_and_list_continuation_prefixes() {
    for (source, cursor, pasted, expected, expected_cursor) in [
        // 1. BlockQuote 文本行粘贴多行（含空行与尾部换行），保留 `> ` 与 `>`。
        (
            "> first",
            "> first".len(),
            " second\nthird\n\nfourth",
            "> first second\n> third\n>\n> fourth",
            "> first second\n> third\n>\n> fourth".len(),
        ),
        // 2. 无序与有序列表项中粘贴多行，后续行按列表内容列缩进补齐前缀。
        ("- first", "- fir".len(), "X\nY", "- firX\n  Yst", "- firX\n  Y".len()),
        ("10. first", "10. first".len(), "\nsub", "10. first\n    sub", "10. first\n    sub".len()),
        // 3. 引用内嵌套列表项多行粘贴。
        (
            "> - first",
            "> - first".len(),
            "a\n\nb",
            "> - firsta\n>\n>   b",
            "> - firsta\n>\n>   b".len(),
        ),
        // 4. CRLF 引用块多行粘贴。
        (
            "> first\r\n> second",
            "> first".len(),
            " line\r\nadded",
            "> first line\r\n> added\r\n> second",
            "> first line\r\n> added".len(),
        ),
        // 5. 容器内虚拟可编辑空段粘贴多行，同时补齐隐藏分隔与容器续行前缀。
        (
            "> first\n> \n> after",
            "> first\n> ".len(),
            "mid1\nmid2",
            "> first\n> \n> mid1\n> mid2\n> \n> after",
            "> first\n> \n> mid1\n> mid2".len(),
        ),
    ] {
        let augmentation = augment_edit(source, cursor, AugmentKind::InsertText(pasted.to_owned()))
            .unwrap_or_else(|| panic!("container multiline insert must augment {source:?}"));
        assert_eq!(
            apply_augmentation_at(source, cursor, &augmentation),
            expected,
            "mismatch for {source:?}"
        );
        assert_eq!(augmentation.cursor_byte_after, expected_cursor, "wrong caret for {source:?}");
    }

    // 6. 容器内选区替换为多行文本也应补齐容器续行前缀。
    let quoted = "> hello world";
    let sel_start = "> hello ".len();
    let sel_end = quoted.len();
    let sel_aug = augment_selected_text(quoted, sel_start..sel_end, "a\nb")
        .expect("selected text inside blockquote should augment multiline paste");
    let mut edited = quoted.to_owned();
    edited.replace_range(sel_start..sel_end, "");
    let virtual_range = sel_aug.replace_range.unwrap_or(sel_start..sel_start);
    edited.replace_range(virtual_range, sel_aug.insert_text.as_deref().unwrap_or_default());
    assert_eq!(edited, "> hello a\n> b");
    assert_eq!(sel_aug.cursor_byte_after, "> hello a\n> b".len());
}
