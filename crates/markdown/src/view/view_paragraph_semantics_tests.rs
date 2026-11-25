use super::*;

fn rendered_row_count(view: &MarkdownEditorView) -> usize {
    view.engine.flat_lines().len()
}

#[test]
fn paragraph_enter_adds_one_row_at_each_source_boundary() {
    let mut violations = Vec::new();
    for newline in ["\n", "\r\n"] {
        for prefix in ["head", "# Title", "## Title"] {
            for suffix in ["", "\n", "\n\n", "\n\ntail", "\n\n---", "\n \ntail", "\n\t\ntail"] {
                let source = format!("{prefix}{}", suffix.replace('\n', newline));
                let (entered, cursor) = apply_edit(&source, prefix.len(), AugmentKind::Enter);
                let mut view = MarkdownEditorView::new();
                render_at(&mut view, &source, prefix.len(), 1, 1.0);
                let original_rows = rendered_row_count(&view);
                render_at(&mut view, &entered, cursor, 2, 1.0);
                let entered_rows = rendered_row_count(&view);
                if entered_rows != original_rows + 1 {
                    violations.push(format!(
                        "{source:?} -> {entered:?}: {original_rows} -> {entered_rows}"
                    ));
                }
            }
        }
    }
    assert!(violations.is_empty(), "one Enter must create one paragraph: {violations:#?}");
}

fn editable_empty_paragraph_count(source: &str) -> usize {
    fn count(blocks: &[crate::builder::BlockNode]) -> usize {
        blocks
            .iter()
            .map(|block| usize::from(block.is_editable_empty_paragraph()) + count(&block.children))
            .sum()
    }
    let parsed = crate::parser::parse_markdown(source);
    let doc = crate::builder::MarkdownDoc::build_for_editing(
        &parsed,
        &crate::test_utils::default_style(),
        source,
    );
    count(&doc.blocks)
}

#[test]
fn paragraph_enter_at_inline_breaks_adds_one_row_and_zero_empty_paragraphs() {
    let user_paste = "不做浏览器插件。\nPDF阅读能力加强\n新建PDF的推荐模版有问题";
    for cursor in ["不做浏览器插件。".len(), "不做浏览器插件。\n".len()] {
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, user_paste, cursor, 1, 1.0);
        let original_rows = rendered_row_count(&view);
        let (entered, entered_cursor) = apply_edit(user_paste, cursor, AugmentKind::Enter);
        assert_eq!(editable_empty_paragraph_count(&entered), 0, "{entered:?}");
        render_at(&mut view, &entered, entered_cursor, 2, 1.0);
        assert_eq!(rendered_row_count(&view), original_rows + 1, "{entered:?}");
    }

    for newline in ["\n", "\r\n"] {
        for (source_pattern, offset) in [
            ("left\nright", "left".len()),
            ("left\nright", "left\n".len()),
            ("left \n   right", "left".len()),
            ("left  \nright", "left".len()),
            ("left  \nright", "left ".len()),
            ("left  \nright", "left  ".len()),
            ("left\\\nright", "left".len()),
            ("left<br>right", "left".len()),
            ("left<br>\nright", "left".len()),
        ] {
            let source = source_pattern.replace('\n', newline);
            let cursor = if source_pattern.contains('\n')
                && offset > source_pattern.find('\n').unwrap_or(usize::MAX)
            {
                offset + newline.len() - 1
            } else {
                offset
            };
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, cursor, 1, 1.0);
            let original_rows = rendered_row_count(&view);
            let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
            assert_eq!(editable_empty_paragraph_count(&entered), 0, "{source:?} -> {entered:?}");
            render_at(&mut view, &entered, entered_cursor, 2, 1.0);
            let expected_rows = if source_pattern.contains("  \n")
                || source_pattern.contains("\\\n")
                || source_pattern.contains("<br>")
            {
                original_rows
            } else {
                original_rows + 1
            };
            assert_eq!(rendered_row_count(&view), expected_rows, "{source:?} -> {entered:?}");
        }
    }

    for (source, cursor) in
        [("head\t\n\ntail", "head".len()), ("head\n\u{3000}\ntail", "head".len())]
    {
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, source, cursor, 1, 1.0);
        let original_rows = rendered_row_count(&view);
        let (entered, entered_cursor) = apply_edit(source, cursor, AugmentKind::Enter);
        render_at(&mut view, &entered, entered_cursor, 2, 1.0);
        assert_eq!(rendered_row_count(&view), original_rows + 1, "{source:?} -> {entered:?}");
    }
}

#[test]
fn paragraph_enter_at_document_start_adds_only_one_empty_row() {
    for source in ["head", "段落", "head\n\ntail"] {
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, source, 0, 1, 1.0);
        let original_rows = rendered_row_count(&view);
        let (entered, cursor) = apply_edit(source, 0, AugmentKind::Enter);
        assert_eq!(entered, format!("\n{source}"));
        assert_eq!(cursor, 1);
        render_at(&mut view, &entered, cursor, 2, 1.0);
        assert_eq!(rendered_row_count(&view), original_rows + 1);
    }
}

#[test]
fn heading_start_enter_keeps_heading_and_inserts_one_body_paragraph_before_it() {
    for newline in ["\n", "\r\n"] {
        for heading in ["# Title", "##  Title", "Title\n===", "Title\n---"] {
            for prefix in ["", "head\n\n"] {
                let prefix = prefix.replace('\n', newline);
                let heading = heading.replace('\n', newline);
                let source = format!("{prefix}{heading}");
                let cursor = source.find("Title").expect("fixture contains heading content");
                let inserted_newline = if source.contains("\r\n") { "\r\n" } else { "\n" };
                let (entered, after_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                assert_eq!(
                    entered,
                    format!("{prefix}{inserted_newline}{heading}"),
                    "heading must retain its style at the content start"
                );
                assert_eq!(after_cursor, cursor + inserted_newline.len());
                let mut view = MarkdownEditorView::new();
                render_at(&mut view, &source, cursor, 1, 1.0);
                let original_rows = rendered_row_count(&view);
                render_at(&mut view, &entered, after_cursor, 2, 1.0);
                assert_eq!(rendered_row_count(&view), original_rows + 1);
            }
        }
    }
}

fn apply_policy_erasure(
    source: &str,
    cursor: usize,
    selection: Option<std::ops::Range<usize>>,
    intent: ui::plugin::EditIntent,
) -> (String, usize) {
    use ui::plugin::{EditPlan, EditPolicy, EditRequest, EditSelection};
    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    let request = EditRequest { source_generation: 1, cursor_byte: cursor, selection, intent };
    let mut edited = source.to_owned();
    match view.plan_edit(&request) {
        EditPlan::Apply(transaction) | EditPlan::ApplyDefault(transaction, _) => {
            assert_eq!(transaction.source_generation, 1);
            for replacement in transaction.replacements.iter().rev() {
                edited.replace_range(replacement.range.clone(), &replacement.text);
            }
            let EditSelection::Caret(cursor) = transaction.selection_after else {
                panic!("paragraph erasure must collapse the selection");
            };
            (edited, cursor)
        }
        EditPlan::UseDefault => {
            let erased = request.selection.unwrap_or_else(|| match request.intent {
                ui::plugin::EditIntent::DeleteBackward => {
                    let start = grapheme_start_indices(&source[..cursor])
                        .last()
                        .map_or(cursor, |(_, byte)| byte);
                    start..cursor
                }
                ui::plugin::EditIntent::DeleteForward => {
                    cursor
                        ..cursor + crate::grapheme_map::byte_at_grapheme_index(&source[cursor..], 1)
                }
                _ => panic!("test helper only supports erasure"),
            });
            let cursor = erased.start;
            edited.replace_range(erased, "");
            (edited, cursor)
        }
        EditPlan::Consume => (edited, cursor),
        EditPlan::MoveCursor(update) => (edited, update.cursor_after),
        EditPlan::SetSelection(_) => panic!("erasure must collapse selection"),
    }
}

#[test]
fn paragraph_erasure_policy_is_direction_independent_and_keeps_geometry() {
    use ui::plugin::EditIntent;
    for newline in ["\n", "\r\n"] {
        for follower in ["b", "# heading", "---", "- item", "> quote", "```\ncode\n```"] {
            for content in ["x", "👩‍💻", "e\u{301}", "**x**", "[x](url)"] {
                let source = format!("a{newline}{newline}{content}{newline}{newline}{follower}");
                let visible = if content.contains('x') { "x" } else { content };
                let start = source.find(visible).expect("fixture has visible content");
                let end = start + visible.len();
                let expected = format!("a{newline}{newline}{newline}{follower}");
                let expected_cursor = 1 + newline.len() * 2;
                for (cursor, selection, intent) in [
                    (end, None, EditIntent::DeleteBackward),
                    (start, None, EditIntent::DeleteForward),
                    (end, Some(start..end), EditIntent::DeleteBackward),
                    (end, Some(start..end), EditIntent::DeleteForward),
                ] {
                    let (edited, cursor) = apply_policy_erasure(&source, cursor, selection, intent);
                    assert_eq!((&edited, cursor), (&expected, expected_cursor), "{source:?}");
                }
                if content == "x" || content == "👩‍💻" {
                    for dpi in [1.0, 2.0] {
                        let mut view = MarkdownEditorView::new();
                        let empty_geometry =
                            render_at(&mut view, &expected, expected_cursor, 1, dpi);
                        let typed_geometry = render_at(&mut view, &source, start, 2, dpi);
                        let erased_geometry =
                            render_at(&mut view, &expected, expected_cursor, 3, dpi);
                        assert!((empty_geometry.1 - typed_geometry.1).abs() < GEOMETRY_TOLERANCE);
                        assert_eq!(empty_geometry, erased_geometry);
                        assert_eq!(
                            view.engine.hit_test_byte(
                                erased_geometry.0,
                                erased_geometry.1 + erased_geometry.2 * 0.5,
                                0.0,
                                0.0
                            ),
                            Some(expected_cursor)
                        );
                    }
                }
            }
        }
    }
}

#[path = "view_heading_container_tests.rs"]
mod heading_container_tests;

#[test]
fn paragraph_boundary_policy_removes_one_neighbor_before_merging_blocks() {
    use ui::plugin::EditIntent;
    for newline in ["\n", "\r\n"] {
        for before in ["a", "# title", "> quote", "- item", "**words**"] {
            for after in ["b", "# title", "> quote", "- item", "_words_", "**words**"] {
                let source = format!("{before}{}{after}", newline.repeat(4));
                let expected = format!("{before}{}{after}", newline.repeat(3));
                let before_cursor = before.strip_suffix("**").map_or(before.len(), str::len);
                let after_offset = after
                    .find(|character: char| character.is_alphanumeric())
                    .expect("fixture contains visible text");
                let after_cursor = before.len() + newline.len() * 4 + after_offset;
                for (cursor, intent, expected_cursor) in [
                    (before_cursor, EditIntent::DeleteForward, before_cursor),
                    (after_cursor, EditIntent::DeleteBackward, after_cursor - newline.len()),
                ] {
                    let (edited, cursor) = apply_policy_erasure(&source, cursor, None, intent);
                    assert_eq!((&edited, cursor), (&expected, expected_cursor), "{source:?}");
                }
            }
        }
    }
}

#[test]
fn heading_start_enter_then_backspace_restores_heading_source_and_caret() {
    for source in ["# Title", "head\n\n## Title", "Title\n===", "head\r\n\r\n# Title"] {
        let cursor = source.find("Title").expect("fixture has heading content");
        let (entered, entered_cursor) = apply_edit(source, cursor, AugmentKind::Enter);
        assert_eq!(
            apply_edit(&entered, entered_cursor, AugmentKind::Backspace),
            (source.to_owned(), cursor)
        );
    }
}

#[test]
fn paragraph_selection_policy_keeps_partial_and_invalid_erasure_on_default_path() {
    use ui::plugin::{EditIntent, EditPlan, EditPolicy, EditRequest};
    for source in ["a\n\nwords\n\nb", "```\nwords\n```", "`words`", "é"] {
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        let partial_start = source.find("words").unwrap_or(0);
        for selection in [partial_start..partial_start + 1, 0..source.len() + 1] {
            let request = EditRequest {
                source_generation: 1,
                cursor_byte: selection.end,
                selection: Some(selection),
                intent: EditIntent::DeleteBackward,
            };
            assert_eq!(view.plan_edit(&request), EditPlan::UseDefault);
        }
    }
}

#[test]
fn empty_paragraph_delete_keeps_caret_before_each_following_block() {
    for follower in ["b", "# heading", "---", "> quote", "- item", "```\ncode\n```", "| a |\n| - |"]
    {
        let source = format!("a\n\n\n{follower}");
        let cursor = "a\n\n".len();
        let (edited, cursor) =
            apply_policy_erasure(&source, cursor, None, ui::plugin::EditIntent::DeleteForward);
        assert_eq!(edited, format!("a\n\n{follower}"));
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, &source, "a\n\n".len(), 1, 1.0);
        let original_rows = rendered_row_count(&view);
        render_at(&mut view, &edited, cursor, 2, 1.0);
        if !follower.starts_with(['`', '|']) {
            assert_eq!(rendered_row_count(&view) + 1, original_rows, "{source:?}");
        }
    }
}

#[test]
fn creation_visible_styled_start_preserves_source_and_adds_one_row() {
    for styled in ["**Title**", "***Title***", "[Title](url)", "[**Title**](url)"] {
        for source in [styled.to_owned(), format!("# {styled}"), format!("{styled}\n===")] {
            let cursor = source.find("Title").expect("fixture contains visible content");
            let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
            assert_eq!(entered, format!("\n{source}"), "{source:?}");
            assert_eq!(entered_cursor, cursor + 1);
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, cursor, 1, 1.0);
            let original_rows = rendered_row_count(&view);
            render_at(&mut view, &entered, entered_cursor, 2, 1.0);
            assert_eq!(rendered_row_count(&view), original_rows + 1);
        }
    }
}

#[test]
fn creation_heading_without_existing_separator_adds_one_editable_row() {
    for newline in ["\n", "\r\n"] {
        for before in ["a", "a\n---", "---", "- item", "> quote"] {
            for heading in ["# Title", "# **Title**"] {
                let before = before.replace('\n', newline);
                let source = format!("{before}{newline}{heading}");
                let cursor = source.find("Title").expect("fixture contains title");
                let mut view = MarkdownEditorView::new();
                render_at(&mut view, &source, cursor, 1, 1.0);
                let original_rows = rendered_row_count(&view);
                let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                assert!(entered.ends_with(heading), "heading syntax must stay intact");
                render_at(&mut view, &entered, entered_cursor, 2, 1.0);
                assert_eq!(
                    rendered_row_count(&view),
                    original_rows + 1,
                    "{source:?} -> {entered:?}"
                );
                let (restored, restored_cursor) =
                    apply_edit(&entered, entered_cursor, AugmentKind::Backspace);
                render_at(&mut view, &restored, restored_cursor, 3, 1.0);
                assert_eq!(rendered_row_count(&view), original_rows, "{restored:?}");
            }
        }
    }
}

#[test]
fn heading_visible_end_enter_preserves_hidden_closing_syntax() {
    for newline in ["\n", "\r\n"] {
        for heading in ["# Title ##", "# **Title**", "# [Title](url)", "# **Title** ##"] {
            for follower in ["", "tail", "---", "# Next", "- item"] {
                let suffix = if follower.is_empty() {
                    String::new()
                } else {
                    format!("{newline}{newline}{follower}")
                };
                let source = format!("{heading}{suffix}");
                let newline = if source.contains("\r\n") { "\r\n" } else { "\n" };
                let cursor = source.find("Title").expect("fixture contains title") + "Title".len();
                let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                let expected = if follower.is_empty() {
                    format!("{heading}{newline}{newline}")
                } else {
                    format!("{heading}{newline}{newline}{newline}{follower}")
                };
                assert_eq!(entered, expected, "visible title end: {source:?}");
                assert_eq!(entered_cursor, heading.len() + newline.len() * 2);
                let mut view = MarkdownEditorView::new();
                render_at(&mut view, &source, cursor, 1, 1.0);
                let original_rows = rendered_row_count(&view);
                render_at(&mut view, &entered, entered_cursor, 2, 1.0);
                assert_eq!(rendered_row_count(&view), original_rows + 1);
            }
        }
    }
}

#[test]
fn heading_visible_end_delete_removes_one_neighbor_before_closing_syntax() {
    for newline in ["\n", "\r\n"] {
        for heading in ["# Title ##", "# **Title** ##", "# [Title](url) ##"] {
            let source = format!("{heading}{newline}{newline}{newline}tail");
            let cursor = source.find("Title").expect("fixture contains title") + "Title".len();
            let (edited, edited_cursor) =
                apply_policy_erasure(&source, cursor, None, ui::plugin::EditIntent::DeleteForward);
            assert_eq!(edited, format!("{heading}{newline}{newline}tail"), "{source:?}");
            assert_eq!(edited_cursor, cursor);
        }
    }
}

#[test]
fn paragraph_erasure_with_hidden_trailing_whitespace_is_direction_independent() {
    use ui::plugin::EditIntent;
    for newline in ["\n", "\r\n"] {
        for content in ["x ", "x\t", "**x** ", "[x](url)\t"] {
            for follower in ["tail", "---", "# Heading", "- item"] {
                let source = format!("a{newline}{newline}{content}{newline}{newline}{follower}");
                let start = source.find('x').expect("fixture contains editable text");
                let expected = format!("a{newline}{newline}{newline}{follower}");
                for (cursor, selection, intent) in [
                    (start + 1, None, EditIntent::DeleteBackward),
                    (start, None, EditIntent::DeleteForward),
                    (start + 1, Some(start..start + 1), EditIntent::DeleteBackward),
                ] {
                    let (edited, cursor) = apply_policy_erasure(&source, cursor, selection, intent);
                    assert_eq!(edited, expected, "erasing visible content of {source:?}");
                    assert_eq!(cursor, 1 + newline.len() * 2);
                }
            }
        }
    }
}

#[test]
fn closed_heading_final_boundary_merge_keeps_inline_style_and_removes_block_suffix() {
    use ui::plugin::EditIntent;
    for newline in ["\n", "\r\n"] {
        for (heading, merged_heading) in [
            ("# Title ##", "# Title"),
            ("# **Title** ##", "# **Title**"),
            ("# [Title](url) ##", "# [Title](url)"),
            ("# Title ##  ", "# Title"),
        ] {
            for blank in ["", " ", "\t"] {
                let source = format!("{heading}{newline}{blank}{newline}tail");
                let expected = format!("{merged_heading}tail");
                let title_end =
                    source.find("Title").expect("fixture contains title") + "Title".len();
                for (cursor, intent) in [
                    (title_end, EditIntent::DeleteForward),
                    (heading.len(), EditIntent::DeleteForward),
                    (source.len() - "tail".len(), EditIntent::DeleteBackward),
                ] {
                    let (edited, _) = apply_policy_erasure(&source, cursor, None, intent);
                    assert_eq!(edited, expected, "final hidden separator: {source:?}");
                }
            }
        }
    }
}

#[test]
fn heading_final_boundary_preserves_literals_and_empty_heading_prefix() {
    for (heading, joined) in [
        ("# Title \\##", "# Title \\##tail"),
        ("# Title#", "# Title#tail"),
        ("# Title ## \t", "# Titletail"),
        ("# ##", "# tail"),
        ("### ###", "### tail"),
    ] {
        let source = format!("{heading}\n\ntail");
        let (edited, _) = apply_policy_erasure(
            &source,
            heading.len(),
            None,
            ui::plugin::EditIntent::DeleteForward,
        );
        assert_eq!(edited, joined, "{source:?}");
    }
}

#[test]
fn heading_visible_end_delete_keeps_atomic_neighbors_and_eof_intact() {
    for heading in ["# Title ##", "# **Title**", "# [Title](url) ##", "# **Title**  "] {
        for suffix in ["", "\n\n---", "\n\n# Next", "\n\n- item", "\n\n```\ncode\n```"] {
            let source = format!("{heading}{suffix}");
            let cursor = source.find("Title").expect("fixture contains title") + "Title".len();
            assert_eq!(
                apply_policy_erasure(&source, cursor, None, ui::plugin::EditIntent::DeleteForward),
                (source, cursor),
            );
        }
    }
}

fn run_selection_intent_with_single_undo(
    source: &str,
    selection: std::ops::Range<usize>,
    intent: ui::plugin::EditIntent,
) -> (String, usize) {
    use ui::plugin::{EditPlan, EditPolicy, EditRequest, EditSelection};

    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: selection.end,
        selection: Some(selection.clone()),
        intent: intent.clone(),
    };
    let mut edited = source.to_owned();
    match view.plan_edit(&request) {
        EditPlan::Apply(transaction) | EditPlan::ApplyDefault(transaction, _) => {
            let mut undo_stack: Vec<(std::ops::Range<usize>, String)> = Vec::new();
            for replacement in transaction.replacements.iter().rev() {
                let start = replacement.range.start;
                let replaced_len = replacement.text.len();
                let original_slice = edited[replacement.range.clone()].to_owned();
                edited.replace_range(replacement.range.clone(), &replacement.text);
                undo_stack.push((start..start + replaced_len, original_slice));
            }
            // Verify single-transaction Undo restores original source cleanly.
            let mut undone = edited.clone();
            for (applied_range, original_slice) in undo_stack.into_iter().rev() {
                undone.replace_range(applied_range, &original_slice);
            }
            assert_eq!(undone, source, "single transaction Undo must restore original source");
            let caret = match transaction.selection_after {
                EditSelection::Caret(c) => c,
                EditSelection::Range { cursor, .. } => cursor,
            };
            (edited, caret)
        }
        EditPlan::Consume => (edited, selection.start),
        EditPlan::MoveCursor(update) => (edited, update.cursor_after),
        EditPlan::UseDefault => {
            let text = match &intent {
                ui::plugin::EditIntent::InsertText(t) => t.as_str(),
                ui::plugin::EditIntent::InsertParagraphBreak
                | ui::plugin::EditIntent::InsertLineBreak => "\n",
                _ => "",
            };
            edited.replace_range(selection.clone(), text);
            (edited, selection.start + text.len())
        }
        EditPlan::SetSelection(_) => panic!("unexpected selection result"),
    }
}

#[test]
fn selection_edits_protect_fences_tables_and_setext_across_all_intents_and_undo() {
    use ui::plugin::EditIntent;

    // 1. Cross-fence selections: never break opening/closing fences across Delete, Type, Paste, Enter.
    let fenced_doc = "para\n\n```\nline1\nline2\n```\n\nafter";
    let body_to_closing_start = fenced_doc.find("line2").expect("body exists");
    let body_to_closing_end = fenced_doc.find("\n\nafter").expect("closing fence end exists");
    let closing_only_start = fenced_doc.find("\n```\n\nafter").expect("closing fence") + 1;
    let open_to_body_start = fenced_doc.find("```").expect("opening fence");
    let open_to_body_end = fenced_doc.find("\nline2").expect("line1 end");

    for intent in [
        EditIntent::DeleteBackward,
        EditIntent::DeleteForward,
        EditIntent::InsertText(String::from("X")),
        EditIntent::InsertText(String::from("p1\np2")),
        EditIntent::InsertParagraphBreak,
    ] {
        let (edited, _) = run_selection_intent_with_single_undo(
            fenced_doc,
            body_to_closing_start..body_to_closing_end,
            intent.clone(),
        );
        assert!(
            edited.ends_with("\n```\n\nafter"),
            "closing fence & trailing prose must stay intact for {intent:?}: {edited:?}"
        );

        let (opened_edited, _) = run_selection_intent_with_single_undo(
            fenced_doc,
            open_to_body_start..open_to_body_end,
            intent.clone(),
        );
        assert!(
            opened_edited.starts_with("para\n\n```\n"),
            "opening fence must stay intact for {intent:?}: {opened_edited:?}"
        );

        let (marker_only, _) = run_selection_intent_with_single_undo(
            fenced_doc,
            closing_only_start..body_to_closing_end,
            intent,
        );
        assert_eq!(marker_only, fenced_doc, "pure closing fence marker must not be destroyed");
    }

    // 2. Cross-cell table selections: preserve pipes and separator row across Delete, Type, Paste, Enter.
    let table_doc = "| alpha | beta |\n| --- | --- |\n| gamma | delta |";
    let cross_cells = table_doc.find("pha").expect("pha exists")
        ..table_doc.find("beta").expect("beta exists") + 2;
    for intent in [
        EditIntent::DeleteBackward,
        EditIntent::DeleteForward,
        EditIntent::InsertText(String::from("X|Y")),
        EditIntent::InsertText(String::from("p1\np2")),
        EditIntent::InsertParagraphBreak,
    ] {
        let (edited, _) =
            run_selection_intent_with_single_undo(table_doc, cross_cells.clone(), intent.clone());
        let rows: Vec<&str> = edited.lines().collect();
        assert_eq!(rows.len(), 3, "table row count broken for {intent:?}: {edited:?}");
        assert_eq!(rows[1], "| --- | --- |", "separator row broken for {intent:?}: {edited:?}");
        assert!(rows[0].starts_with("| al"), "cell 0 prefix broken: {edited:?}");
        assert!(rows[0].ends_with("ta |"), "cell 1 suffix broken: {edited:?}");
    }

    // 3. Setext heading full-title selection + Enter must remove underline cleanly (no orphan `---` HR).
    let setext_doc = "Setext\n---\n\nafter";
    let (after_enter, caret) = run_selection_intent_with_single_undo(
        setext_doc,
        0.."Setext".len(),
        EditIntent::InsertParagraphBreak,
    );
    assert!(
        !after_enter.contains("---"),
        "Enter over erased Setext heading must not leave orphan HR: {after_enter:?}"
    );
    assert_eq!(after_enter, "\n\nafter");
    assert_eq!(caret, 1);
}
