use super::*;
use ui::plugin::AugmentKind;
use unicode_segmentation::UnicodeSegmentation;

const VIEWPORT_WIDTH: f32 = 800.0;
const VIEWPORT_HEIGHT: f32 = 1600.0;
const GEOMETRY_TOLERANCE: f32 = 0.1;
const FIRST_PARAGRAPH: &str = "前段";
const INSERTED_CHARACTER: &str = "新";

fn apply_edit(source: &str, cursor: usize, kind: AugmentKind) -> (String, usize) {
    let fallback = match &kind {
        AugmentKind::InsertText(text) => (cursor..cursor, text.clone()),
        AugmentKind::Enter | AugmentKind::LineBreak => (cursor..cursor, String::from("\n")),
        AugmentKind::Backspace => {
            let start = source[..cursor]
                .grapheme_indices(true)
                .next_back()
                .map_or(cursor, |(start, _)| start);
            (start..cursor, String::new())
        }
        AugmentKind::Tab => (cursor..cursor, String::from("\t")),
    };
    let Some(augmentation) = crate::augmenter::augment_edit(source, cursor, kind) else {
        let mut updated = source.to_owned();
        let cursor_after = fallback.0.start + fallback.1.len();
        updated.replace_range(fallback.0, &fallback.1);
        return (updated, cursor_after);
    };
    let mut updated = source.to_owned();
    updated.replace_range(
        augmentation.replace_range.unwrap_or(cursor..cursor),
        augmentation.insert_text.as_deref().unwrap_or(""),
    );
    (updated, augmentation.cursor_byte_after)
}

fn render_at(
    view: &mut MarkdownEditorView,
    source: &str,
    cursor: usize,
    generation: u32,
    dpi: f32,
) -> (f32, f32, f32) {
    view.set_source(source.to_owned(), generation);
    view.engine.handle_set_cursor_byte(cursor);
    let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
    let bounds = ui::core::geom::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
    let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
    view.render(&core::document::StringDocView::new(source), bounds, &theme, &mut shaper, dpi);
    let (x, y, _, height) =
        view.engine.cursor_screen_pos().expect("editable empty paragraph must have a caret");
    (x, y + view.engine.scroll_y, height)
}

#[test]
fn empty_paragraph_keeps_its_baseline_when_typing_before_each_block_kind() {
    let followers = [
        "tail",
        "---\n\ntail",
        "---",
        "## tail",
        "> tail",
        "- tail",
        "```rust\nlet value = 1;\n```",
        "| tail |\n| --- |\n| cell |",
    ];
    for dpi in [1.0, 2.0] {
        for follower in followers {
            let source = format!("{FIRST_PARAGRAPH}\n\n{follower}");
            let (empty, cursor) = apply_edit(&source, FIRST_PARAGRAPH.len(), AugmentKind::Enter);
            let (typed, typed_cursor) =
                apply_edit(&empty, cursor, AugmentKind::InsertText(INSERTED_CHARACTER.to_owned()));
            let (deleted, deleted_cursor) =
                apply_edit(&typed, typed_cursor, AugmentKind::Backspace);
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, FIRST_PARAGRAPH.len(), 1, dpi);
            let empty_geometry = render_at(&mut view, &empty, cursor, 2, dpi);
            let typed_geometry = render_at(&mut view, &typed, typed_cursor, 3, dpi);
            let deleted_geometry = render_at(&mut view, &deleted, deleted_cursor, 4, dpi);
            assert_eq!((deleted, deleted_cursor), (empty, cursor));
            assert!(
                (empty_geometry.1 - typed_geometry.1).abs() < GEOMETRY_TOLERANCE,
                "empty/typed baseline changed before {follower:?} at {dpi}x: {empty_geometry:?} -> {typed_geometry:?}"
            );
            assert_eq!(
                empty_geometry, deleted_geometry,
                "deleting text must restore the empty slot"
            );
        }
    }
}

#[test]
fn empty_paragraph_uses_body_typography_outside_each_preceding_block() {
    let predecessors = ["# Title", "```\ncode\n```", "> quote", "- item", "---"];
    for predecessor in predecessors {
        let empty = format!("{predecessor}\n\n\n后段");
        let cursor = predecessor.len() + "\n\n".len();
        let (typed, typed_cursor) =
            apply_edit(&empty, cursor, AugmentKind::InsertText(INSERTED_CHARACTER.to_owned()));
        let mut view = MarkdownEditorView::new();
        let empty_geometry = render_at(&mut view, &empty, cursor, 1, 1.0);
        let typed_geometry = render_at(&mut view, &typed, typed_cursor, 2, 1.0);
        let typed_line = view
            .engine
            .flat_lines()
            .iter()
            .find(|line| line.text == INSERTED_CHARACTER)
            .expect("typing creates a plain paragraph outside the preceding block");
        assert!(
            (empty_geometry.0 - typed_line.rect.x).abs() < GEOMETRY_TOLERANCE
                && (empty_geometry.1 - typed_geometry.1).abs() < GEOMETRY_TOLERANCE
                && (empty_geometry.2 - typed_geometry.2).abs() < GEOMETRY_TOLERANCE,
            "empty paragraph inherited {predecessor:?} typography: {empty_geometry:?} -> {typed_geometry:?}, text x={}",
            typed_line.rect.x
        );
    }
}

#[test]
fn quote_before_extra_empty_line_is_shifted_by_one_paragraph_extent() {
    let mut view = MarkdownEditorView::new();
    render_at(&mut view, "前段\n\n> tail", FIRST_PARAGRAPH.len(), 1, 1.0);
    let baseline_y = view
        .engine
        .flat_lines()
        .iter()
        .find(|line| line.text == "tail")
        .expect("baseline quote exists")
        .rect
        .y;
    render_at(&mut view, "前段\n\n\n> tail", "前段\n\n".len(), 2, 1.0);
    let expanded_y = view
        .engine
        .flat_lines()
        .iter()
        .find(|line| line.text == "tail")
        .expect("expanded quote exists")
        .rect
        .y;
    let paragraph_extent = view.engine.base_line_height + view.engine.paragraph_spacing;
    assert!(
        (expanded_y - baseline_y - paragraph_extent).abs() < GEOMETRY_TOLERANCE,
        "one extra empty paragraph reserved {}px instead of {paragraph_extent}px",
        expanded_y - baseline_y
    );
}

#[test]
fn empty_paragraph_ime_and_hit_testing_share_the_layout_row() {
    const PREEDIT: &str = "拼音";
    let source = "前段\n\n\n---\n\n后段";
    let cursor = "前段\n\n".len();
    for dpi in [1.0, 2.0] {
        let mut view = MarkdownEditorView::new();
        let before = render_at(&mut view, source, cursor, 1, dpi);
        assert_eq!(
            view.engine.hit_test_byte(before.0, before.1 + before.2 * 0.5, 0.0, 0.0),
            Some(cursor),
            "clicking the empty caret row must resolve to its insertion anchor"
        );
        view.engine.set_preedit_text(PREEDIT.to_owned(), Some((PREEDIT.len(), PREEDIT.len())));
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let drawing = view.render(
            &core::document::StringDocView::new(source),
            ui::core::geom::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
            &theme,
            &mut shaper,
            dpi,
        );
        let preedit_draws = drawing.cmds.iter().filter(|command| {
            matches!(command, ui::core::DrawCmd::TextLayout { layout, .. } if layout.text == PREEDIT)
        }).count();
        assert_eq!(preedit_draws, 1, "preedit must be rendered exactly once");
        let (_, preedit_y, _, preedit_height) =
            view.engine.visual_cursor_screen_pos().expect("preedit must have a visual cursor");
        assert!((preedit_y - before.1).abs() < GEOMETRY_TOLERANCE);
        assert_eq!(preedit_height, before.2);
        assert_eq!(view.source, source, "composition must not mutate source");
        view.engine.set_preedit_text(String::new(), None);
        let (typed, typed_cursor) =
            apply_edit(source, cursor, AugmentKind::InsertText(PREEDIT.to_owned()));
        let after = render_at(&mut view, &typed, typed_cursor, 2, dpi);
        assert!((after.1 - before.1).abs() < GEOMETRY_TOLERANCE);
        assert_eq!(after.2, before.2);
    }
}

#[test]
fn typing_in_each_document_edge_empty_paragraph_keeps_all_slots() {
    for source in ["", "\n", "\n\nhead", "head\n", "head\n\n\n", "head\n\n\n\n", "# Heading\n"] {
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, source, source.len(), 1, 1.0);
        let empty_anchors = view
            .engine
            .flat_lines()
            .iter()
            .filter_map(|line| {
                let projection = line.source_projection.as_ref()?;
                matches!(projection.owner, crate::projection::ProjectionOwnerId::EmptyLine { .. })
                    .then_some(projection.source_extent.start)
            })
            .collect::<Vec<_>>();
        for cursor in empty_anchors {
            let mut view = MarkdownEditorView::new();
            let before = render_at(&mut view, source, cursor, 1, 1.0);
            let before_count = view.engine.flat_lines().len();
            let (typed, typed_cursor) =
                apply_edit(source, cursor, AugmentKind::InsertText(INSERTED_CHARACTER.to_owned()));
            let after = render_at(&mut view, &typed, typed_cursor, 2, 1.0);
            assert!(
                (before.1 - after.1).abs() < GEOMETRY_TOLERANCE
                    && (before.2 - after.2).abs() < GEOMETRY_TOLERANCE,
                "edge paragraph moved: {source:?}@{cursor}: {before:?} -> {after:?}, typed={typed:?}"
            );
            assert_eq!(
                view.engine.flat_lines().len(),
                before_count,
                "typing consumed another empty slot: {source:?}@{cursor} -> {typed:?}"
            );
        }
    }
}

#[test]
fn typing_in_container_and_padded_empty_paragraphs_preserves_rows() {
    for newline in ["\n", "\r\n"] {
        for fixture in [
            "- first\n\n\n  second",
            "> first\n>\n>\n> second",
            "> first\n> \n> \n> second",
            "> - first\n>\n>\n>   second",
            "- > first\n  >\n  >\n  > second",
            "head\n \n \ntail",
        ] {
            let source = fixture.replace('\n', newline);
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, source.len(), 1, 1.0);
            let cursor = view
                .engine
                .flat_lines()
                .iter()
                .find_map(|line| {
                    let projection = line.source_projection.as_ref()?;
                    match projection.owner {
                        crate::projection::ProjectionOwnerId::EmptyLine { source_byte } => {
                            Some(source_byte)
                        }
                        _ => None,
                    }
                })
                .expect("fixture must contain a real empty paragraph");
            let before = render_at(&mut view, &source, cursor, 2, 1.0);
            let before_rects =
                view.engine.flat_lines().iter().map(|line| line.rect).collect::<Vec<_>>();
            let (typed, typed_cursor) =
                apply_edit(&source, cursor, AugmentKind::InsertText(INSERTED_CHARACTER.to_owned()));
            let after = render_at(&mut view, &typed, typed_cursor, 3, 1.0);
            let after_rects =
                view.engine.flat_lines().iter().map(|line| line.rect).collect::<Vec<_>>();
            assert_eq!(
                before_rects.len(),
                after_rects.len(),
                "typing consumed a row: {source:?} -> {typed:?}"
            );
            for (before_rect, after_rect) in before_rects.iter().zip(&after_rects) {
                assert_eq!(
                    (before_rect.x, before_rect.y, before_rect.h),
                    (after_rect.x, after_rect.y, after_rect.h),
                    "typing changed a paragraph's container or baseline: {source:?} -> {typed:?}"
                );
            }
            assert_eq!((before.1, before.2), (after.1, after.2));
            let (deleted, deleted_cursor) =
                apply_edit(&typed, typed_cursor, AugmentKind::Backspace);
            let restored = render_at(&mut view, &deleted, deleted_cursor, 4, 1.0);
            assert_eq!(
                (before.1, before.2),
                (restored.1, restored.2),
                "deleting the last character moved the slot: {source:?} -> {typed:?} -> {deleted:?}"
            );
            assert_eq!(view.engine.flat_lines().len(), before_rects.len());
        }
    }
}

#[test]
fn inserting_only_blank_characters_preserves_the_empty_paragraph_count() {
    for source in ["head\n\n\ntail", "> first\n>\n>\n> second", "- first\n\n\n  second"] {
        for text in [" ", "\t"] {
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, source, source.len(), 1, 1.0);
            let cursor = view
                .engine
                .flat_lines()
                .iter()
                .find_map(|line| match line.source_projection.as_ref()?.owner {
                    crate::projection::ProjectionOwnerId::EmptyLine { source_byte } => {
                        Some(source_byte)
                    }
                    _ => None,
                })
                .expect("fixture contains an editable empty paragraph");
            let before = render_at(&mut view, source, cursor, 2, 1.0);
            let before_count = view.engine.flat_lines().len();
            let (typed, typed_cursor) =
                apply_edit(source, cursor, AugmentKind::InsertText(text.to_owned()));
            let after = render_at(&mut view, &typed, typed_cursor, 3, 1.0);
            assert_eq!(
                view.engine.flat_lines().len(),
                before_count,
                "blank input added a paragraph: {source:?} -> {typed:?}"
            );
            assert_eq!((before.1, before.2), (after.1, after.2));
        }
    }
}

#[test]
fn repeated_enter_and_backspace_in_container_empty_paragraphs_restore_rows() {
    for newline in ["\n", "\r\n"] {
        for fixture in [
            "",
            "\n\n",
            ">\n>\n>",
            "head\n",
            "> first\n>",
            "head\n \n \ntail",
            "- first\n\n\n  second",
            "> first\n>\n>\n> second",
        ] {
            let mut source = fixture.replace('\n', newline);
            let mut view = MarkdownEditorView::new();
            render_at(&mut view, &source, source.len(), 1, 1.0);
            let mut cursor = view
                .engine
                .flat_lines()
                .iter()
                .find_map(|line| match line.source_projection.as_ref()?.owner {
                    crate::projection::ProjectionOwnerId::EmptyLine { source_byte } => {
                        Some(source_byte)
                    }
                    _ => None,
                })
                .expect("fixture contains an empty paragraph");
            let mut snapshots = Vec::new();
            for generation in 2..5 {
                let geometry = render_at(&mut view, &source, cursor, generation, 1.0);
                snapshots.push((geometry.1, geometry.2, view.engine.flat_lines().len()));
                if generation < 4 {
                    (source, cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                }
            }
            assert_eq!(snapshots[1].2, snapshots[0].2 + 1, "Enter lost a slot in {fixture:?}");
            assert_eq!(
                snapshots[2].2,
                snapshots[1].2 + 1,
                "second Enter lost a slot in {fixture:?}"
            );
            for (generation, previous) in [snapshots[1], snapshots[0]].into_iter().enumerate() {
                (source, cursor) = apply_edit(&source, cursor, AugmentKind::Backspace);
                let geometry = render_at(&mut view, &source, cursor, generation as u32 + 5, 1.0);
                assert_eq!(
                    (geometry.1, geometry.2, view.engine.flat_lines().len()),
                    previous,
                    "Backspace failed to restore the preceding slot in {fixture:?}: {source:?}"
                );
            }
        }
    }
}

#[test]
fn list_enter_before_blank_separator_keeps_new_item_empty_and_caret_on_it() {
    const FIRST_ITEM: &str = "- 金蝶频繁变化,调整太多";
    const FOLLOWING_PARAGRAPH: &str = "  小红书是异地, 上海的";
    for newline in ["\n", "\r\n"] {
        for separator in ["", " ", "\t"] {
            let source =
                format!("{FIRST_ITEM}{newline}{separator}{newline}{FOLLOWING_PARAGRAPH}{newline}");
            let (entered, cursor) = apply_edit(&source, FIRST_ITEM.len(), AugmentKind::Enter);
            let expected = format!(
                "{FIRST_ITEM}{newline}- {newline}{separator}{newline}{FOLLOWING_PARAGRAPH}{newline}"
            );
            assert_eq!(entered, expected, "Enter must preserve the existing paragraph separator");
            assert_eq!(cursor, FIRST_ITEM.len() + newline.len() + "- ".len());

            let parsed = crate::parser::parse_markdown(&entered);
            let document = crate::builder::MarkdownDoc::build_structure(&parsed);
            assert!(matches!(document.blocks[1].kind, crate::builder::BlockKind::ListItem { .. }));
            assert_eq!(document.blocks[1].text_lines, [""]);
            assert!(matches!(document.blocks[2].kind, crate::builder::BlockKind::Paragraph));
            assert_eq!(document.blocks[2].text_lines, [FOLLOWING_PARAGRAPH.trim_start()]);

            for dpi in [1.0, 2.0] {
                let mut view = MarkdownEditorView::new();
                let caret = render_at(&mut view, &entered, cursor, 1, dpi);
                let item_row = &view.engine.flat_lines()[1];
                assert!(
                    caret.1 >= item_row.rect.y && caret.1 + caret.2 <= item_row.rect.bottom(),
                    "new item caret must stay on its own row: {caret:?}, {:?}",
                    item_row.rect
                );
                assert_eq!(
                    view.engine.hit_test_byte(caret.0, caret.1 + caret.2 * 0.5, 0.0, 0.0),
                    Some(cursor)
                );
            }
        }
    }
}

#[path = "view_paragraph_semantics_tests.rs"]
mod paragraph_semantics;
