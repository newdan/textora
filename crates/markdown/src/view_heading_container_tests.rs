use super::*;
use crate::builder::{BlockKind, BlockNode, EditableParagraphMap, MarkdownDoc};

fn heading_owner_path(blocks: &[BlockNode]) -> Option<Vec<BlockKind>> {
    for block in blocks {
        if matches!(block.kind, BlockKind::Heading { .. }) {
            return Some(Vec::new());
        }
        if let Some(mut ancestors) = heading_owner_path(&block.children) {
            ancestors.insert(0, block.kind.clone());
            return Some(ancestors);
        }
    }
    None
}

#[test]
fn creation_existing_empty_list_start_keeps_heading_owner_on_repeated_enter() {
    for newline in ["\n", "\r\n"] {
        for initial in [
            "- \n  # Title",
            "> - \n>   # Title",
            "- > \n  > # Title",
            "12. \n    # Title",
            "- [ ]\n  # Title",
            "- - \n    # Title",
        ] {
            let original_source = initial.replace('\n', newline);
            let mut source = original_source.clone();
            let original = MarkdownDoc::build_structure(&crate::parser::parse_markdown(&source));
            let original_owner = heading_owner_path(&original.blocks);
            assert!(original_owner.is_some(), "fixture must contain a parsed heading");
            let mut cursor = source.find("Title").expect("fixture contains heading");
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, cursor, 1, 1.0);
            let original_rows = rendered_row_count(&view);
            for enter_count in 1..=3 {
                (source, cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                let updated = MarkdownDoc::build_structure(&crate::parser::parse_markdown(&source));
                assert_eq!(
                    heading_owner_path(&updated.blocks),
                    original_owner,
                    "{original_source:?} -> {source:?}"
                );
                render_at(&mut view, &source, cursor, enter_count + 1, 1.0);
                assert_eq!(
                    rendered_row_count(&view),
                    original_rows + enter_count as usize,
                    "{source:?}"
                );
                assert!(
                    source.contains(&original_source),
                    "original list syntax must stay intact: {source:?}"
                );
            }
        }
    }
}

#[test]
fn heading_container_enter_creates_one_row_and_keeps_ownership() {
    let mut violations = Vec::new();
    for newline in ["\n", "\r\n"] {
        for heading in ["> # Title", "- # Title", "> - # Title", "- > # Title"] {
            let source = format!("{heading}{newline}{newline}tail");
            let cursor = source.find("Title").expect("fixture contains heading");
            let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
            let original = MarkdownDoc::build_structure(&crate::parser::parse_markdown(&source));
            let updated = MarkdownDoc::build_structure(&crate::parser::parse_markdown(&entered));
            assert_eq!(
                heading_owner_path(&updated.blocks),
                heading_owner_path(&original.blocks),
                "{source:?} -> {entered:?}"
            );
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, cursor, 1, 1.0);
            let original_rows = rendered_row_count(&view);
            let entered_geometry = render_at(&mut view, &entered, entered_cursor, 2, 1.0);
            let entered_rows = rendered_row_count(&view);
            if entered_rows != original_rows + 1 {
                let paragraphs = EditableParagraphMap::from_blocks(&updated.blocks, &entered);
                violations.push(format!("{source:?} -> {entered:?}: rows {original_rows} -> {entered_rows}; cursor {entered_cursor}, geometry {entered_geometry:?}; blocks {:#?}; map {paragraphs:#?}", updated.blocks));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "container heading Enter must add exactly one row: {violations:#?}"
    );
}

#[test]
fn heading_container_repeated_enter_adds_one_row_each_time() {
    for newline in ["\n", "\r\n"] {
        for heading in ["> # Title", "- # Title", "> - # Title", "- > # Title"] {
            let mut source = format!("{heading}{newline}{newline}tail");
            let mut cursor = source.find("Title").expect("fixture contains heading");
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, cursor, 1, 1.0);
            let original_rows = rendered_row_count(&view);
            for enter_count in 1..=3 {
                (source, cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                render_at(&mut view, &source, cursor, enter_count + 1, 1.0);
                assert_eq!(
                    rendered_row_count(&view),
                    original_rows + enter_count as usize,
                    "{source:?}: {:#?}",
                    MarkdownDoc::build_structure(&crate::parser::parse_markdown(&source))
                );
            }
        }
    }
}

#[test]
fn heading_container_new_empty_first_line_keeps_baseline_when_typing() {
    for newline in ["\n", "\r\n"] {
        for heading in ["> # Title", "- # Title", "> - # Title", "- > # Title"] {
            let source = format!("{heading}{newline}{newline}tail");
            let cursor = source.find("Title").expect("fixture contains heading");
            let (entered, _) = apply_edit(&source, cursor, AugmentKind::Enter);
            let first_line_end =
                entered.find(newline).expect("entered source has empty first line");
            let mut view = MarkdownEditorView::new();
            let before = render_at(&mut view, &entered, first_line_end, 1, 1.0);
            let before_rows = rendered_row_count(&view);
            let (typed, typed_cursor) =
                apply_edit(&entered, first_line_end, AugmentKind::InsertText("new".to_owned()));
            let after = render_at(&mut view, &typed, typed_cursor, 2, 1.0);
            assert_eq!(rendered_row_count(&view), before_rows, "{entered:?} -> {typed:?}");
            assert!(
                (before.1 - after.1).abs() < GEOMETRY_TOLERANCE,
                "{entered:?} -> {typed:?}: {before:?} -> {after:?}"
            );
            assert!(
                (before.2 - after.2).abs() < GEOMETRY_TOLERANCE,
                "{entered:?} -> {typed:?}: {before:?} -> {after:?}"
            );
        }
    }
}

#[test]
fn heading_container_enter_backspace_restores_heading_source() {
    for newline in ["\n", "\r\n"] {
        for heading in ["> # Title", "- # Title", "> - # Title", "- > # Title"] {
            let source = format!("{heading}{newline}{newline}tail");
            let cursor = source.find("Title").expect("fixture contains heading");
            let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
            let (restored, restored_cursor) =
                apply_edit(&entered, entered_cursor, AugmentKind::Backspace);
            assert_eq!(restored, source, "{entered:?}");
            assert_eq!(restored_cursor, cursor);
        }
    }
}

#[test]
fn heading_container_map_owns_only_unprojected_empty_list_first_lines() {
    for (source, anchor, owner) in [
        ("- \n  # Title", 2, vec![0]),
        ("> - \n>   # Title", 4, vec![0, 0]),
        ("- > \n  > # Title", 4, vec![0, 0]),
        ("12. \n    # Title", 4, vec![0]),
    ] {
        let document = MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
        let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
        let run = paragraphs.run_at_byte(anchor).expect("empty list first line must be editable");
        assert_eq!(run.owner_path, owner, "{source:?}");
        assert_eq!(run.hidden_separator_count, 0);
        assert_eq!(run.lines[0].source_byte, anchor);
    }
    for source in [
        "- ",
        "> - ",
        "- > ",
        "- # Title",
        "- > # Title",
        "- ```\n  code\n  ```",
        "- ---",
        "123. para\n     ***",
        "> 123. para\n>      ***",
        "123. para\r\n     ***",
    ] {
        let document = MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
        let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
        let first_line_end = source.find('\n').unwrap_or(source.len());
        assert!(
            paragraphs.run_at_byte(first_line_end).is_none(),
            "existing block content must not gain a duplicate first line: {source:?}"
        );
    }
}

#[test]
fn creation_heading_after_list_content_stays_inside_its_item() {
    for source in ["- para\n\n  # Title", "- [x] done\n\n  # Title", "123. para\n\n     # Title"] {
        let cursor = source.find("Title").expect("fixture contains heading");
        let (entered, entered_cursor) = apply_edit(source, cursor, AugmentKind::Enter);
        assert!(entered.starts_with(source.lines().next().expect("fixture first line")));
        let original = MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
        let updated = MarkdownDoc::build_structure(&crate::parser::parse_markdown(&entered));
        assert_eq!(
            heading_owner_path(&updated.blocks),
            heading_owner_path(&original.blocks),
            "{entered:?}"
        );
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, source, cursor, 1, 1.0);
        let original_rows = rendered_row_count(&view);
        render_at(&mut view, &entered, entered_cursor, 2, 1.0);
        assert_eq!(rendered_row_count(&view), original_rows + 1, "{entered:?}");
    }
}
