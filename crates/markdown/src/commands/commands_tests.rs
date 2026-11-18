use super::*;

#[test]
fn table_structure_row_insertion_preserves_blockquote_prefix_and_adjacent_blocks() {
    let source = "before\n\n> | name | value |\n> | --- | --- |\n> | a | b |\n\n# after";
    let cursor = source.find("a | b").expect("blockquote table body exists");
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowAfter),
        cursor,
        None,
    );
    assert!(result.contains("> | a | b |\n> |  |  |"), "result: {result:?}");
    assert!(result.starts_with("before\n\n"));
    assert!(result.ends_with("\n\n# after"));
}

#[test]
fn deleting_last_container_table_row_clears_cells_without_losing_prefix() {
    let source = "> | heading | value |\n> | --- | --- |\n> | a | b |";
    let cursor = source.find("a | b").expect("container table body exists");
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
        cursor,
        None,
    );
    assert!(result.ends_with("> |  |  |"), "result: {result:?}");
    assert_eq!(parsed_table_column_count(&result), Some(2));
}

#[test]
fn container_row_insertion_preserves_crlf_and_adjacent_heading() {
    let source =
        "before\r\n\r\n> | heading | value |\r\n> | --- | --- |\r\n> | a | b |\r\n\r\n# after";
    let cursor = source.find("a | b").expect("container table body exists");
    assert!(source_table_at(source, cursor).is_some(), "container table should be modeled");
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowAfter),
        cursor,
        None,
    );
    assert!(result.contains("> | a | b |\r\n> |  |  |"), "result: {result:?}");
    assert!(!result.replace("\r\n", "").contains('\r'));
    assert!(result.ends_with("\r\n\r\n# after"));
}

#[test]
fn table_structure_column_edit_preserves_list_continuation_prefix() {
    let source = "- item\n\n  | name | value |\n  | --- | --- |\n  | a | b |";
    let cursor = source.find("a | b").expect("list table body exists");
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertColumnAfter),
        cursor,
        None,
    );
    assert!(result.contains("  | a |  | b |"), "result: {result:?}");
    assert!(result.contains("- item\n\n  | name |  | value |"), "result: {result:?}");
    assert_eq!(parsed_table_column_count(&result), Some(3));
}

#[test]
fn table_structure_conservatively_rejects_non_table_pipe_text() {
    let source = "A | B\n---x---\nx | y";
    assert_eq!(parsed_table_column_count(source), None);
    let cursor = source.find("x | y").expect("table body exists");
    assert_eq!(
        table_structure_capabilities(source, cursor),
        None,
        "pipe-like text rejected by the parser must remain unavailable"
    );
    assert_eq!(
        plan_semantic_edit(
            source,
            3,
            cursor,
            None,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::InsertColumnAfter
            )
        ),
        SemanticEditPlan::Unsupported
    );
}

#[test]
fn table_structure_alignment_edits_a_parser_recognized_table_without_outer_pipes() {
    let source = "A | B\n---|---\nx | y";
    assert_eq!(parsed_table_column_count(source), Some(2));
    let cursor = source.find("y").expect("second body cell exists");
    let capability = table_structure_capabilities(source, cursor);
    assert!(capability.is_some(), "the parsed GFM table should expose structure operations");
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::SetColumnAlignment(
            ui::plugin::TableColumnAlignment::Right,
        )),
        cursor,
        None,
    );
    assert_eq!(result, "A | B\n---|---:\nx | y");
}

#[test]
fn table_structure_cell_scanning_treats_unclosed_backticks_as_plain_text() {
    let source = "A | B\n---|---\nleft` | right";
    assert_eq!(parsed_table_column_count(source), Some(2));
    let cursor = source.find("right").expect("second body cell exists");
    assert!(table_structure_capabilities(source, cursor).is_some());
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::SetColumnAlignment(
            ui::plugin::TableColumnAlignment::Right,
        )),
        cursor,
        None,
    );
    assert_eq!(result, "A | B\n---|---:\nleft` | right");
}

#[test]
fn table_structure_row_and_column_edits_preserve_omitted_outer_pipe_style() {
    let source = "A | B\n---|---\nx | y";
    let cursor = source.find("y").expect("second body cell exists");
    let (inserted_row, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowAfter),
        cursor,
        None,
    );
    assert!(inserted_row.ends_with(" | "), "row insertion style: {inserted_row:?}");
    assert_eq!(parsed_table_column_count(&inserted_row), Some(2));

    let (inserted_column, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertColumnAfter),
        cursor,
        None,
    );
    assert_eq!(
        parsed_table_column_count(&inserted_column),
        Some(3),
        "inserted output: {inserted_column:?}"
    );
    assert!(
        inserted_column.lines().all(|line| !line.trim_start().starts_with('|')),
        "column edits must keep leading pipes omitted: {inserted_column:?}"
    );
    let inserted_rows = inserted_column.lines().collect::<Vec<_>>();
    assert!(inserted_rows[0].trim_end().ends_with('|'));
    assert!(inserted_rows[2].trim_end().ends_with('|'));

    let inserted_body_cursor = inserted_column.find("y").expect("body cell remains");
    let (deleted_column, _) = applied_text(
        &inserted_column,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteColumn),
        inserted_body_cursor,
        None,
    );
    assert_eq!(parsed_table_column_count(&deleted_column), Some(2));
    assert!(deleted_column.lines().all(|line| !line.trim_start().starts_with('|')));
}

#[test]
fn large_table_context_lookup_and_column_edit_complete_within_budget() {
    use std::time::{Duration, Instant};

    let column_count = 24;
    let body_row_count = 500;
    let make_row = |cells: Vec<String>| format_table_row(&cells);
    let header = make_row((0..column_count).map(|column| format!("h{column}")).collect());
    let separator = make_row(vec!["---".to_owned(); column_count]);
    let body = (0..body_row_count)
        .map(|row| make_row((0..column_count).map(|column| format!("r{row}c{column}")).collect()))
        .collect::<Vec<_>>();
    let source = std::iter::once(header)
        .chain(std::iter::once(separator))
        .chain(body)
        .collect::<Vec<_>>()
        .join("\n");
    let cursor = source.find("r250c12").expect("middle table cell exists");
    let started = Instant::now();
    let capabilities = table_structure_capabilities(&source, cursor);
    let plan = plan_semantic_edit(
        &source,
        5,
        cursor,
        None,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertColumnAfter),
    );
    assert!(capabilities.is_some(), "the large table should be recognized");
    assert!(matches!(plan, SemanticEditPlan::Apply(_)), "column edit should be planned");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "table context lookup and structure planning exceeded two seconds: {:?}",
        started.elapsed()
    );
}

#[test]
fn menu_table_creation_keeps_blockquote_context() {
    let source = "> paragraph";
    let cursor = source.len();
    let (result, cursor_after) = applied_text(
        source,
        SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
        cursor,
        None,
    );
    assert_eq!(parsed_table_count(&result, 2), 1, "result: {result:?}");
    assert!(result.starts_with("> paragraph\n> \n> | | |"), "result: {result:?}");
    assert_eq!(
        cursor_after,
        ui::plugin::EditSelection::Caret(result.find("| | |").expect("table header exists") + 2)
    );
}

#[test]
fn menu_table_creation_keeps_list_item_context() {
    let source = "- paragraph";
    let cursor = source.len();
    let expected = "- paragraph\n\n  | | |\n  | --- | --- |\n  | | |";
    assert_eq!(parsed_table_count(expected, 2), 1, "expected candidate must parse");
    assert!(container_paragraph_context(source, cursor).is_some());
    let (result, _) = applied_text(
        source,
        SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
        cursor,
        None,
    );
    assert_eq!(parsed_table_count(&result, 2), 1, "result: {result:?}");
    assert!(result.starts_with("- paragraph\n\n  | | |"), "result: {result:?}");
}

#[test]
fn menu_table_creation_in_blockquote_splits_at_start_middle_and_end() {
    let source = "> abcd";
    for (cursor, before, after) in [(2, "> ", "> abcd"), (4, "> ab", "> cd"), (6, "> abcd", "")] {
        let (result, cursor_after) = applied_text(
            source,
            SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
            cursor,
            None,
        );
        assert_eq!(parsed_table_count(&result, 2), 1, "cursor={cursor}, result={result:?}");
        assert!(result.starts_with(before), "cursor={cursor}, result: {result:?}");
        if !after.is_empty() {
            assert!(result.ends_with(after), "cursor={cursor}, result: {result:?}");
        }
        assert!(
            result.lines().filter(|line| line.contains('|')).all(|line| line.starts_with("> "))
        );
        assert_eq!(
            cursor_after,
            ui::plugin::EditSelection::Caret(
                result.find("| | |").expect("table header exists") + 2
            )
        );
    }
}

#[test]
fn menu_table_creation_in_list_splits_at_start_middle_and_end() {
    let source = "- abcd";
    for (cursor, before, after) in [(2, "- ", "  abcd"), (4, "- ab", "  cd"), (6, "- abcd", "")] {
        let (result, cursor_after) = applied_text(
            source,
            SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
            cursor,
            None,
        );
        assert_eq!(parsed_table_count(&result, 2), 1, "cursor={cursor}, result={result:?}");
        assert!(
            result.contains("  | | |"),
            "table rows need list continuation indentation: {result:?}"
        );
        assert!(result.starts_with(before), "cursor={cursor}, result: {result:?}");
        if !after.is_empty() {
            assert!(result.ends_with(after), "cursor={cursor}, result: {result:?}");
        }
        assert_eq!(
            cursor_after,
            ui::plugin::EditSelection::Caret(
                result.find("| | |").expect("table header exists") + 2
            )
        );
    }
}

#[test]
fn table_structure_capabilities_disable_header_and_only_column_deletion() {
    let source = "| only |\n| --- |\n| value |";
    let header = table_structure_capabilities(source, source.find("only").expect("header exists"))
        .expect("header lies in table");
    assert!(!header.can_delete_row);
    assert!(!header.can_delete_column);

    let body = table_structure_capabilities(source, source.find("value").expect("body exists"))
        .expect("body lies in table");
    assert!(body.can_delete_row);
    assert!(!body.can_delete_column);
}

#[test]
fn table_structure_commands_preserve_escaped_and_code_pipes() {
    let source = "| 名称 | 说明 |\n| --- | --- |\n| a\\|b | `x|y` |";
    let cursor = source.find("a\\|b").expect("table body cell exists");
    let (inserted, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertColumnAfter),
        cursor,
        None,
    );
    assert_eq!(parsed_table_column_count(&inserted), Some(3));
    assert!(inserted.contains("a\\|b"));
    assert!(inserted.contains("`x|y`"));
}

#[test]
fn row_and_column_insertions_and_deletions_keep_a_parseable_table() {
    let source = "| a | b |\r\n| --- | --- |\r\n| c | d |\r\n| e | f |";
    let cursor = source.find("c").expect("first body cell exists");
    let (row_inserted, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowBefore),
        cursor,
        None,
    );
    assert_eq!(parsed_table_column_count(&row_inserted), Some(2));
    assert!(row_inserted.contains("|  |\r\n| c | d |"));

    let (row_deleted, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
        cursor,
        None,
    );
    assert_eq!(row_deleted.matches("| --- | --- |").count(), 1);
    assert!(!row_deleted.contains("| c | d |"));
    assert!(row_deleted.contains("| e | f |"));

    let (column_deleted, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteColumn),
        cursor,
        None,
    );
    assert_eq!(parsed_table_column_count(&column_deleted), Some(1));
    assert!(
        column_deleted.contains("| b |\r\n| --- |\r\n| d |\r\n| f |"),
        "result: {column_deleted:?}"
    );
}

#[test]
fn table_structure_commands_protect_header_and_only_column() {
    let source = "| only |\n| --- |\n| value |";
    let cursor = source.find("value").expect("table body exists");
    let delete_column = plan_semantic_edit(
        source,
        7,
        cursor,
        None,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteColumn),
    );
    assert_eq!(delete_column, SemanticEditPlan::Unsupported);

    let header_cursor = source.find("only").expect("header exists");
    let delete_header = plan_semantic_edit(
        source,
        7,
        header_cursor,
        None,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
    );
    assert_eq!(delete_header, SemanticEditPlan::Unsupported);
}

#[test]
fn alignment_updates_only_its_separator_cell_and_accepts_short_body_rows() {
    let source = "| a | b |\n| --- | --- |\n| short |\n\nafter";
    let cursor = source.find("short").expect("body cell exists");
    let plan = plan_semantic_edit(
        source,
        7,
        cursor,
        None,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::SetColumnAlignment(
            ui::plugin::TableColumnAlignment::Right,
        )),
    );
    let SemanticEditPlan::Apply(transaction) = plan else {
        panic!("column alignment should produce a transaction");
    };
    assert_eq!(transaction.replacements.len(), 1);
    assert_eq!(&source[transaction.replacements[0].range.clone()], "---");
    assert_eq!(transaction.replacements[0].text, "---:");

    let (expanded, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertColumnAfter),
        cursor,
        None,
    );
    assert_eq!(parsed_table_column_count(&expanded), Some(3));
    assert!(expanded.ends_with("\n\nafter"));
}

#[test]
fn deleting_the_only_body_row_clears_it_and_table_delete_is_atomic() {
    let source = "before\n\n| h |\n| --- |\n| value |\n\nafter";
    let cursor = source.find("value").expect("table body exists");
    let (cleared, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
        cursor,
        None,
    );
    assert!(cleared.contains("| h |\n| --- |\n|  |"));
    assert!(cleared.starts_with("before\n\n") && cleared.ends_with("\n\nafter"));

    let (deleted, _) = applied_text(
        source,
        SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteTable),
        cursor,
        None,
    );
    assert_eq!(deleted, "before\n\nafter");
}

fn parsed_table_column_count(source: &str) -> Option<usize> {
    crate::parser::parse_markdown(source).events.into_iter().find_map(|event| match event {
        crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(alignments)) => {
            Some(alignments.len())
        }
        _ => None,
    })
}

fn applied_text(
    source: &str,
    command: SemanticEditCommand,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
) -> (String, ui::plugin::EditSelection) {
    let plan = plan_semantic_edit(source, 7, cursor_byte, selection, command);
    let SemanticEditPlan::Apply(transaction) = plan else {
        panic!("semantic command should produce an edit transaction");
    };
    let replacement =
        transaction.replacements.first().expect("semantic command should have one replacement");
    let mut result = source.to_owned();
    result.replace_range(replacement.range.clone(), &replacement.text);
    (result, transaction.selection_after)
}

#[test]
fn toggle_bold_without_selection_inserts_a_pair_and_places_the_caret_inside() {
    let (result, selection) =
        applied_text("标题", SemanticEditCommand::ToggleBold, "标题".len(), None);

    assert_eq!(result, "标题****");
    assert_eq!(selection, ui::plugin::EditSelection::Caret("标题".len() + 2));
}

#[test]
fn toggle_bold_wraps_a_single_line_selection() {
    let (result, _) = applied_text("中文文本", SemanticEditCommand::ToggleBold, 6, Some(0..6));

    assert_eq!(result, "**中文**文本");
}

#[test]
fn inline_commands_format_each_selected_paragraph() {
    let mut mismatches = Vec::new();
    for (command, expected) in [
        (SemanticEditCommand::ToggleBold, "**第一段**\n\n**第二段**"),
        (SemanticEditCommand::ToggleItalic, "*第一段*\n\n*第二段*"),
        (SemanticEditCommand::ToggleInlineCode, "`第一段`\n\n`第二段`"),
        (SemanticEditCommand::InsertLink, "[第一段](https://)\n\n[第二段](https://)"),
    ] {
        let source = "第一段\n\n第二段";
        let (formatted, _) =
            applied_text(source, command.clone(), source.len(), Some(0..source.len()));
        if formatted != expected {
            mismatches.push((command, formatted, expected));
        }
    }
    assert!(mismatches.is_empty(), "incorrect cross-paragraph formatting: {mismatches:?}");
}

#[test]
fn bold_and_italic_remove_formatting_from_every_selected_paragraph() {
    for (command, source) in [
        (SemanticEditCommand::ToggleBold, "**第一段**\n\n**第二段**"),
        (SemanticEditCommand::ToggleItalic, "*第一段*\n\n*第二段*"),
    ] {
        let (restored, _) =
            applied_text(source, command.clone(), source.len(), Some(0..source.len()));
        assert_eq!(restored, "第一段\n\n第二段", "command: {command:?}");
    }
}

#[test]
fn emphasis_toggles_preserve_other_styles_and_alternate_markers() {
    for (command, source, expected) in [
        (SemanticEditCommand::ToggleBold, "__第一段__\n\n__第二段__", "第一段\n\n第二段"),
        (SemanticEditCommand::ToggleItalic, "_第一段_\n\n_第二段_", "第一段\n\n第二段"),
        (SemanticEditCommand::ToggleBold, "***第一段***\n\n***第二段***", "*第一段*\n\n*第二段*"),
        (
            SemanticEditCommand::ToggleItalic,
            "***第一段***\n\n***第二段***",
            "**第一段**\n\n**第二段**",
        ),
        (SemanticEditCommand::ToggleBold, "**第一段**\n\n第二段", "**第一段**\n\n**第二段**"),
        (SemanticEditCommand::ToggleItalic, "*第一段*\n\n第二段", "*第一段*\n\n*第二段*"),
    ] {
        let (formatted, _) =
            applied_text(source, command.clone(), source.len(), Some(0..source.len()));
        assert_eq!(formatted, expected, "command: {command:?}, source: {source:?}");
    }
}

#[test]
fn cross_paragraph_inline_code_preserves_existing_code_and_safe_delimiters() {
    for (source, expected) in [
        ("`第一段`\n\n`第二段`", "第一段\n\n第二段"),
        ("`第一段`\n\n第二段", "`第一段`\n\n`第二段`"),
        ("a`b\n\nc`d", "``a`b``\n\n``c`d``"),
    ] {
        let (formatted, _) = applied_text(
            source,
            SemanticEditCommand::ToggleInlineCode,
            source.len(),
            Some(0..source.len()),
        );
        assert_eq!(formatted, expected, "source: {source:?}");
    }
}

#[test]
fn inline_commands_preserve_containers_and_atomic_blocks() {
    for (command, opening, closing) in [
        (SemanticEditCommand::ToggleBold, "**", "**"),
        (SemanticEditCommand::ToggleItalic, "*", "*"),
        (SemanticEditCommand::ToggleInlineCode, "`", "`"),
        (SemanticEditCommand::InsertLink, "[", "](https://)"),
    ] {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            for source in [
                format!("## 第一段{newline}{newline}- 第二段"),
                format!("> 第一段{newline}>{newline}> 第二段"),
                format!(
                    "第一段{newline}{newline}```{newline}代码{newline}```{newline}{newline}第二段"
                ),
                format!("| 第一段 |{newline}| --- |{newline}| 第二段 |"),
            ] {
                let expected = source
                    .replace("第一段", &format!("{opening}第一段{closing}"))
                    .replace("第二段", &format!("{opening}第二段{closing}"));
                let (formatted, _) =
                    applied_text(&source, command.clone(), source.len(), Some(0..source.len()));
                assert_eq!(formatted, expected, "command: {command:?}, source: {source:?}");
                let parsed = crate::parser::parse_markdown(&formatted);
                let formatted_spans = parsed
                    .events
                    .iter()
                    .filter(|event| {
                        matches!(
                            (&command, event),
                            (
                                SemanticEditCommand::ToggleBold,
                                crate::parser::MarkdownEvent::Start(
                                    crate::parser::MarkdownTag::Strong,
                                ),
                            ) | (
                                SemanticEditCommand::ToggleItalic,
                                crate::parser::MarkdownEvent::Start(
                                    crate::parser::MarkdownTag::Emphasis,
                                ),
                            ) | (
                                SemanticEditCommand::ToggleInlineCode,
                                crate::parser::MarkdownEvent::Code(_),
                            ) | (
                                SemanticEditCommand::InsertLink,
                                crate::parser::MarkdownEvent::Start(
                                    crate::parser::MarkdownTag::Link { .. },
                                ),
                            )
                        )
                    })
                    .count();
                assert_eq!(formatted_spans, 2, "generated markup must parse: {formatted:?}");
            }
        }
    }
}

#[test]
fn mixed_line_formatting_does_not_duplicate_existing_prefixes() {
    let mut mismatches = Vec::new();
    for (command, source, expected) in [
        (SemanticEditCommand::UnorderedList, "- 第一项\n第二项", "- 第一项\n- 第二项"),
        (SemanticEditCommand::OrderedList, "1. 第一项\n第二项", "1. 第一项\n1. 第二项"),
        (SemanticEditCommand::TaskList, "- [x] 第一项\n第二项", "- [x] 第一项\n- [ ] 第二项"),
        (SemanticEditCommand::Quote, "> 第一段\n第二段", "> 第一段\n> 第二段"),
    ] {
        let (formatted, _) =
            applied_text(source, command.clone(), source.len(), Some(0..source.len()));
        if formatted != expected {
            mismatches.push((command, formatted, expected));
        }
    }
    assert!(mismatches.is_empty(), "duplicated line prefixes: {mismatches:?}");
}

#[test]
fn strikethrough_formats_each_selected_paragraph() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source =
            format!("前文{newline}{newline}第一段{newline}{newline}第二段{newline}{newline}后文");
        let start = source.find("第一段").expect("first selected paragraph exists");
        let end = source.find("第二段").expect("last selected paragraph exists") + "第二段".len();
        let (formatted, _) =
            applied_text(&source, SemanticEditCommand::ToggleStrikethrough, end, Some(start..end));

        assert_eq!(
            formatted,
            source.replace("第一段", "~~第一段~~").replace("第二段", "~~第二段~~")
        );
        assert_eq!(
            crate::parser::parse_markdown(&formatted)
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Strikethrough)
                ))
                .count(),
            2,
        );
    }
}

#[test]
fn strikethrough_removes_markers_from_every_selected_paragraph() {
    let source = "~~第一段~~\n\n~~第二段~~\n\n~~第三段~~";
    let (restored, _) = applied_text(
        source,
        SemanticEditCommand::ToggleStrikethrough,
        source.len(),
        Some(0..source.len()),
    );

    assert_eq!(restored, "第一段\n\n第二段\n\n第三段");
}

#[test]
fn strikethrough_applies_consistently_to_mixed_selected_paragraphs() {
    let source = "~~第一段~~\n\n第二段\n\n~~第三段~~";
    let (formatted, _) = applied_text(
        source,
        SemanticEditCommand::ToggleStrikethrough,
        source.len(),
        Some(0..source.len()),
    );

    assert_eq!(formatted, "~~第一段~~\n\n~~第二段~~\n\n~~第三段~~");
}

#[test]
fn strikethrough_preserves_block_structure_and_round_trips() {
    for (source, expected) in [
        ("## 标题\n\n正文", "## ~~标题~~\n\n~~正文~~"),
        ("- 第一项\n- 第二项", "- ~~第一项~~\n- ~~第二项~~"),
        ("- [ ] 任务一\n- [x] 任务二", "- [ ] ~~任务一~~\n- [x] ~~任务二~~"),
        ("> 第一段\n>\n> 第二段", "> ~~第一段~~\n>\n> ~~第二段~~"),
        ("> 第一行\n> 第二行", "> ~~第一行\n> 第二行~~"),
        ("  第一段  \r\n\r\n第二段\r\n", "  ~~第一段~~  \r\n\r\n~~第二段~~\r\n"),
        ("正文\n\n```\n代码\n```\n\n末段", "~~正文~~\n\n```\n代码\n```\n\n~~末段~~"),
        ("| 标题 |\n| --- |\n| 内容 |", "| ~~标题~~ |\n| --- |\n| ~~内容~~ |"),
    ] {
        let (formatted, _) = applied_text(
            source,
            SemanticEditCommand::ToggleStrikethrough,
            source.len(),
            Some(0..source.len()),
        );
        assert_eq!(formatted, expected, "source: {source:?}");
        let (restored, _) = applied_text(
            &formatted,
            SemanticEditCommand::ToggleStrikethrough,
            formatted.len(),
            Some(0..formatted.len()),
        );
        assert_eq!(restored, source, "formatted: {formatted:?}");
    }
}

#[test]
fn strikethrough_only_formats_selected_parts_of_boundary_paragraphs() {
    let source = "前文第一段\n\n第二段后文";
    let start = "前文".len();
    let end = source.len() - "后文".len();
    let (formatted, _) =
        applied_text(source, SemanticEditCommand::ToggleStrikethrough, end, Some(start..end));
    assert_eq!(formatted, "前文~~第一段~~\n\n~~第二段~~后文");

    let start = formatted.find("第一段").expect("first paragraph exists");
    let end = formatted.find("第二段").expect("second paragraph exists") + "第二段".len();
    let (restored, _) =
        applied_text(&formatted, SemanticEditCommand::ToggleStrikethrough, end, Some(start..end));
    assert_eq!(restored, source);
}

#[test]
fn strikethrough_removes_multiple_spans_within_each_paragraph() {
    let source = "~~第一处~~ ~~第二处~~\n\n~~第三处~~";
    let (restored, _) = applied_text(
        source,
        SemanticEditCommand::ToggleStrikethrough,
        source.len(),
        Some(0..source.len()),
    );
    assert_eq!(restored, "第一处 第二处\n\n第三处");
}

#[test]
fn strikethrough_does_not_nest_markers_in_partially_formatted_paragraphs() {
    let source = "第一处 ~~第二处~~\n\n第三处";
    let (formatted, _) = applied_text(
        source,
        SemanticEditCommand::ToggleStrikethrough,
        source.len(),
        Some(0..source.len()),
    );
    assert_eq!(formatted, "~~第一处 第二处~~\n\n~~第三处~~");
}

#[test]
fn unordered_list_formats_every_line_in_a_multiline_selection() {
    let (result, _) = applied_text(
        "第一行\n第二行",
        SemanticEditCommand::UnorderedList,
        0,
        Some(0.."第一行\n第二行".len()),
    );

    assert_eq!(result, "- 第一行\n- 第二行");
}

#[test]
fn toggling_bold_again_removes_existing_nested_markers() {
    let (result, _) = applied_text(
        "**重点**",
        SemanticEditCommand::ToggleBold,
        "**重点**".len(),
        Some(0.."**重点**".len()),
    );

    assert_eq!(result, "重点");
}

#[test]
fn heading_command_does_not_demote_the_notora_title_h1() {
    let plan = plan_semantic_edit(
        "# 标题\n\n正文",
        7,
        2,
        Some(0..8),
        SemanticEditCommand::SetHeadingLevel(2),
    );

    assert_eq!(plan, SemanticEditPlan::NoChange);
}

#[test]
fn inline_toggle_wraps_a_selection_that_contains_only_one_marker() {
    for (command, marker) in [
        (SemanticEditCommand::ToggleBold, "**"),
        (SemanticEditCommand::ToggleItalic, "*"),
        (SemanticEditCommand::ToggleStrikethrough, "~~"),
    ] {
        let (result, _) = applied_text(marker, command, marker.len(), Some(0..marker.len()));

        assert_eq!(result, marker.repeat(3));
    }
}

#[test]
fn inline_code_command_preserves_embedded_backticks_and_boundary_spaces() {
    for selected in ["a`b", "`edge", "edge`", " leading", "trailing ", " both "] {
        let (formatted, _) = applied_text(
            selected,
            SemanticEditCommand::ToggleInlineCode,
            selected.len(),
            Some(0..selected.len()),
        );
        let parsed_code: Vec<String> = crate::parser::parse_markdown(&formatted)
            .events
            .into_iter()
            .filter_map(|event| match event {
                crate::parser::MarkdownEvent::Code(code) => Some(code),
                _ => None,
            })
            .collect();

        assert_eq!(parsed_code, [selected], "generated Markdown: {formatted:?}");

        let formatted_length = formatted.len();
        let (restored, _) = applied_text(
            &formatted,
            SemanticEditCommand::ToggleInlineCode,
            formatted_length,
            Some(0..formatted_length),
        );
        assert_eq!(restored, selected, "failed to toggle {formatted:?}");
    }
}

#[test]
fn inline_code_command_uses_safe_delimiters_for_a_single_backtick() {
    let (formatted, _) = applied_text(
        CODE_MARKER,
        SemanticEditCommand::ToggleInlineCode,
        CODE_MARKER.len(),
        Some(0..CODE_MARKER.len()),
    );

    assert_eq!(formatted, "`` ` ``");
}

#[test]
fn code_block_command_uses_a_fence_longer_than_content_runs() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("before{newline}```{newline}after");
        let (formatted, _) = applied_text(
            &source,
            SemanticEditCommand::CodeBlock,
            source.len(),
            Some(0..source.len()),
        );

        assert!(formatted.starts_with(&format!("````{newline}")));
        assert!(formatted.ends_with(&format!("{newline}````")));
        let parsed = crate::parser::parse_markdown(&formatted);
        assert_eq!(
            parsed
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    crate::parser::MarkdownEvent::Start(
                        crate::parser::MarkdownTag::CodeBlock { .. }
                    )
                ))
                .count(),
            1,
            "generated Markdown: {formatted:?}"
        );

        let formatted_length = formatted.len();
        let (restored, _) = applied_text(
            &formatted,
            SemanticEditCommand::CodeBlock,
            formatted_length,
            Some(0..formatted_length),
        );
        assert_eq!(restored, source);
    }
}

#[test]
fn code_block_toggle_accepts_a_longer_closing_fence() {
    for source in ["```\na\n````", "```rust\r\na\r\n````"] {
        let (restored, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));
        assert_eq!(restored, "a");
    }
}

#[test]
fn code_block_toggle_removes_an_empty_fenced_block() {
    let source = "```\n```";
    let (result, _) =
        applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));

    assert_eq!(result, "");
}

#[test]
fn line_commands_format_an_empty_document() {
    for (command, expected) in [
        (SemanticEditCommand::SetHeadingLevel(2), "## "),
        (SemanticEditCommand::UnorderedList, "- "),
        (SemanticEditCommand::OrderedList, "1. "),
        (SemanticEditCommand::TaskList, "- [ ] "),
        (SemanticEditCommand::Quote, "> "),
        (SemanticEditCommand::CodeBlock, "```\n\n```"),
    ] {
        let (result, _) = applied_text("", command, 0, None);

        assert_eq!(result, expected);
    }
}

#[test]
fn line_commands_format_a_blank_line_between_paragraphs() {
    for (command, expected) in [
        (SemanticEditCommand::SetHeadingLevel(2), "before\n## \nafter"),
        (SemanticEditCommand::UnorderedList, "before\n- \nafter"),
        (SemanticEditCommand::OrderedList, "before\n1. \nafter"),
        (SemanticEditCommand::TaskList, "before\n- [ ] \nafter"),
        (SemanticEditCommand::Quote, "before\n> \nafter"),
        (SemanticEditCommand::CodeBlock, "before\n```\n\n```\nafter"),
    ] {
        let (result, _) = applied_text("before\n\nafter", command, "before\n".len(), None);

        assert_eq!(result, expected);
    }
}

#[test]
fn heading_command_preserves_hashes_that_are_part_of_plain_text() {
    for source in ["#hashtag", "##正文", "####### seven hashes"] {
        let (result, _) = applied_text(source, SemanticEditCommand::SetHeadingLevel(2), 0, None);

        assert_eq!(result, format!("## {source}"));
    }
}

#[test]
fn line_command_keeps_the_crlf_sequence_outside_its_replacement_range() {
    let plan = plan_semantic_edit("first\r\nsecond", 7, 0, None, SemanticEditCommand::Quote);
    let SemanticEditPlan::Apply(transaction) = plan else {
        panic!("quote command should produce an edit transaction");
    };

    assert_eq!(transaction.replacements[0].range, 0.."first".len());
    assert_eq!(transaction.replacements[0].text, "> first");
}

#[test]
fn code_block_toggle_preserves_crlf_when_adding_fences() {
    let source = "first\r\nsecond";
    let (result, _) =
        applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));

    assert_eq!(result, "```\r\nfirst\r\nsecond\r\n```");
}

#[test]
fn code_block_toggle_preserves_crlf_when_removing_fences() {
    let source = "```rust\r\nfirst\r\nsecond\r\n```\r\nafter";
    let closing_end = source.find("\r\nafter").expect("fixture has a paragraph after the fence");
    let (result, _) = applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..closing_end));

    assert_eq!(result, "first\r\nsecond\r\nafter");
}

#[test]
fn code_block_toggle_preserves_a_selected_blank_line_after_the_closing_fence() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("```{newline}a{newline}```{newline}{newline}last");
        let selection_end = source.find("last").expect("fixture ends with a paragraph");
        let (result, _) =
            applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

        assert_eq!(result, format!("a{newline}{newline}last"));
    }
}

#[test]
fn empty_code_block_toggle_preserves_a_selected_blank_line_after_the_closing_fence() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("```{newline}```{newline}{newline}last");
        let selection_end = source.find("last").expect("fixture ends with a paragraph");
        let (result, _) =
            applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

        assert_eq!(result, format!("{newline}{newline}last"));
    }
}

#[test]
fn code_block_toggle_preserves_multiple_selected_blank_lines_after_the_closing_fence() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("```{newline}a{newline}```{newline}{newline}{newline}last");
        let selection_end = source.find("last").expect("fixture ends with a paragraph");
        let (result, _) =
            applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

        assert_eq!(result, format!("a{newline}{newline}{newline}last"));
    }
}

#[test]
fn multiline_heading_selection_formats_its_last_blank_line() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("first{newline}{newline}last");
        let selection_end = format!("first{newline}{newline}").len();
        let (result, _) = applied_text(
            &source,
            SemanticEditCommand::SetHeadingLevel(2),
            0,
            Some(0..selection_end),
        );

        assert_eq!(result, format!("## first{newline}## {newline}last"));
    }
}

#[test]
fn multiline_quote_selection_counts_its_last_blank_line_when_toggling() {
    for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
        let source = format!("> first{newline}{newline}last");
        let selection_end = format!("> first{newline}{newline}").len();
        let (result, _) =
            applied_text(&source, SemanticEditCommand::Quote, 0, Some(0..selection_end));

        assert_eq!(result, format!("> first{newline}> {newline}last"));
    }
}

#[test]
fn first_h1_protection_ignores_code_block_hashes_while_guarding_real_top_level_atx_h1() {
    let fenced_then_h2 = "```\n# code comment\n```\n\n## Sub";
    let h2_cursor = fenced_then_h2.find("## Sub").expect("h2 exists");
    let (promoted, _) =
        applied_text(fenced_then_h2, SemanticEditCommand::SetHeadingLevel(3), h2_cursor, None);
    assert_eq!(promoted, "```\n# code comment\n```\n\n### Sub");

    let indented_then_h2 = "    # indented comment\n\n## Sub";
    let indented_h2_cursor = indented_then_h2.find("## Sub").expect("h2 exists");
    let (indented_promoted, _) = applied_text(
        indented_then_h2,
        SemanticEditCommand::SetHeadingLevel(3),
        indented_h2_cursor,
        None,
    );
    assert_eq!(indented_promoted, "    # indented comment\n\n### Sub");

    let code_then_real_h1 = "```\n# code comment\n```\n\n# Real H1\n\nBody";
    let real_h1_cursor = code_then_real_h1.find("# Real H1").expect("real h1 exists") + 2;
    let plan = plan_semantic_edit(
        code_then_real_h1,
        7,
        real_h1_cursor,
        None,
        SemanticEditCommand::SetHeadingLevel(2),
    );
    assert_eq!(plan, SemanticEditPlan::NoChange);
}

#[test]
fn heading_command_converts_setext_heading_and_removes_underline() {
    let setext_h2 = "Setext Title\n---\n\nafter";
    for cursor in [0usize, "Setext Title\n-".len()] {
        let (result, _) =
            applied_text(setext_h2, SemanticEditCommand::SetHeadingLevel(3), cursor, None);
        assert_eq!(result, "### Setext Title\n\nafter");
    }

    let setext_h1 = "Setext H1\n===\n\nafter";
    let (h1_converted, _) =
        applied_text(setext_h1, SemanticEditCommand::SetHeadingLevel(2), 0, None);
    assert_eq!(h1_converted, "## Setext H1\n\nafter");

    let crlf_setext = "Setext\r\n===\r\nafter";
    let (crlf_converted, _) =
        applied_text(crlf_setext, SemanticEditCommand::SetHeadingLevel(2), 0, None);
    assert_eq!(crlf_converted, "## Setext\r\nafter");

    let closed_atx = "# Keep\n\n## Closed ##";
    let closed_cursor = closed_atx.find("## Closed").expect("closed atx exists");
    let (closed_converted, _) =
        applied_text(closed_atx, SemanticEditCommand::SetHeadingLevel(3), closed_cursor, None);
    assert_eq!(closed_converted, "# Keep\n\n### Closed");
}

#[test]
fn list_prefix_commands_convert_between_unordered_ordered_and_task_without_nesting() {
    for (source, command, expected) in [
        ("- item", SemanticEditCommand::OrderedList, "1. item"),
        ("- item", SemanticEditCommand::TaskList, "- [ ] item"),
        ("+ item", SemanticEditCommand::OrderedList, "1. item"),
        ("1) item", SemanticEditCommand::UnorderedList, "- item"),
        ("1. item", SemanticEditCommand::TaskList, "- [ ] item"),
        ("- [ ] item", SemanticEditCommand::UnorderedList, "- item"),
        ("- [x] item", SemanticEditCommand::OrderedList, "1. item"),
        ("- [ ] a\n* b\n1) c", SemanticEditCommand::TaskList, "- [ ] a\n- [ ] b\n- [ ] c"),
        ("- [x] a\n1. b", SemanticEditCommand::UnorderedList, "- a\n- b"),
        ("- [x] a\n+ b", SemanticEditCommand::UnorderedList, "- a\n+ b"),
        ("1) first", SemanticEditCommand::OrderedList, "first"),
    ] {
        let (result, _) = applied_text(source, command.clone(), 0, Some(0..source.len()));
        assert_eq!(result, expected, "conversion failed for {source:?} with {command:?}");
    }
}
