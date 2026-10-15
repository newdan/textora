extern crate core as doc_core;
#[path = "support/editing_audit.rs"]
mod editing_audit;
use doc_core::document::StringDocView;
use editing_audit::*;
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::{EditIntent, PluginMessage, ViewPlugin};
use unicode_segmentation::UnicodeSegmentation;

#[test]
fn end_enter_and_backspace_matrix() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for before in
            [PARAGRAPH, "**前段**", RULE, "***", "___", "- - -", "---   ", CODE, "~~~~\ncode\n~~~~"]
        {
            let before = before.replace('\n', newline);
            for suffix in [
                "",
                "\n",
                "\n\n",
                "\n\n后段",
                "\n\n---",
                "\n\n```\ncode\n```",
                "\n \n后段",
                "\n\t\n后段",
            ] {
                let initial = format!("{before}{}", suffix.replace('\n', newline));
                let mut states = vec![(initial.clone(), before.len())];
                for _ in 0..REPEAT_COUNT {
                    let (source, cursor) = states.last().expect("initial state exists");
                    let next = apply(source, *cursor, EditIntent::InsertParagraphBreak);
                    checked += 1;
                    let previous_count = empty_anchors(source).len();
                    let next_count = empty_anchors(&next.0).len();
                    if next_count != previous_count + 1
                        || atomic_signature(source) != atomic_signature(&next.0)
                        || content_signature(source) != content_signature(&next.0)
                    {
                        violations.push(format!("Enter {source:?}@{cursor} -> {:?}@{}: slots {previous_count}->{next_count}", next.0, next.1));
                    }
                    states.push(next);
                }
                let (mut source, mut cursor) = states.last().expect("entered state exists").clone();
                for index in (0..REPEAT_COUNT).rev() {
                    let previous = apply(&source, cursor, EditIntent::DeleteBackward);
                    checked += 1;
                    let expected = &states[index];
                    if previous.1 != expected.1
                        || empty_anchors(&previous.0).len() != empty_anchors(&expected.0).len()
                        || atomic_signature(&previous.0) != atomic_signature(&expected.0)
                        || content_signature(&previous.0) != content_signature(&expected.0)
                    {
                        violations.push(format!(
                            "Backspace {source:?}@{cursor} -> {previous:?}, expected {expected:?}"
                        ));
                    }
                    (source, cursor) = previous;
                }
            }
        }
    }
    println!("checked {checked} end Enter/Backspace edits");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn atomic_neighbors_remain_atomic_on_boundary_deletion() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for before in [PARAGRAPH, RULE, CODE] {
            for after in [NEXT_PARAGRAPH, RULE, CODE] {
                let before = before.replace('\n', newline);
                let after = after.replace('\n', newline);
                for newline_count in 1..=5 {
                    let source = format!("{before}{}{after}", newline.repeat(newline_count));
                    let signature = atomic_signature(&source);
                    for (cursor, intent) in [
                        (before.len(), EditIntent::DeleteForward),
                        (before.len() + newline.len() * newline_count, EditIntent::DeleteBackward),
                    ] {
                        let (edited, after_cursor) = apply(&source, cursor, intent.clone());
                        checked += 1;
                        if atomic_signature(&edited) != signature {
                            violations.push(format!(
                                "{intent:?}: {source:?}@{cursor} -> {edited:?}@{after_cursor}"
                            ));
                        }
                    }
                }
            }
        }
    }
    println!("checked {checked} neighbor deletions");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn empty_slot_typing_and_erasure_matrix() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for before in ["", PARAGRAPH, RULE, CODE] {
            for after in ["", NEXT_PARAGRAPH, RULE, CODE] {
                for padding in ["", " ", "\t"] {
                    for count in 1..=4 {
                        let before = before.replace('\n', newline);
                        let after = after.replace('\n', newline);
                        let source = format!(
                            "{before}{}{after}",
                            format!("{newline}{padding}").repeat(count)
                        );
                        for cursor in empty_anchors(&source) {
                            let (typed, typed_cursor) =
                                apply(&source, cursor, EditIntent::InsertText("新".to_owned()));
                            let (cleared, cleared_cursor) =
                                apply(&typed, typed_cursor, EditIntent::DeleteBackward);
                            checked += 1;
                            if blank_paragraph_count(&cleared) != blank_paragraph_count(&source)
                                || atomic_signature(&cleared) != atomic_signature(&source)
                                || content_signature(&cleared) != content_signature(&source)
                                || !caret_is_on_whitespace_only_source_line(
                                    &cleared,
                                    cleared_cursor,
                                )
                            {
                                violations.push(format!("{source:?}@{cursor} -> {typed:?} -> {cleared:?}@{cleared_cursor}"));
                            }
                        }
                    }
                }
            }
        }
    }
    println!("checked {checked} empty-slot typing/erasure cycles");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn paragraph_interior_split_roundtrips() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for paragraph in ["前后", "first second", "**前后**", "a\nb", "a\\\nb", "a  \nb"] {
            for (prefix, suffix) in
                [("", ""), ("---\n\n", "\n\n---"), ("```\ncode\n```\n\n", "\n\n```\ncode\n```")]
            {
                let prefix = prefix.replace('\n', newline);
                let paragraph = paragraph.replace('\n', newline);
                let source = format!("{prefix}{paragraph}{}", suffix.replace('\n', newline));
                for (offset, character) in paragraph.char_indices() {
                    if character.is_whitespace() || "*\\".contains(character) || offset == 0 {
                        continue;
                    }
                    let cursor = prefix.len() + offset;
                    let mut session = EditingSession::new(&source, cursor);
                    let (entered, entered_cursor) = session.apply(EditIntent::InsertParagraphBreak);
                    let restored = session.apply(EditIntent::DeleteBackward);
                    checked += 1;
                    if restored.0 != source {
                        violations.push(format!(
                            "{source:?}@{cursor} -> {entered:?}@{entered_cursor} -> {restored:?}"
                        ));
                    }
                }
            }
        }
    }
    println!("checked {checked} paragraph splits");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn document_start_enter_preserves_blocks_and_roundtrips() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for block in [PARAGRAPH, RULE, "# 标题", "***", "    code"] {
            for prefix in ["", "\n", "\n\n"] {
                let prefix = prefix.replace('\n', newline);
                let block = block.replace('\n', newline);
                let source = format!("{prefix}{block}");
                let cursor = prefix.len() + if block.starts_with("# ") { "# ".len() } else { 0 };
                let (entered, entered_cursor) =
                    apply(&source, cursor, EditIntent::InsertParagraphBreak);
                let restored = apply(&entered, entered_cursor, EditIntent::DeleteBackward);
                checked += 1;
                if empty_anchors(&entered).len() != empty_anchors(&source).len() + 1
                    || atomic_signature(&entered) != atomic_signature(&source)
                    || restored != (source.clone(), cursor)
                {
                    violations.push(format!(
                        "{source:?}@{cursor} -> {entered:?}@{entered_cursor} -> {restored:?}"
                    ));
                }
            }
        }
    }
    println!("checked {checked} block-start edits");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn empty_slot_caret_geometry_is_stable_across_neighbors() {
    let mut violations = Vec::new();
    let mut checked = 0;
    let mut shaper = shaping::Shaper::new().expect("audit shaper must initialize");
    let theme = ui::theme::test_theme();
    for dpi in [1.0, 2.0] {
        for before in [PARAGRAPH, RULE, CODE] {
            for after in [NEXT_PARAGRAPH, RULE, CODE] {
                for newline in ["\n", "\r\n"] {
                    let before = before.replace('\n', newline);
                    let after = after.replace('\n', newline);
                    let source = format!("{before}{}{after}", newline.repeat(3));
                    for cursor in empty_anchors(&source) {
                        let mut view = MarkdownEditorView::new();
                        let mut previous_geometry: Option<(f32, f32)> = None;
                        let (typed, typed_cursor) =
                            apply(&source, cursor, EditIntent::InsertText("新".to_owned()));
                        let (cleared, cleared_cursor) =
                            apply(&typed, typed_cursor, EditIntent::DeleteBackward);
                        for (generation, (edited, edited_cursor)) in [
                            (source.clone(), cursor),
                            (typed, typed_cursor),
                            (cleared, cleared_cursor),
                        ]
                        .iter()
                        .enumerate()
                        {
                            view.set_source(edited.clone(), generation as u32 + 1);
                            let mut document = AuditDocument(edited.clone());
                            view.handle_message(
                                PluginMessage::SetCursorByte(*edited_cursor),
                                &mut document,
                            );
                            view.render(
                                &StringDocView::new(edited),
                                ui::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                                &theme,
                                &mut shaper,
                                dpi,
                            );
                            let current = view.engine().cursor_screen_pos();
                            if let Some((_, y, _, height)) = current {
                                if let Some((previous_y, previous_height)) = previous_geometry
                                    && ((y - previous_y).abs() > GEOMETRY_TOLERANCE
                                        || (height - previous_height).abs() > GEOMETRY_TOLERANCE)
                                {
                                    violations.push(format!(
                                        "{source:?}@{cursor}, dpi={dpi}: {:?} -> {current:?}",
                                        previous_geometry
                                    ));
                                }
                                previous_geometry = Some((y, height));
                            } else {
                                violations.push(format!(
                                    "missing caret: {edited:?}@{edited_cursor}, dpi={dpi}"
                                ));
                            }
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    println!("checked {checked} caret geometry cycles");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn code_body_backspace_only_edits_literal_text() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for line in ["# value", "- value", "> value", "1. value", "a\\", "a  "] {
            let source = format!("```{newline}{line}{newline}next{newline}```");
            let line_start = "```".len() + newline.len();
            let cursor = if line.ends_with(['\\', ' ']) {
                line_start + line.len() + newline.len()
            } else {
                line_start + line.find("value").expect("marker fixture has content")
            };
            let start = source[..cursor]
                .grapheme_indices(true)
                .next_back()
                .expect("cursor follows a grapheme")
                .0;
            let mut expected = source.clone();
            expected.replace_range(start..cursor, "");
            let edited = apply(&source, cursor, EditIntent::DeleteBackward);
            checked += 1;
            if edited.0 != expected {
                violations
                    .push(format!("{source:?}@{cursor} -> {edited:?}, expected {expected:?}"));
            }
        }
        let source = format!("```{newline}first{newline}# next{newline}```");
        let cursor = source.find("# next").expect("fixture has marker-like code");
        let edited = apply(&source, cursor, EditIntent::DeleteBackward);
        if edited.0 == source {
            violations.push(format!("ordinary code line join blocked: {source:?}@{cursor}"));
        }
        checked += 1;
    }
    println!("checked {checked} literal code deletions");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn nested_atomic_block_end_roundtrips_source() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for source in [
            "> ---",
            "- ---",
            "> ```\n> code\n> ```",
            "- ```\n  code\n  ```",
            "> - ```\n>   code\n>   ```",
        ] {
            let source = source.replace('\n', newline);
            let (entered, entered_cursor) =
                apply(&source, source.len(), EditIntent::InsertParagraphBreak);
            checked += 1;
            if atomic_signature(&entered) != atomic_signature(&source) {
                violations.push(format!("{source:?} -> {entered:?}@{entered_cursor}"));
            }
            let restored = apply(&entered, entered_cursor, EditIntent::DeleteBackward);
            if restored.0 != source {
                violations.push(format!(
                    "roundtrip: {source:?} -> {entered:?}@{entered_cursor} -> {restored:?}"
                ));
            }
        }
    }
    println!("checked {checked} nested atomic end edits");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn adjacent_empty_paragraph_deletion_reduces_one_slot() {
    let mut violations = Vec::new();
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for before in [PARAGRAPH, RULE, CODE, "# 标题", "> 引用", "- 条目"] {
            for after in [NEXT_PARAGRAPH, RULE, CODE, "# 后标题", "> 后引用", "- 后条目"] {
                for padding in ["", " ", "\t"] {
                    let before = before.replace('\n', newline);
                    let after = after.replace('\n', newline);
                    let source = format!(
                        "{before}{newline}{padding}{newline}{padding}{newline}{padding}{newline}{after}"
                    );
                    let previous_count = empty_anchors(&source).len();
                    if previous_count == 0 {
                        continue;
                    }
                    for (cursor, intent) in [
                        (before.len(), EditIntent::DeleteForward),
                        (
                            source.len() - after.len()
                                + if after.starts_with(['#', '>', '-']) && after != RULE {
                                    after
                                        .find(|character: char| character.is_alphanumeric())
                                        .expect("text blocks have visible content")
                                } else {
                                    0
                                },
                            EditIntent::DeleteBackward,
                        ),
                    ] {
                        let (edited, edited_cursor) = apply(&source, cursor, intent.clone());
                        checked += 1;
                        let count_after = empty_anchors(&edited).len();
                        if count_after + 1 != previous_count
                            || atomic_signature(&edited) != atomic_signature(&source)
                            || content_signature(&edited) != content_signature(&source)
                        {
                            violations.push(format!("{intent:?}: {source:?}@{cursor} -> {edited:?}@{edited_cursor}, slots {previous_count}->{count_after}"));
                        }
                    }
                }
            }
        }
    }
    println!("checked {checked} adjacent empty-paragraph deletions");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn nested_closing_fences_use_fence_enter_context() {
    use textora_markdown::augmenter::{EnterContext, classify_enter_context};
    let mut violations = Vec::new();
    for source in ["> ```\n> code\n> ```", "- ```\n  code\n  ```", "> - ```\n>   code\n>   ```"] {
        let context = classify_enter_context(source, source.len());
        let edited = apply(source, source.len(), EditIntent::InsertParagraphBreak);
        if !matches!(context, EnterContext::CodeBlockFenceLine { is_opening: false, .. }) {
            violations.push(format!("{source:?}: {context:?}; Enter -> {edited:?}"));
        }
    }
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn split_preserves_separators_at_each_physical_line_end() {
    let mut violations = Vec::new();
    for source in [
        "hello\nworld",
        "hello\\\nworld",
        "hello  \nworld",
        "hello\r\nworld",
        "hello\\\r\nworld",
        "hello  \r\nworld",
    ] {
        let cursor = source.find(['\\', ' ', '\r', '\n']).expect("fixture has a line separator");
        let mut session = EditingSession::new(source, cursor);
        let (entered, after_cursor) = session.apply(EditIntent::InsertParagraphBreak);
        let restored = session.apply(EditIntent::DeleteBackward);
        if restored.0 != source {
            violations
                .push(format!("{source:?}@{cursor} -> {entered:?}@{after_cursor} -> {restored:?}"));
        }
    }
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn fenced_code_structural_newlines_survive_backspace() {
    let mut violations = Vec::new();
    for newline in ["\n", "\r\n"] {
        for fence in ["```", "~~~~"] {
            let opening = format!("{fence}rust{newline}");
            let body = format!("{opening}code{newline}");
            let source = format!("{body}{fence}{newline}{newline}after");
            for cursor in [opening.len(), body.len()] {
                let edited = apply(&source, cursor, EditIntent::DeleteBackward);
                if edited.0 != source {
                    violations.push(format!("{source:?}@{cursor} -> {edited:?}"));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn persistent_splits_survive_typing_erasure_and_multiple_enters() {
    for newline in ["\n", "\r\n"] {
        for source in ["a\nb", "a\\\nb", "a  \nb", "中👨‍👩‍👧e\u{301}后"] {
            let source = source.replace('\n', newline);
            let cursor = source.find('b').unwrap_or("中".len());
            for typed in ["中文", "👩🏽‍💻", "e\u{301}"] {
                let mut session = EditingSession::new(&source, cursor);
                for _ in 0..3 {
                    session.apply(EditIntent::InsertParagraphBreak);
                }
                session.apply(EditIntent::InsertText(typed.to_owned()));
                for _ in typed.graphemes(true) {
                    session.apply(EditIntent::DeleteBackward);
                }
                for _ in 0..3 {
                    session.apply(EditIntent::DeleteBackward);
                }
                assert_eq!((session.source, session.cursor), (source.clone(), cursor));
            }
        }
    }
}

#[test]
fn uncommitted_split_and_external_boundary_changes_do_not_restore_stale_source() {
    use ui::plugin::{EditPolicy, EditRequest};
    let mut session = EditingSession::new("a\nb", 2);
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: 2,
        selection: None,
        intent: EditIntent::InsertParagraphBreak,
    };
    session.view.plan_edit(&request);
    session.view.set_source("a\n\nb changed".to_owned(), 2);
    session.source = "a\n\nb changed".to_owned();
    session.cursor = 3;
    session.generation = 2;
    assert_eq!(session.apply(EditIntent::DeleteBackward).0, "ab changed");
    let mut session = EditingSession::new("a\nb", 2);
    session.apply(EditIntent::InsertParagraphBreak);
    session.view.set_source("x\nb".to_owned(), 3);
    session.source = "x\nb".to_owned();
    session.cursor = 2;
    session.generation = 3;
    assert_eq!(session.apply(EditIntent::DeleteBackward).0, "xb");
}

#[test]
fn selection_deletion_and_paste_replace_exact_grapheme_ranges() {
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for before in [PARAGRAPH, RULE, CODE] {
            for after in [NEXT_PARAGRAPH, RULE, CODE] {
                let source = format!(
                    "{}{}{}",
                    before.replace('\n', newline),
                    newline.repeat(3),
                    after.replace('\n', newline)
                );
                for range in [
                    0..source.len(),
                    0..before.replace('\n', newline).len(),
                    before.replace('\n', newline).len()..source.len(),
                ] {
                    for intent in [
                        EditIntent::DeleteBackward,
                        EditIntent::DeleteForward,
                        EditIntent::InsertText("中文👩🏽‍💻".into()),
                        EditIntent::InsertText("段落\n\n---\n\n```rust\ncode\n```".into()),
                    ] {
                        let inserted = match &intent {
                            EditIntent::InsertText(text) => text.as_str(),
                            _ => "",
                        };
                        let mut expected = source.clone();
                        expected.replace_range(range.clone(), inserted);
                        // Clearing a complete paragraph retains its visible empty slot;
                        // its former hidden separator is no longer owned by that paragraph.
                        if before == PARAGRAPH && range == (0..before.len()) && inserted.is_empty()
                        {
                            expected =
                                format!("{}{}", newline.repeat(2), after.replace('\n', newline));
                        }
                        for cursor in [range.start, range.end] {
                            let mut session = EditingSession::new(&source, cursor);
                            session.selection = Some(range.clone());
                            let actual = session.apply(intent.clone());
                            assert_eq!(actual.0, expected, "{source:?}, {range:?}, {intent:?}");
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    println!("checked {checked} forward/backward selection replacements");
}

#[test]
fn line_break_and_literal_code_variants_use_exact_source_semantics() {
    for newline in ["\n", "\r\n"] {
        let mut paragraph = EditingSession::new("前后", "前".len());
        if newline == "\r\n" {
            paragraph = EditingSession::new("前后\r\n", "前".len());
        }
        let expected = if newline == "\n" { "前\\\n后" } else { "前\\\r\n后\r\n" };
        assert_eq!(paragraph.apply(EditIntent::InsertLineBreak).0, expected);
        for fence in ["```", "~~~~", "`````"] {
            for body in ["# value", "- value", "> value", "a\\", "a  ", "👩🏽‍💻中", "```pseudo"]
            {
                for closing in [true, false] {
                    let source = format!(
                        "{fence}rust{newline}{body}{}",
                        if closing { format!("{newline}{fence}") } else { String::new() }
                    );
                    let cursor = fence.len() + "rust".len() + newline.len() + body.len();
                    for intent in [EditIntent::InsertParagraphBreak, EditIntent::InsertLineBreak] {
                        let mut session = EditingSession::new(&source, cursor);
                        let mut expected = source.clone();
                        expected.insert_str(cursor, newline);
                        assert_eq!(
                            session.apply(intent).0,
                            expected,
                            "literal newline in {source:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn resized_layout_hit_testing_and_ime_cancellation_restore_exact_caret_geometry() {
    use ui::plugin::{PluginQuery, PluginResponse};
    let mut shaper = shaping::Shaper::new().expect("geometry audit requires font shaping");
    let mut checked = 0;
    for dpi in [1.0, 1.5, 2.0] {
        for width in [240.0, 800.0] {
            for font_size in [12.0, 22.0] {
                for theme in [ui::theme::test_theme(), ui::theme::test_light_theme()] {
                    for newline in ["\n", "\r\n"] {
                        let source = format!(
                            "{}{}---{}~~~rust{}code{}~~~",
                            "长段落中文 mixed 👩🏽‍💻 e\u{301} ".repeat(6),
                            newline.repeat(3),
                            newline.repeat(3),
                            newline,
                            newline
                        );
                        let cursor = empty_anchors(&source)[0];
                        let mut session = EditingSession::new(&source, cursor);
                        let mut document = AuditDocument(source.clone());
                        session.view.handle_message(
                            PluginMessage::SetRenderSettings {
                                font_family: ui::settings::platform_default_font_family().into(),
                                font_size,
                                line_height: font_size * 1.5,
                                toc_max_depth: 3,
                                markdown_first_line_indent: false,
                            },
                            &mut document,
                        );
                        session
                            .view
                            .handle_message(PluginMessage::SetCursorByte(cursor), &mut document);
                        let bounds = ui::Rect::new(0.0, 0.0, width, VIEWPORT_HEIGHT);
                        session.view.render(&document, bounds, &theme, &mut shaper, dpi);
                        let original = session
                            .view
                            .engine()
                            .cursor_screen_pos()
                            .expect("empty slot has a visible caret");
                        let hit = session.view.query(
                            PluginQuery::HitTestEditTarget {
                                x: original.0,
                                y: original.1 + original.3 * 0.5,
                                offset_x: 0.0,
                                offset_y: 0.0,
                            },
                            &document,
                        );
                        assert!(
                            matches!(hit, PluginResponse::EditHitTarget(Some(ui::plugin::EditHitTarget::TextCaret { byte_offset, .. })) if byte_offset == cursor),
                            "mouse hit must resolve the empty slot: {hit:?}"
                        );
                        for preedit in ["拼音", "👩🏽‍💻", "e\u{301}"] {
                            session.view.handle_message(
                                PluginMessage::SetPreedit {
                                    text: preedit.into(),
                                    cursor: Some((preedit.len(), preedit.len())),
                                },
                                &mut document,
                            );
                            session.view.render(&document, bounds, &theme, &mut shaper, dpi);
                            assert_eq!(session.source, source, "composition must not edit source");
                            session.view.handle_message(
                                PluginMessage::SetPreedit { text: String::new(), cursor: None },
                                &mut document,
                            );
                            session.view.render(&document, bounds, &theme, &mut shaper, dpi);
                            assert_eq!(
                                session.view.engine().cursor_screen_pos(),
                                Some(original),
                                "IME cancellation restores X/Y/width/height"
                            );
                        }
                        session.apply(EditIntent::InsertText("中文👩🏽‍💻e\u{301}".into()));
                        for _ in "中文👩🏽‍💻e\u{301}".graphemes(true) {
                            session.apply(EditIntent::DeleteBackward);
                        }
                        document.0 = session.source.clone();
                        session.view.handle_message(
                            PluginMessage::SetCursorByte(session.cursor),
                            &mut document,
                        );
                        session.view.render(&document, bounds, &theme, &mut shaper, dpi);
                        assert_eq!(
                            session.view.engine().cursor_screen_pos(),
                            Some(original),
                            "typing and erasure restore full caret geometry"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    println!("checked {checked} width/font/theme/DPI/IME combinations");
}

#[test]
fn three_block_permutations_delete_slots_without_changing_atomic_content() {
    let mut checked = 0;
    for newline in ["\n", "\r\n"] {
        for first in ["前段", "***", "~~~~rust\n# literal\n~~~~"] {
            for second in ["中段", "___", "`````python\n- literal\n`````"] {
                for third in ["后段", "- - -", CODE] {
                    for padding in ["", " ", "\t"] {
                        let first = first.replace('\n', newline);
                        let separator = format!("{newline}{padding}{newline}{padding}{newline}");
                        let source = format!(
                            "{first}{separator}{}{separator}{}",
                            second.replace('\n', newline),
                            third.replace('\n', newline)
                        );
                        let mut session = EditingSession::new(&source, first.len());
                        let expected_atoms = atomic_signature(&source);
                        for _ in 0..4 {
                            let previous = session.source.clone();
                            session.apply(EditIntent::DeleteForward);
                            assert_eq!(
                                atomic_signature(&session.source),
                                expected_atoms,
                                "{previous:?} -> {:?}@{}",
                                session.source,
                                session.cursor
                            );
                            for literal in ["# literal", "- literal", "code"] {
                                assert_eq!(
                                    session.source.matches(literal).count(),
                                    source.matches(literal).count()
                                );
                            }
                            if previous == session.source {
                                break;
                            }
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    println!("checked {checked} continuous three-block deletion sequences");
}

#[test]
fn fixed_seed_long_document_edits_preserve_unrelated_blocks() {
    const RANDOM_SEED: u64 = 0x5445_5854_4f52_4101;
    const EDIT_STEPS: usize = 64;
    let source = "段落\n\n\n---\n\n\n```rust\ncode\n```\n\n\n".repeat(100);
    let expected_atoms = atomic_signature(&source);
    let mut session = EditingSession::new(&source, 0);
    let mut random_state = RANDOM_SEED;
    for _ in 0..EDIT_STEPS {
        random_state ^= random_state << 13;
        random_state ^= random_state >> 7;
        random_state ^= random_state << 17;
        let anchors = empty_anchors(&session.source);
        session.cursor = anchors[random_state as usize % anchors.len()];
        let before = session.source.clone();
        session.apply(EditIntent::InsertText("👩🏽‍💻".into()));
        session.apply(EditIntent::DeleteBackward);
        assert_eq!(
            atomic_signature(&session.source),
            expected_atoms,
            "seed={RANDOM_SEED}, source before={before:?}"
        );
        assert_eq!(session.source.matches("段落").count(), 100);
        assert_eq!(session.source.matches("code").count(), 100);
    }
}

#[test]
fn literal_break_markers_do_not_disable_closing_fence_protection() {
    for newline in ["\n", "\r\n"] {
        for body in ["a\\", "a  ", "<br>", "# body"] {
            let source = format!("```rust{newline}{body}{newline}```{newline}after");
            let cursor = "```rust".len() + newline.len() + body.len();
            assert_eq!(apply(&source, cursor, EditIntent::DeleteForward).0, source);
        }
    }
}

#[test]
fn container_code_first_line_backspace_preserves_opening_fence() {
    for newline in ["\n", "\r\n"] {
        for (source, prefix) in [
            ("> ```rust\n> code\n> ```", "> ```rust\n> "),
            ("- ```rust\n  code\n  ```", "- ```rust\n  "),
            ("> - ```rust\n>   code\n>   ```", "> - ```rust\n>   "),
        ] {
            let source = source.replace('\n', newline);
            let cursor = prefix.replace('\n', newline).len();
            assert_eq!(apply(&source, cursor, EditIntent::DeleteBackward).0, source);
        }
    }
}

#[test]
fn split_origin_survives_source_synchronization_from_undo_and_redo() {
    for source in ["a\nb", "a\\\nb", "a  \nb", "a\r\nb"] {
        let cursor = source.find('b').expect("split fixture has a second line");
        let mut session = EditingSession::new(source, cursor);
        let (entered, entered_cursor) = session.apply(EditIntent::InsertParagraphBreak);
        session.view.set_source(source.to_owned(), 1);
        session.view.set_source(entered.clone(), 2);
        session.source = entered;
        session.cursor = entered_cursor;
        session.generation = 2;
        assert_eq!(session.apply(EditIntent::DeleteBackward), (source.to_owned(), cursor));
    }
}

#[test]
fn container_code_backspace_joins_literal_rows_without_container_markers() {
    for newline in ["\n", "\r\n"] {
        for (source, prefix, expected) in [
            (
                "> ```rust\n> first\n> second\n> ```",
                "> ```rust\n> first\n> ",
                "> ```rust\n> firstsecond\n> ```",
            ),
            (
                "- ```rust\n  first\n  second\n  ```",
                "- ```rust\n  first\n  ",
                "- ```rust\n  firstsecond\n  ```",
            ),
            (
                "> - ```rust\n>   first\n>   second\n>   ```",
                "> - ```rust\n>   first\n>   ",
                "> - ```rust\n>   firstsecond\n>   ```",
            ),
        ] {
            let source = source.replace('\n', newline);
            let cursor = prefix.replace('\n', newline).len();
            assert_eq!(
                apply(&source, cursor, EditIntent::DeleteBackward).0,
                expected.replace('\n', newline)
            );
        }
    }
}

#[test]
fn container_code_forward_join_preserves_literal_text_and_tab_residue() {
    for newline in ["\n", "\r\n"] {
        for (source, prefix, expected) in [
            (
                "> ```rust\n> first\n> second\n> ```",
                "> ```rust\n> first",
                "> ```rust\n> firstsecond\n> ```",
            ),
            (
                "- ```rust\n  first\n  second\n  ```",
                "- ```rust\n  first",
                "- ```rust\n  firstsecond\n  ```",
            ),
            (
                "> - ```rust\n>   first\n>   second\n>   ```",
                "> - ```rust\n>   first",
                "> - ```rust\n>   firstsecond\n>   ```",
            ),
            (
                "- ```rust\n  first\n\tsecond\n  ```",
                "- ```rust\n  first",
                "- ```rust\n  first  second\n  ```",
            ),
        ] {
            let source = source.replace('\n', newline);
            let cursor = prefix.replace('\n', newline).len();
            assert_eq!(
                apply(&source, cursor, EditIntent::DeleteForward).0,
                expected.replace('\n', newline)
            );
        }
    }
}

#[test]
fn code_backspace_deletes_tab_residue_without_removing_the_container() {
    for newline in ["\n", "\r\n"] {
        let source = "- ```rust\n  first\n\tsecond\n  ```".replace('\n', newline);
        let cursor = "- ```rust\n  first\n\t".replace('\n', newline).len();
        let expected = "- ```rust\n  first\n  second\n  ```".replace('\n', newline);
        assert_eq!(apply(&source, cursor, EditIntent::DeleteBackward).0, expected);
    }
}

#[test]
fn container_code_enter_preserves_owner_and_literal_body() {
    for newline in ["\n", "\r\n"] {
        for (opening, continuation) in
            [("> ```rust", "> "), ("- ```rust", "  "), ("> - ```rust", ">   ")]
        {
            for intent in [EditIntent::InsertParagraphBreak, EditIntent::InsertLineBreak] {
                let source = format!(
                    "{opening}{newline}{continuation}first> second{newline}{continuation}```"
                );
                let cursor = opening.len() + newline.len() + continuation.len() + "first".len();
                let expected = format!(
                    "{opening}{newline}{continuation}first{newline}{continuation}> second{newline}{continuation}```"
                );
                assert_eq!(
                    apply(&source, cursor, intent.clone()),
                    (expected, cursor + newline.len() + continuation.len()),
                    "{intent:?}: {source:?}"
                );
            }
        }
    }
}

#[test]
fn existing_container_code_opening_enter_keeps_body_inside_owner() {
    for newline in ["\n", "\r\n"] {
        for (opening, continuation) in
            [("> ```rust", "> "), ("- ```rust", "  "), ("> - ```rust", ">   ")]
        {
            let source = format!("{opening}{newline}{continuation}body{newline}{continuation}```");
            for intent in [EditIntent::InsertParagraphBreak, EditIntent::InsertLineBreak] {
                let expected = format!(
                    "{opening}{newline}{continuation}{newline}{continuation}body{newline}{continuation}```"
                );
                assert_eq!(
                    apply(&source, opening.len(), intent.clone()),
                    (expected, opening.len() + newline.len() + continuation.len()),
                    "{intent:?}: {source:?}"
                );
            }
        }
    }
}

#[test]
fn code_body_end_enter_adds_a_visible_row_and_backspace_restores_geometry() {
    fn caret_geometry(
        session: &mut EditingSession,
        shaper: &mut shaping::Shaper,
    ) -> (f32, f32, f32, f32) {
        let mut document = AuditDocument(session.source.clone());
        session.view.handle_message(PluginMessage::SetCursorByte(session.cursor), &mut document);
        session.view.render(
            &StringDocView::new(&session.source),
            ui::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
            &ui::theme::test_theme(),
            shaper,
            1.0,
        );
        session.view.engine().cursor_screen_pos().expect("code-body caret must be rendered")
    }
    let mut shaper = shaping::Shaper::new().expect("code-body geometry shaper must initialize");
    for newline in ["\n", "\r\n"] {
        for (opening, continuation) in
            [("```rust", ""), ("> ```rust", "> "), ("- ```rust", "  "), ("> - ```rust", ">   ")]
        {
            for closed in [false, true] {
                let prefix = format!("{opening}{newline}{continuation}body");
                let source = prefix.clone()
                    + &if closed { format!("{newline}{continuation}```") } else { String::new() };
                let mut session = EditingSession::new(&source, prefix.len());
                let before = caret_geometry(&mut session, &mut shaper);
                session.apply(EditIntent::InsertParagraphBreak);
                let entered = caret_geometry(&mut session, &mut shaper);
                assert!(
                    entered.1 > before.1 + GEOMETRY_TOLERANCE,
                    "Enter must move to a visible code row: {source:?}, {before:?} -> {entered:?}"
                );
                assert_eq!(
                    session.apply(EditIntent::DeleteBackward),
                    (source.clone(), prefix.len())
                );
                let restored = caret_geometry(&mut session, &mut shaper);
                for (actual, expected) in [
                    (restored.0, before.0),
                    (restored.1, before.1),
                    (restored.2, before.2),
                    (restored.3, before.3),
                ] {
                    assert!(
                        (actual - expected).abs() <= GEOMETRY_TOLERANCE,
                        "Backspace must restore exact caret geometry: {source:?}, {before:?} -> {restored:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn long_code_caret_remains_visible_and_hit_testable_after_navigation_and_resize() {
    use ui::plugin::{EditHitTarget, PluginQuery, PluginResponse};
    let mut shaper = shaping::Shaper::new().expect("long-code audit requires font shaping");
    let theme = ui::theme::test_theme();
    let mut violations = Vec::new();
    for newline in ["\n", "\r\n"] {
        for (opening, continuation) in [("```rust", ""), ("> ```rust", "> "), ("- ```rust", "  ")] {
            let prefix = format!("{opening}{newline}{continuation}");
            let body = "let 中文 = \"é 👩🏽‍💻\"; ".repeat(20) + "VISIBLE_END";
            let source = format!("{prefix}{body}{newline}{continuation}```");
            let mut session = EditingSession::new(&source, prefix.len());
            let mut document = AuditDocument(source.clone());
            let middle = body
                .grapheme_indices(true)
                .nth(body.graphemes(true).count() / 2)
                .expect("long code body has a middle grapheme")
                .0;
            for width in [800.0, 240.0, 800.0] {
                for cursor in [prefix.len() + body.len(), prefix.len() + middle, prefix.len()] {
                    session
                        .view
                        .handle_message(PluginMessage::SetCursorByte(cursor), &mut document);
                    session.view.render(
                        &document,
                        ui::Rect::new(0.0, 0.0, width, VIEWPORT_HEIGHT),
                        &theme,
                        &mut shaper,
                        1.0,
                    );
                    let caret =
                        session.view.engine().cursor_screen_pos().expect("code caret exists");
                    if caret.0 < 0.0 || caret.0 + caret.2 > width {
                        violations.push(format!(
                            "caret outside {width}px: {opening:?} {newline:?} @{cursor}: {caret:?}"
                        ));
                    }
                    let hit = session.view.query(
                        PluginQuery::HitTestEditTarget {
                            x: caret.0,
                            y: caret.1 + caret.3 * 0.5,
                            offset_x: 0.0,
                            offset_y: 0.0,
                        },
                        &document,
                    );
                    assert!(
                        matches!(hit, PluginResponse::EditHitTarget(Some(EditHitTarget::TextCaret { byte_offset, .. })) if byte_offset == cursor),
                        "shifted code geometry must hit the same source position: {hit:?}, expected {cursor}"
                    );
                    assert_eq!(
                        document.0, source,
                        "horizontal navigation must preserve source bytes"
                    );
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "{} invisible carets:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

#[test]
fn deferred_caret_reveal_waits_for_layout_and_preserves_later_manual_scrolling() {
    const SECTION_COUNT: usize = 80;
    const VIEWPORT_HEIGHT: f32 = 240.0;
    let source = "段落 English é\n\n---\n\n```rust\nlet value = 1;\n```\n\n".repeat(SECTION_COUNT);
    let mut session = EditingSession::new(&source, source.len());
    let mut document = AuditDocument(source.clone());
    let mut shaper = shaping::Shaper::new().expect("deferred reveal needs font shaping");
    let theme = ui::theme::test_theme();
    session.view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut document);
    session.view.handle_message(PluginMessage::RevealCursor(source.len()), &mut document);
    session.view.render(
        &document,
        ui::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
        &theme,
        &mut shaper,
        1.0,
    );
    let (_, y, _, height) = session.view.engine().cursor_screen_pos().expect("revealed end caret");
    assert!(y >= 0.0 && y + height <= VIEWPORT_HEIGHT + GEOMETRY_TOLERANCE);
    for cancel_pending in [false, true] {
        if cancel_pending {
            session.view.handle_message(PluginMessage::RevealCursor(source.len()), &mut document);
        }
        let delta = -session.view.engine().content_height;
        session.view.handle_message(
            PluginMessage::Scroll { delta, viewport_h: VIEWPORT_HEIGHT },
            &mut document,
        );
        session.view.render(
            &document,
            ui::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
            &theme,
            &mut shaper,
            1.0,
        );
        assert_eq!(
            session.view.engine().scroll_y,
            0.0,
            "manual scroll must supersede an old reveal request"
        );
        assert!(
            session.view.engine().cursor_screen_pos().expect("offscreen end caret").1
                > VIEWPORT_HEIGHT
        );
    }
    assert_eq!(document.0, source);
}

#[test]
fn horizontally_revealed_code_clips_selection_and_search_highlights() {
    use ui::core::paint::DrawCmd;
    use ui::plugin::{PluginQuery, PluginResponse};
    let body = "let value = 1; ".repeat(30);
    let source = format!("```rust\n{body}\n```");
    let body_start = "```rust\n".len();
    let body_end = body_start + body.len();
    let mut session = EditingSession::new(&source, body_end);
    let mut document = AuditDocument(source.clone());
    let mut shaper = shaping::Shaper::new().expect("code highlight audit requires font shaping");
    session.view.handle_message(PluginMessage::SetCursorByte(body_end), &mut document);
    session.view.handle_message(PluginMessage::SetSelAnchorByte(Some(body_start)), &mut document);
    session.view.handle_message(PluginMessage::SetSelCursorByte(Some(body_end)), &mut document);
    session.view.render(
        &document,
        ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
        &ui::theme::test_theme(),
        &mut shaper,
        1.0,
    );
    let color = [0.1, 0.2, 0.3, 0.5];
    for selection_end in [body_end, source.len()] {
        session
            .view
            .handle_message(PluginMessage::SetSelCursorByte(Some(selection_end)), &mut document);
        for query in [
            PluginQuery::SelectionHighlights(color),
            PluginQuery::SourceSearchHighlights {
                matches: std::iter::once(body_start..selection_end).collect(),
                source_generation: session.generation,
                active_idx: 0,
                match_color: color,
                inactive_color: color,
            },
        ] {
            let description = format!("{query:?}");
            let PluginResponse::DrawList(highlights) = session.view.query(query, &document) else {
                panic!("highlight query must return drawing commands");
            };
            assert!(
                !highlights.cmds.is_empty(),
                "visible selected code must retain its highlight: {description}"
            );
            for command in highlights.cmds {
                if let DrawCmd::FillRect { rect, .. } = command {
                    assert!(
                        rect.x >= 0.0 && rect.x + rect.w <= 240.0,
                        "code highlight must stay within its viewport: {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn long_code_left_navigation_preserves_the_visible_horizontal_viewport() {
    use ui::plugin::{EditHitTarget, PluginQuery, PluginResponse};
    let source = format!("```rust\n{}END\n```", "let value = 1; ".repeat(30));
    let end = source.find("END").expect("fixture contains end marker") + "END".len();
    let mut session = EditingSession::new(&source, end);
    let mut document = AuditDocument(source);
    let mut shaper = shaping::Shaper::new().expect("horizontal navigation audit requires fonts");
    let mut positions = Vec::new();
    for cursor in [end, end - 1, end - 2] {
        session.view.handle_message(PluginMessage::SetCursorByte(cursor), &mut document);
        session.view.render(
            &document,
            ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
            &ui::theme::test_theme(),
            &mut shaper,
            1.0,
        );
        positions.push(session.view.engine().cursor_screen_pos().expect("code caret exists").0);
    }
    assert!(
        positions.windows(2).all(|pair| pair[1] < pair[0] - GEOMETRY_TOLERANCE),
        "Left must move the caret inside the existing code viewport: {positions:?}"
    );
    session.cursor = end - 2;
    session.apply(EditIntent::InsertText("x".into()));
    document.0 = session.source.clone();
    session.view.handle_message(PluginMessage::SetCursorByte(session.cursor), &mut document);
    session.view.render(
        &document,
        ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
        &ui::theme::test_theme(),
        &mut shaper,
        1.0,
    );
    let inserted = session.view.engine().cursor_screen_pos().expect("inserted code caret exists");
    assert!(
        inserted.0 > positions[2] + GEOMETRY_TOLERANCE
            && inserted.0 < positions[0] - GEOMETRY_TOLERANCE,
        "source rebuild must retain the horizontal viewport: {positions:?} -> {inserted:?}"
    );
    let hit = session.view.query(
        PluginQuery::HitTestEditTarget {
            x: positions[2],
            y: inserted.1 + inserted.3 * 0.5,
            offset_x: 0.0,
            offset_y: 0.0,
        },
        &document,
    );
    let PluginResponse::EditHitTarget(Some(EditHitTarget::TextCaret { byte_offset, .. })) = hit
    else {
        panic!("visible code must be hit-testable: {hit:?}");
    };
    assert_eq!(byte_offset, end - 2);
    session.view.handle_message(PluginMessage::SetCursorByte(byte_offset), &mut document);
    session.view.render(
        &document,
        ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
        &ui::theme::test_theme(),
        &mut shaper,
        1.0,
    );
    let clicked = session.view.engine().cursor_screen_pos().expect("clicked code caret exists");
    assert!(
        (clicked.0 - positions[2]).abs() <= GEOMETRY_TOLERANCE,
        "mouse targeting must keep the caret at the clicked horizontal position"
    );
}

#[test]
fn long_code_preedit_keeps_the_composition_caret_inside_the_viewport() {
    use ui::core::paint::DrawCmd;
    use ui::plugin::{PluginQuery, PluginResponse};
    let mut shaper = shaping::Shaper::new().expect("code composition audit requires fonts");
    let theme = ui::theme::test_theme();
    for newline in ["\n", "\r\n"] {
        for (opening, continuation) in [("```rust", ""), ("> ```rust", "> "), ("- ```rust", "  ")] {
            let source = format!(
                "{opening}{newline}{continuation}{}END{newline}{continuation}```",
                "let value = 1; ".repeat(30)
            );
            let cursor = source.find("END").expect("fixture contains end marker") + "END".len();
            let mut session = EditingSession::new(&source, cursor);
            let mut document = AuditDocument(source.clone());
            session.view.handle_message(PluginMessage::SetCursorByte(cursor), &mut document);
            session.view.handle_message(PluginMessage::SetCursorVisible(true), &mut document);
            session.view.render(
                &document,
                ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
                &theme,
                &mut shaper,
                1.0,
            );
            let original =
                session.view.engine().cursor_screen_pos().expect("original code caret exists");
            for preedit in ["拼音".to_owned(), "中文".repeat(20), "é👩🏽‍💻".to_owned()]
            {
                session.view.handle_message(
                    PluginMessage::SetPreedit {
                        text: preedit.clone(),
                        cursor: Some((preedit.len(), preedit.len())),
                    },
                    &mut document,
                );
                let drawing = session.view.render(
                    &document,
                    ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
                    &theme,
                    &mut shaper,
                    1.0,
                );
                let carets = drawing
                    .cmds
                    .iter()
                    .filter_map(|command| match command {
                        DrawCmd::FillRect { rect, color, .. } if *color == theme.editor.cursor => {
                            Some(rect)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(carets.len(), 1, "composition must draw one caret");
                assert!(
                    carets[0].x >= 0.0 && carets[0].x + carets[0].w <= 240.0,
                    "long-code composition caret must stay visible: {:?}",
                    carets[0]
                );
                assert_eq!(document.0, source, "preedit must not modify source bytes");
                let PluginResponse::CursorScreenRect(Some(candidate)) =
                    session.view.query(PluginQuery::CursorScreenPos(cursor), &document)
                else {
                    panic!("composition requires a candidate anchor")
                };
                assert!(
                    (candidate.0 - (carets[0].x + carets[0].w * 0.5)).abs() <= GEOMETRY_TOLERANCE,
                    "candidate anchor and rendered caret must agree"
                );
                for command in &drawing.cmds {
                    if let DrawCmd::FillRect { rect, color, .. } = command
                        && *color == theme.editor.foreground
                        && rect.h == 1.0
                    {
                        assert!(
                            rect.x >= 0.0 && rect.x + rect.w <= 240.0,
                            "composition underline must stay inside the code viewport: {rect:?}"
                        );
                    }
                }
                session.view.handle_message(
                    PluginMessage::SetPreedit { text: String::new(), cursor: None },
                    &mut document,
                );
                session.view.render(
                    &document,
                    ui::Rect::new(0.0, 0.0, 240.0, VIEWPORT_HEIGHT),
                    &theme,
                    &mut shaper,
                    1.0,
                );
                let restored =
                    session.view.engine().cursor_screen_pos().expect("cancelled code caret exists");
                for (actual, expected) in [
                    (restored.0, original.0),
                    (restored.1, original.1),
                    (restored.2, original.2),
                    (restored.3, original.3),
                ] {
                    assert!(
                        (actual - expected).abs() <= GEOMETRY_TOLERANCE,
                        "cancellation must restore caret geometry"
                    );
                }
            }
        }
    }
}
