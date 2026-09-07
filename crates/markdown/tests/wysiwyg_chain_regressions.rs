extern crate core as doc_core;
use doc_core::document::{DocView, DocViewMut, StringDocView};
use std::borrow::Cow;
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::{MoveDirection, PluginMessage, PluginQuery, PluginResponse, ViewPlugin};
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
impl DocViewMut for Document {
    fn set_scroll_y(&mut self, _: f32) {}
    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.0.replace_range(range, text)
    }
}
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
    fn editor(source: &str, cursor: usize) -> (MarkdownEditorView, Document) {
        let mut document = Document(source.to_owned());
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        view.handle_message(PluginMessage::SetCursorByte(cursor), &mut document);
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
        let (view, document) = editor("before\n\nabc ![x](y) tail\n\nafter", 0);
        assert!(
            matches!(
                view.query(PluginQuery::CursorScreenPos(0), &document),
                PluginResponse::CursorScreenRect(Some(_))
            ),
            "image must not remove first paragraph caret"
        );
        assert!(matches!(
            view.query(
                PluginQuery::VisualMove {
                    current_byte: 0,
                    direction: MoveDirection::Right,
                    target_x: None
                },
                &document
            ),
            PluginResponse::BytePosition(Some(1))
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
            let (view, document) = editor(&source, 0);
            let tail = source.find('末').expect("tail exists");
            for byte in [0, 3, tail, tail + 3, source.len()] {
                assert!(
                    matches!(
                        view.query(PluginQuery::CursorScreenPos(byte), &document),
                        PluginResponse::CursorScreenRect(Some(_))
                    ),
                    "missing caret {byte} for {source:?}"
                );
            }
            assert!(
                matches!(view.query(PluginQuery::VisualMove{current_byte:tail,direction:MoveDirection::Right,target_x:None},&document),PluginResponse::BytePosition(Some(end)) if end==tail+3)
            );
        }
    }
    #[test]
    fn decoded_entity_navigation_uses_its_complete_source_range() {
        for entity in ["&copy;", "&NotEqualTilde;", "&#x1F600;", "&fjlig;"] {
            let source = format!("a {entity} z");
            let (view, document) = editor(&source, 0);
            assert!(
                matches!(view.query(PluginQuery::VisualMove{current_byte:2,direction:MoveDirection::Right,target_x:None},&document),PluginResponse::BytePosition(Some(end)) if end==2+entity.len()),
                "entity must be a complete source unit: {source:?}"
            );
        }
    }
    #[test]
    fn inline_code_expands_both_delimiters_and_maps_chinese_caret() {
        let (view, document) = editor("abc `中文` tail", 5);
        assert_eq!(
            rendered_text(&view),
            "abc `中文` tail",
            "active inline code must expose both delimiters"
        );
        assert!(matches!(
            view.query(PluginQuery::CursorScreenPos(5), &document),
            PluginResponse::CursorScreenRect(Some(_))
        ));
    }
    #[test]
    fn inline_code_content_mapping_preserves_delimiters_and_padding() {
        for source in ["abc ``中`文`` tail", "abc `` `中` `` tail", "abc `  中文  ` tail"] {
            let cursor = source.find('中').expect("fixture has chinese code");
            let (view, document) = editor(source, cursor);
            assert_eq!(
                rendered_text(&view),
                source,
                "active code must retain the exact delimiters and padding"
            );
            for (byte, character) in source.char_indices() {
                if character == '中' || character == '文' {
                    assert!(
                        matches!(
                            view.query(PluginQuery::CursorScreenPos(byte), &document),
                            PluginResponse::CursorScreenRect(Some(_))
                        ),
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
            let (view, document) = editor(source, cursor);
            assert!(matches!(
                view.query(PluginQuery::CursorScreenPos(cursor), &document),
                PluginResponse::CursorScreenRect(Some(_))
            ));
            let visible = rendered_text(&view);
            assert!(visible.contains('a') && visible.contains('中'));
        }
    }
    #[test]
    fn multiline_inline_code_normalizes_display_without_inventing_source_offsets() {
        for newline in ["\n", "\r\n"] {
            let source = format!("abc ``a{newline}中文`` tail");
            let cursor = source.find('中').expect("Chinese code content");
            let (view, document) = editor(&source, cursor);
            assert_eq!(
                rendered_text(&view),
                format!("abc ``a{}中文`` tail", " ".repeat(newline.len()))
            );
            assert!(matches!(
                view.query(PluginQuery::CursorScreenPos(cursor), &document),
                PluginResponse::CursorScreenRect(Some(_))
            ));
        }
    }
    #[test]
    fn code_projection_preserves_parser_normalization_in_containers_and_tables() {
        for source in ["> ``a\n> >中文``", "| column |\n| --- |\n| `中\\|文` |"] {
            let cursor = source.find('中').expect("Chinese code");
            let (view, document) = editor(source, cursor);
            let visible = rendered_text(&view);
            assert!(
                visible.contains("中文``") || visible.contains("中|文"),
                "code text differs from parser: {visible:?}"
            );
            assert!(matches!(
                view.query(PluginQuery::CursorScreenPos(cursor), &document),
                PluginResponse::CursorScreenRect(Some(_))
            ));
        }
    }
    #[test]
    fn navigation_remains_available_between_edit_and_paint() {
        let (mut view, mut document) = editor("abc", 1);
        document.0 = "aXbc".into();
        view.set_source(document.0.clone(), 2);
        view.handle_message(PluginMessage::SetCursorByte(2), &mut document);
        assert!(
            matches!(
                view.query(
                    PluginQuery::VisualMove {
                        current_byte: 2,
                        direction: MoveDirection::Right,
                        target_x: None
                    },
                    &document
                ),
                PluginResponse::BytePosition(Some(3))
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
            view.handle_message(PluginMessage::SetCursorByte(3), &mut document);
            for (direction, expected) in [
                (MoveDirection::Left, 0),
                (MoveDirection::Right, 6),
                (MoveDirection::LineStart, 0),
                (MoveDirection::LineEnd, source.find('\n').expect("newline")),
            ] {
                assert!(
                    matches!(view.query(PluginQuery::VisualMove{current_byte:3,direction,target_x:None},&document),PluginResponse::BytePosition(Some(byte)) if byte==expected),
                    "generation {generation} must use its own source for {direction:?}"
                );
            }
            let PluginResponse::CursorScreenRect(Some((x, y, _, height))) =
                view.query(PluginQuery::CursorScreenPos(3), &document)
            else {
                panic!("current Unicode caret has geometry")
            };
            assert!(
                matches!(view.query(PluginQuery::HitTestByte{x,y:y+height/2.0,offset_x:0.0,offset_y:0.0},&document),PluginResponse::BytePosition(Some(byte)) if byte<=source.len()&&source.is_char_boundary(byte))
            );
            assert!(
                matches!(view.query(PluginQuery::VisualMove{current_byte:0,direction:MoveDirection::Down,target_x:Some(0.0)},&document),PluginResponse::BytePosition(Some(byte)) if byte>0&&source.is_char_boundary(byte))
            );
        }
        render(&mut view, &document);
        assert_eq!(rendered_text(&view), "一二三\n末");
    }
    #[test]
    fn search_highlights_include_visible_matches() {
        let (view, document) = editor("one two one", 0);
        let response = view.query(
            PluginQuery::SearchHighlights {
                query: "one".into(),
                match_case: true,
                use_regex: false,
                active_idx: 0,
                match_color: ui::theme::test_theme().palette.highlight,
                inactive_color: ui::theme::test_theme().palette.inactive_highlight,
            },
            &document,
        );
        assert!(
            matches!(response,PluginResponse::DrawList(ref commands) if !commands.cmds.is_empty()),
            "two visible matches need search highlight geometry"
        );
    }
    #[test]
    fn source_search_highlights_folded_markup_and_preserves_match_indices() {
        let source = "**one** two one";
        let (view, document) = editor(source, source.len());
        let active = [1.0, 0.0, 0.0, 1.0];
        let inactive = [0.0, 1.0, 0.0, 1.0];
        let response = view.query(
            PluginQuery::SourceSearchHighlights {
                matches: vec![0..7, 11..14],
                source_generation: 1,
                active_idx: 1,
                match_color: active,
                inactive_color: inactive,
            },
            &document,
        );
        let PluginResponse::DrawList(highlights) = response else { panic!("highlights response") };
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
        let (view, document) = editor("xe\u{301}y", 0);
        let response = view.query(
            PluginQuery::SourceSearchHighlights {
                matches: std::iter::once(2..4).collect(),
                source_generation: 1,
                active_idx: 0,
                match_color: [1.0; 4],
                inactive_color: [0.5; 4],
            },
            &document,
        );
        assert!(
            matches!(response,PluginResponse::DrawList(ref highlights) if !highlights.cmds.is_empty()),
            "a combining-mark match must highlight its containing grapheme"
        );
    }
    #[test]
    fn preedit_previews_replacement_at_selection_start() {
        let (mut view, mut document) = editor("one two one", 3);
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut document);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(3)), &mut document);
        view.handle_message(
            PluginMessage::SetPreedit { text: "新".into(), cursor: Some((3, 3)) },
            &mut document,
        );
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
            let plan = view.edit_policy().plan_edit(&EditRequest {
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
}
