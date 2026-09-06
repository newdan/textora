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
                    let start = source[..cursor]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(cursor, |(start, _)| start);
                    start..cursor
                }
                ui::plugin::EditIntent::DeleteForward => {
                    cursor..cursor + source[cursor..].graphemes(true).next().map_or(0, str::len)
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
