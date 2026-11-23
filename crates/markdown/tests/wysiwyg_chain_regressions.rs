use std::borrow::Cow;
use textora_core::document::{DocView, DocViewMut, StringDocView};
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::{EditPolicy, EditableView};
use ui::plugin::{MoveDirection, ViewPlugin, ViewPluginCapabilities};
struct Document(String);
impl DocView for Document {
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
impl DocViewMut for Document {}
fn render(view: &mut MarkdownEditorView, doc: &Document) {
    let mut shaper = shaping::Shaper::new().expect("shaper available");
    view.render(
        doc,
        ui::Rect::new(0.0, 0.0, 800.0, 600.0),
        &ui::theme::test_theme(),
        &mut shaper,
        1.0,
    );
}
mod tests {
    use super::*;
    use ui::plugin::{EditIntent, EditPlan, EditRequest};

    fn inserted_table(
        source: &str,
        cursor_byte: usize,
        columns: usize,
        rows: usize,
    ) -> (String, usize) {
        let plan = textora_markdown::commands::plan_semantic_edit(
            source,
            1,
            cursor_byte,
            None,
            ui::plugin::SemanticEditCommand::InsertTable { columns, rows },
        );
        let ui::plugin::SemanticEditPlan::Apply(transaction) = plan else {
            panic!("table insertion should produce one transaction");
        };
        let replacement = transaction.replacements.first().expect("table replacement exists");
        let mut result = source.to_owned();
        result.replace_range(replacement.range.clone(), &replacement.text);
        let ui::plugin::EditSelection::Caret(cursor_after) = transaction.selection_after else {
            panic!("table insertion should place a caret");
        };
        (result, cursor_after)
    }

    #[test]
    fn insert_table_generates_parseable_empty_tables_and_places_caret_in_first_header_cell() {
        for columns in [1, 3] {
            let (source, cursor) = inserted_table("", 0, columns, 2);
            let parsed = textora_markdown::parser::parse_markdown(&source);
            assert!(
                parsed.events.iter().any(|event| matches!(
                    event,
                    textora_markdown::parser::MarkdownEvent::Start(
                        textora_markdown::parser::MarkdownTag::Table(_)
                    )
                )),
                "generated source must parse as a GFM table: {source:?}"
            );
            assert_eq!(cursor, 2);
        }
    }

    #[test]
    fn insert_table_replaces_empty_paragraph_and_splits_text_at_all_cursor_positions() {
        let (empty, _) = inserted_table("", 0, 1, 2);
        assert!(empty.starts_with('|'));

        let (empty_paragraph, _) = inserted_table("before\n\nafter", 7, 1, 2);
        assert!(empty_paragraph.starts_with("before\n\n|"), "{empty_paragraph:?}");
        assert!(empty_paragraph.ends_with("\n\nafter"));

        let (at_start, _) = inserted_table("alpha", 0, 1, 2);
        assert!(at_start.ends_with("\n\nalpha"));

        let (in_middle, _) = inserted_table("alphaomega", 5, 1, 2);
        assert!(in_middle.starts_with("alpha\n\n|"));
        assert!(in_middle.ends_with("\n\nomega"));

        let (at_end, _) = inserted_table("alpha", 5, 1, 2);
        assert!(at_end.starts_with("alpha\n\n|"));
    }

    #[test]
    fn insert_table_preserves_adjacent_blocks_and_crlf_style() {
        let source = "# title\n\nparagraph\n\n- item";
        let (result, _) = inserted_table(source, 12, 2, 2);
        assert!(result.starts_with("# title\n\n"));
        assert!(result.ends_with("\n\n- item"));

        let (crlf, _) = inserted_table("left right\r\n\r\nnext", 5, 2, 2);
        assert!(crlf.contains("\r\n|"));
        assert!(!crlf.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn insert_table_in_empty_list_line_stays_in_item_and_preserves_following_item() {
        let source = "- before\n  \n- after";
        let cursor = source.find("  \n").expect("list contains a blank line") + 2;
        let (result, _) = inserted_table(source, cursor, 2, 2);
        assert_eq!(result, "- before\n  \n\n  | | |\n  | --- | --- |\n  | | |\n- after");

        let events = textora_markdown::parser::parse_markdown(&result).events;
        let table_columns = events.iter().find_map(|event| match event {
            textora_markdown::parser::MarkdownEvent::Start(
                textora_markdown::parser::MarkdownTag::Table(columns),
            ) => Some(columns.len()),
            _ => None,
        });
        assert_eq!(table_columns, Some(2));
        let first_item_start = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    textora_markdown::parser::MarkdownEvent::Start(
                        textora_markdown::parser::MarkdownTag::Item
                    )
                )
            })
            .expect("first list item should parse");
        let first_item_end = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    textora_markdown::parser::MarkdownEvent::End(
                        textora_markdown::parser::MarkdownTagEnd::Item
                    )
                )
            })
            .expect("first list item should close");
        let table_start = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    textora_markdown::parser::MarkdownEvent::Start(
                        textora_markdown::parser::MarkdownTag::Table(_)
                    )
                )
            })
            .expect("inserted table should parse");
        assert!(first_item_start < table_start && table_start < first_item_end);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    textora_markdown::parser::MarkdownEvent::Start(
                        textora_markdown::parser::MarkdownTag::Item
                    )
                ))
                .count(),
            2,
            "the neighboring list items must remain distinct"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            textora_markdown::parser::MarkdownEvent::Text(text) if text == "before"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            textora_markdown::parser::MarkdownEvent::Text(text) if text == "after"
        )));
    }

    fn editor(source: &str, cursor: usize) -> (MarkdownEditorView, Document) {
        let document = Document(source.to_owned());
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        view.set_cursor_byte(cursor);
        render(&mut view, &document);
        (view, document)
    }
    fn rendered_text(view: &MarkdownEditorView) -> String {
        view.engine()
            .flat_lines()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn image_preserves_unrelated_paragraph_caret_and_navigation() {
        let (view, _document) = editor("before\n\nabc ![x](y) tail\n\nafter", 0);
        assert!(
            view.cursor_screen_rect(0).is_some(),
            "image must not remove first paragraph caret"
        );
        assert!(matches!(
            view.visual_move(0, MoveDirection::Right, None).map(|caret| caret.byte_offset),
            Some(1)
        ));
    }
    #[test]
    fn images_preserve_unicode_and_neighboring_block_navigation() {
        for image in [
            "![](empty.png)",
            "![中文 *强调*](图像.png)",
            "![a](first.png) ![b](second.png)",
            "[![x](image.png)](link)",
        ] {
            let source = format!("前言\n\n{image}\n\n末尾");
            let (view, _document) = editor(&source, 0);
            let tail = source.find('末').expect("tail exists");
            for byte in [0, 3, tail, tail + 3, source.len()] {
                assert!(
                    view.cursor_screen_rect(byte).is_some(),
                    "missing caret {byte} for {source:?}"
                );
            }
            assert!(
                matches!(view.visual_move(tail, MoveDirection::Right, None).map(|caret| caret.byte_offset),Some(end) if end==tail+3)
            );
        }
    }
    #[test]
    fn decoded_entity_navigation_uses_its_complete_source_range() {
        for entity in ["&copy;", "&NotEqualTilde;", "&#x1F600;", "&fjlig;"] {
            let source = format!("a {entity} z");
            let (view, _document) = editor(&source, 0);
            assert!(
                matches!(view.visual_move(2, MoveDirection::Right, None).map(|caret| caret.byte_offset),Some(end) if end==2+entity.len()),
                "entity must be a complete source unit: {source:?}"
            );
        }
    }
    #[test]
    fn inline_code_expands_both_delimiters_and_maps_chinese_caret() {
        let (view, _document) = editor("abc `中文` tail", 5);
        assert_eq!(
            rendered_text(&view),
            "abc `中文` tail",
            "active inline code must expose both delimiters"
        );
        assert!(view.cursor_screen_rect(5).is_some());
    }
    #[test]
    fn inline_code_content_mapping_preserves_delimiters_and_padding() {
        for source in ["abc ``中`文`` tail", "abc `` `中` `` tail", "abc `  中文  ` tail"] {
            let cursor = source.find('中').expect("fixture has chinese code");
            let (view, _document) = editor(source, cursor);
            assert_eq!(
                rendered_text(&view),
                source,
                "active code must retain the exact delimiters and padding"
            );
            for (byte, character) in source.char_indices() {
                if character == '中' || character == '文' {
                    assert!(
                        view.cursor_screen_rect(byte).is_some(),
                        "unmapped source boundary {byte} in {source:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn adjacent_code_spans_retain_independent_source_delimiters() {
        let source = "```a```**`中`**";
        for cursor in [3, source.find('中').expect("Chinese code")] {
            let (view, _document) = editor(source, cursor);
            assert!(view.cursor_screen_rect(cursor).is_some());
            let visible = rendered_text(&view);
            assert!(visible.contains('a') && visible.contains('中'));
        }
    }
    #[test]
    fn multiline_inline_code_normalizes_display_without_inventing_source_offsets() {
        for newline in ["\n", "\r\n"] {
            let source = format!("abc ``a{newline}中文`` tail");
            let cursor = source.find('中').expect("Chinese code content");
            let (view, _document) = editor(&source, cursor);
            assert_eq!(
                rendered_text(&view),
                format!("abc ``a{}中文`` tail", " ".repeat(newline.len()))
            );
            assert!(view.cursor_screen_rect(cursor).is_some());
        }
    }
    #[test]
    fn code_projection_preserves_parser_normalization_in_containers_and_tables() {
        for source in ["> ``a\n> >中文``", "| column |\n| --- |\n| `中\\|文` |"] {
            let cursor = source.find('中').expect("Chinese code");
            let (view, _document) = editor(source, cursor);
            let visible = rendered_text(&view);
            assert!(
                visible.contains("中文``") || visible.contains("中|文"),
                "code text differs from parser: {visible:?}"
            );
            assert!(view.cursor_screen_rect(cursor).is_some());
        }
    }
    #[test]
    fn navigation_remains_available_between_edit_and_paint() {
        let (mut view, mut document) = editor("abc", 1);
        document.0 = "aXbc".into();
        view.set_source(document.0.clone(), 2);
        view.set_cursor_byte(2);
        assert!(
            matches!(
                view.visual_move(2, MoveDirection::Right, None).map(|caret| caret.byte_offset),
                Some(3)
            ),
            "next input event must see current source without requiring a paint"
        );
    }
    #[test]
    fn consecutive_source_updates_publish_unicode_navigation_before_paint() {
        let (mut view, mut document) = editor("old text", 0);
        for (generation, source) in [(2, "甲乙\n\n尾"), (3, "一二三\n\n末")] {
            document.0 = source.into();
            view.set_source(document.0.clone(), generation);
            view.set_cursor_byte(3);
            for (direction, expected) in [
                (MoveDirection::Left, 0),
                (MoveDirection::Right, 6),
                (MoveDirection::LineStart, 0),
                (MoveDirection::LineEnd, source.find('\n').expect("newline")),
            ] {
                assert!(
                    matches!(view.visual_move(3, direction, None).map(|caret| caret.byte_offset),Some(byte) if byte==expected),
                    "generation {generation} must use its own source for {direction:?}"
                );
            }
            let Some((x, y, _, height)) = view.cursor_screen_rect(3) else {
                panic!("current Unicode caret has geometry")
            };
            assert!(
                matches!(view.hit_test_caret(x, y+height/2.0, 0.0, 0.0).map(|caret| caret.byte_offset),Some(byte) if byte<=source.len()&&source.is_char_boundary(byte))
            );
            assert!(
                matches!(view.visual_move(0, MoveDirection::Down, Some(0.0)).map(|caret| caret.byte_offset),Some(byte) if byte>0&&source.is_char_boundary(byte))
            );
        }
        render(&mut view, &document);
        assert_eq!(rendered_text(&view), "一二三\n末");
    }
    #[test]
    fn search_highlights_include_visible_matches() {
        let (view, _document) = editor("one two one", 0);
        let response = view.search().expect("Markdown editor provides search").search_highlights(
            ui::plugin::SearchHighlightsRequest {
                query: "one",
                match_case: true,
                use_regex: false,
                active_idx: 0,
                match_color: ui::theme::test_theme().palette.highlight,
                inactive_color: ui::theme::test_theme().palette.inactive_highlight,
            },
        );
        assert!(!response.cmds.is_empty(), "two visible matches need search highlight geometry");
    }
    #[test]
    fn source_search_highlights_folded_markup_and_preserves_match_indices() {
        let source = "**one** two one";
        let (view, _document) = editor(source, source.len());
        let active = [1.0, 0.0, 0.0, 1.0];
        let inactive = [0.0, 1.0, 0.0, 1.0];
        let response = view
            .search()
            .expect("Markdown editor provides search")
            .source_search_highlights(ui::plugin::SourceSearchHighlightsRequest {
                matches: &[0..7, 11..14],
                source_generation: 1,
                active_idx: 1,
                match_color: active,
                inactive_color: inactive,
            });
        let highlights = response.expect("source geometry is supported");
        let colors = highlights
            .cmds
            .iter()
            .filter_map(|command| match command {
                ui::core::paint::DrawCmd::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            colors,
            vec![inactive, active],
            "folded markers must not hide their matched content"
        );
    }
    #[test]
    fn source_search_highlights_partial_grapheme_matches() {
        let (view, _document) = editor("xe\u{301}y", 0);
        let response = view
            .search()
            .expect("Markdown editor provides search")
            .source_search_highlights(ui::plugin::SourceSearchHighlightsRequest {
                matches: std::slice::from_ref(&(2..4)),
                source_generation: 1,
                active_idx: 0,
                match_color: [1.0; 4],
                inactive_color: [0.5; 4],
            });
        assert!(
            response.is_some_and(|highlights| !highlights.cmds.is_empty()),
            "a combining-mark match must highlight its containing grapheme"
        );
    }
    #[test]
    fn preedit_previews_replacement_at_selection_start() {
        let (mut view, document) = editor("one two one", 3);
        view.selection_mut()
            .expect("test view exposes selection")
            .set_selection_anchor_byte(Some(0));
        view.selection_mut()
            .expect("test view exposes selection")
            .set_selection_cursor_byte(Some(3));
        view.set_preedit("新".into(), Some((3, 3)));
        render(&mut view, &document);
        assert_eq!(
            rendered_text(&view),
            "新 two one",
            "preedit should show the selection replacement that commit will apply"
        );
    }
    #[test]
    fn code_html_literal_backspace_removes_one_character() {
        for source in ["```\n<br>\n```", "`<br>`"] {
            let cursor = source.find("<br>").expect("fixture has html text") + 4;
            let (view, _document) = editor(source, cursor);
            let plan = view.plan_edit(&EditRequest {
                source_generation: 1,
                cursor_byte: cursor,
                selection: None,
                intent: EditIntent::DeleteBackward,
            });
            let actual = match plan {
                EditPlan::UseDefault => {
                    let mut text = source.to_owned();
                    text.replace_range(cursor - 1..cursor, "");
                    text
                }
                EditPlan::Apply(transaction) => {
                    let mut text = source.to_owned();
                    for replacement in transaction.replacements.iter().rev() {
                        text.replace_range(replacement.range.clone(), &replacement.text)
                    }
                    text
                }
                other => panic!("unexpected plan {other:?}"),
            };
            let mut expected = source.to_owned();
            expected.remove(cursor - 1);
            assert_eq!(actual, expected, "code contents must remain literal");
        }
    }
    #[test]
    fn rich_paste_preserves_literal_html_and_entities() {
        use textora_markdown::{parser, paste};
        for (html, plain) in [
            ("<p><strong>&lt;br&gt;</strong></p>", "<br>"),
            ("<p><strong>&amp;copy;</strong></p>", "&copy;"),
        ] {
            let prepared = paste::prepare_paste(paste::PasteRepresentations {
                markdown: None,
                html: Some(html),
                rtf: None,
                plain: Some(plain),
                source_url: None,
            });
            let markdown = prepared.into_text().expect("converted paste text");
            let parsed = parser::parse_markdown(&markdown);
            let rendered = parsed
                .events
                .iter()
                .filter_map(|event| {
                    if let parser::MarkdownEvent::Text(text) = event {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<String>();
            assert_eq!(
                rendered, plain,
                "serialized markdown must preserve clipboard text: {markdown:?}"
            );
        }
    }
    fn formatted_source(source: &str, command: ui::plugin::SemanticEditCommand) -> String {
        let plan = textora_markdown::commands::plan_semantic_edit(
            source,
            1,
            source.len(),
            Some(0..source.len()),
            command,
        );
        let ui::plugin::SemanticEditPlan::Apply(transaction) = plan else {
            panic!("format command must apply")
        };
        let mut text = source.to_owned();
        for replacement in transaction.replacements.iter().rev() {
            text.replace_range(replacement.range.clone(), &replacement.text)
        }
        text
    }
    #[test]
    fn inline_code_command_preserves_embedded_backticks() {
        use textora_markdown::parser::{MarkdownEvent, parse_markdown};
        let formatted = formatted_source("a`b", ui::plugin::SemanticEditCommand::ToggleInlineCode);
        let parsed = parse_markdown(&formatted);
        let codes = parsed
            .events
            .iter()
            .filter_map(|event| {
                if let MarkdownEvent::Code(text) = event { Some(text.as_str()) } else { None }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            vec!["a`b"],
            "formatted source {formatted:?} must preserve all selected code"
        );
    }
    #[test]
    fn code_block_command_preserves_embedded_fences() {
        use textora_markdown::parser::{MarkdownEvent, MarkdownTag, parse_markdown};
        let formatted =
            formatted_source("before\n```\nafter", ui::plugin::SemanticEditCommand::CodeBlock);
        let parsed = parse_markdown(&formatted);
        let block_count = parsed
            .events
            .iter()
            .filter(|event| matches!(event, MarkdownEvent::Start(MarkdownTag::CodeBlock { .. })))
            .count();
        assert_eq!(
            block_count, 1,
            "formatted source {formatted:?} must form one intact code block"
        );
    }

    fn apply_view_intent_on(
        view: &mut MarkdownEditorView,
        source: &mut String,
        generation: &mut u32,
        cursor: usize,
        selection: Option<std::ops::Range<usize>>,
        intent: EditIntent,
    ) -> (usize, Option<ui::plugin::EditTransaction>) {
        use ui::plugin::{EditPolicy, EditSelection};

        let request = EditRequest {
            source_generation: *generation,
            cursor_byte: cursor,
            selection: selection.clone(),
            intent: intent.clone(),
        };
        let mut applied_tx = None;
        let next_cursor = match view.plan_edit(&request) {
            EditPlan::Apply(transaction) | EditPlan::ApplyDefault(transaction, _) => {
                for replacement in transaction.replacements.iter().rev() {
                    source.replace_range(replacement.range.clone(), &replacement.text);
                }
                let caret = match transaction.selection_after {
                    EditSelection::Caret(c) => c,
                    EditSelection::Range { cursor, .. } => cursor,
                };
                applied_tx = Some(transaction);
                caret
            }
            EditPlan::Consume => cursor,
            EditPlan::MoveCursor(u) => u.cursor_after,
            EditPlan::UseDefault => {
                let range = selection.unwrap_or(cursor..cursor);
                let text = match intent {
                    EditIntent::InsertText(t) => t,
                    EditIntent::InsertParagraphBreak | EditIntent::InsertLineBreak => {
                        "\n".to_owned()
                    }
                    _ => String::new(),
                };
                let start = range.start;
                source.replace_range(range, &text);
                start + text.len()
            }
            EditPlan::SetSelection(_) => panic!("unexpected SetSelection"),
        };
        *generation += 1;
        view.set_source(source.clone(), *generation);
        let doc = Document(source.clone());
        render(view, &doc);
        (next_cursor, applied_tx)
    }

    fn apply_view_intent(
        source: &str,
        cursor: usize,
        selection: Option<std::ops::Range<usize>>,
        intent: EditIntent,
    ) -> (String, usize, Option<ui::plugin::EditTransaction>) {
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        let doc = Document(source.to_owned());
        render(&mut view, &doc);
        let mut edited = source.to_owned();
        let mut generation = 1;
        let (caret, tx) = apply_view_intent_on(
            &mut view,
            &mut edited,
            &mut generation,
            cursor,
            selection,
            intent,
        );
        (edited, caret, tx)
    }

    #[test]
    fn wysiwyg_end_to_end_chains_blockquote_lists_and_protected_selections() {
        // 1. BlockQuote 段中 Enter 分裂 -> 同视图 Backspace 还原。
        let mut quote_view = MarkdownEditorView::new();
        let mut quote = String::from("> left right");
        let mut generation = 1;
        quote_view.set_source(quote.clone(), generation);
        render(&mut quote_view, &Document(quote.clone()));
        let (split_caret, _) = apply_view_intent_on(
            &mut quote_view,
            &mut quote,
            &mut generation,
            "> left".len(),
            None,
            EditIntent::InsertParagraphBreak,
        );
        assert_eq!(quote, "> left \n>\n> right");
        assert_eq!(split_caret, "> left \n>\n> ".len());
        let (merged_caret, _) = apply_view_intent_on(
            &mut quote_view,
            &mut quote,
            &mut generation,
            split_caret,
            None,
            EditIntent::DeleteBackward,
        );
        assert_eq!(quote, "> left right");
        assert_eq!(merged_caret, "> left".len());

        // 2. OrderedList 插入新项 -> 自动重编号后续项 -> 空项 Enter 退出再前移重编号。
        let list = "1. alpha\n2. beta\n3. gamma";
        let (inserted, list_caret, _) =
            apply_view_intent(list, "1. alpha".len(), None, EditIntent::InsertParagraphBreak);
        assert_eq!(inserted, "1. alpha\n2. \n3. beta\n4. gamma");
        let (exited, exit_caret, _) =
            apply_view_intent(&inserted, list_caret, None, EditIntent::InsertParagraphBreak);
        assert_eq!(exited, "1. alpha\n\n2. beta\n3. gamma");
        assert_eq!(exit_caret, "1. alpha\n".len());

        // 3. ListItem 完整子树 Tab 缩进 -> Shift+Tab 还原。
        let tree = "- parent\n- target\n  - nested";
        let (indented, tab_caret, _) =
            apply_view_intent(tree, "- parent\n- tar".len(), None, EditIntent::Indent);
        assert_eq!(indented, "- parent\n  - target\n    - nested");
        let (outdented, _, _) = apply_view_intent(&indented, tab_caret, None, EditIntent::Outdent);
        assert_eq!(outdented, tree);

        // 4. 跨 HardBreak 粗体块逐行展开定界符且渲染无异常。
        let hard_break_bold = "**first  \nsecond**";
        let mut bold_view = MarkdownEditorView::new();
        bold_view.set_source(hard_break_bold.to_owned(), 1);
        let bold_doc = Document(hard_break_bold.to_owned());
        bold_view.set_cursor_byte(3);
        render(&mut bold_view, &bold_doc);
        assert_eq!(bold_view.engine().flat_lines().len(), 2);
        assert!(bold_view.engine().flat_lines()[0].text.starts_with("**first"));
        assert!(!bold_view.engine().flat_lines()[1].text.ends_with("**"));

        // 5. 跨围栏与跨单元格选区覆盖 Delete / Type / Paste / Enter，并校验单事务 Undo 还原。
        let fenced = "```\nline1\nline2\n```";
        let close_sel = fenced.find("line2").expect("body")..fenced.len();
        let table = "| col1 | col2 |\n| --- | --- |\n| c1 | c2 |";
        let cross_cell_sel =
            table.find("ol1").expect("col1")..table.find("col2").expect("col2") + 2;
        for intent in [
            EditIntent::DeleteBackward,
            EditIntent::InsertText("X|Y".into()),
            EditIntent::InsertParagraphBreak,
        ] {
            let (fenced_edited, _, tx_opt) =
                apply_view_intent(fenced, close_sel.end, Some(close_sel.clone()), intent.clone());
            assert!(fenced_edited.ends_with("\n```"), "closing fence broken: {fenced_edited:?}");
            let tx = tx_opt.expect("protected fence selection produces atomic tx");
            let mut undone = fenced_edited.clone();
            let mut applied = Vec::new();
            for r in tx.replacements.iter().rev() {
                let s = r.range.start;
                applied.push((s..s + r.text.len(), fenced[r.range.clone()].to_owned()));
            }
            for (r, orig) in applied {
                undone.replace_range(r, &orig);
            }
            assert_eq!(undone, fenced);

            let (table_edited, _, t_tx) =
                apply_view_intent(table, cross_cell_sel.end, Some(cross_cell_sel.clone()), intent);
            assert!(table_edited.contains("| --- | --- |"));
            assert!(t_tx.is_some());
        }
    }
}
