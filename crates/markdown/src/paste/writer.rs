#[cfg(test)]
mod tests {
    use super::write_markdown;
    use crate::parser::MarkdownEvent;
    use crate::paste::{HeadingLevel, ListKind, RichBlock, RichDocument, RichInline};

    fn text(value: &str) -> Vec<RichInline> {
        vec![RichInline::Text(value.into())]
    }

    #[test]
    fn writes_nested_inline_styles_and_escapes_plain_markers() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("literal * ".into()),
            RichInline::Strong(vec![RichInline::Emphasis(vec![RichInline::Text("both".into())])]),
        ])]);

        assert_eq!(write_markdown(&document), r"literal \* ***both***");
    }

    #[test]
    fn plain_text_gfm_markers_reparse_as_text_instead_of_formatting_or_lists() {
        let document = RichDocument::new(vec![
            RichBlock::Paragraph(text("~~plain~~")),
            RichBlock::Paragraph(text("1. item")),
            RichBlock::Paragraph(text("  2. indented item")),
        ]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "\\~\\~plain\\~\\~\n\n1\\. item\n\n  2\\. indented item");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(
                crate::parser::MarkdownTag::Strikethrough | crate::parser::MarkdownTag::List(..)
            )
        )));
        let reparsed_text: String = parsed
            .events
            .iter()
            .filter_map(|event| match event {
                MarkdownEvent::Text(value) => Some(value.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(reparsed_text, "~~plain~~1. item2. indented item");
    }

    #[test]
    fn plain_text_keeps_ordinary_non_structural_punctuation_unescaped() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(text(
            "release 1.2 (stable) | C# and e-mail",
        ))]);

        assert_eq!(write_markdown(&document), "release 1.2 (stable) | C# and e-mail");
    }

    #[test]
    fn code_fence_is_longer_than_backticks_in_content() {
        let document = RichDocument::new(vec![RichBlock::CodeBlock {
            language: Some("rust".into()),
            text: "let marker = ```;\n".into(),
        }]);

        assert_eq!(write_markdown(&document), "````rust\nlet marker = ```;\n````");
    }

    #[test]
    fn code_fence_omits_empty_and_unsafe_language_tags() {
        for language in [Some(""), Some("rust\n# injected")] {
            let document = RichDocument::new(vec![RichBlock::CodeBlock {
                language: language.map(str::to_owned),
                text: "let x = 1;".into(),
            }]);

            assert_eq!(write_markdown(&document), "```\nlet x = 1;\n```");
        }
    }

    #[test]
    fn writes_strikethrough_as_gfm_semantic_markup() {
        let document =
            RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Strikethrough(text(
                "gone",
            ))])]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "~~gone~~");
        assert!(parsed.events.iter().any(|event| matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Strikethrough)
        )));
    }

    #[test]
    fn writes_gfm_table_and_nested_list() {
        let document = RichDocument::new(vec![
            RichBlock::Table {
                header: vec![text("Name"), text("Value")],
                rows: vec![vec![text("A"), text("1")]],
            },
            RichBlock::List {
                kind: ListKind::Unordered,
                items: vec![vec![
                    RichBlock::Paragraph(text("parent")),
                    RichBlock::List {
                        kind: ListKind::Ordered { start: 1 },
                        items: vec![vec![RichBlock::Paragraph(text("child"))]],
                    },
                ]],
            },
        ]);

        assert_eq!(
            write_markdown(&document),
            "| Name | Value |\n| --- | --- |\n| A | 1 |\n\n- parent\n  1. child"
        );
    }

    #[test]
    fn writes_block_families_with_top_level_spacing_and_no_final_newline() {
        let document = RichDocument::new(vec![
            RichBlock::Heading { level: HeadingLevel::H2, content: text("Heading") },
            RichBlock::BlockQuote(vec![
                RichBlock::Paragraph(vec![
                    RichInline::Text("first".into()),
                    RichInline::LineBreak,
                    RichInline::Text("second".into()),
                ]),
                RichBlock::List {
                    kind: ListKind::Unordered,
                    items: vec![vec![RichBlock::Paragraph(text("item"))]],
                },
            ]),
            RichBlock::HorizontalRule,
        ]);

        assert_eq!(
            write_markdown(&document),
            "## Heading\n\n> first  \n> second\n> \n> - item\n\n---"
        );
    }

    #[test]
    fn top_level_separator_preserves_a_final_hard_break_without_three_newlines() {
        let document = RichDocument::new(vec![
            RichBlock::Paragraph(vec![RichInline::Text("before".into()), RichInline::LineBreak]),
            RichBlock::Heading { level: HeadingLevel::H2, content: text("after") },
        ]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "before  \n\n## after");
        assert!(!markdown.ends_with('\n'));
        assert!(parsed.events.iter().any(|event| matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Heading { level: 2 })
        )));
    }

    #[test]
    fn final_text_line_break_does_not_leave_a_trailing_newline() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(text("before\n"))]);

        assert_eq!(write_markdown(&document), "before");
    }

    #[test]
    fn top_level_paragraphs_always_keep_a_double_newline_boundary() {
        let documents = [
            (
                RichDocument::new(vec![
                    RichBlock::Paragraph(vec![
                        RichInline::Text("first".into()),
                        RichInline::LineBreak,
                    ]),
                    RichBlock::Paragraph(text("second")),
                ]),
                "first  \n\nsecond",
            ),
            (
                RichDocument::new(vec![
                    RichBlock::Paragraph(text("first\n")),
                    RichBlock::Paragraph(text("second")),
                ]),
                "first\n\nsecond",
            ),
        ];

        for (document, expected) in documents {
            let markdown = write_markdown(&document);
            let parsed = crate::parser::parse_markdown(&markdown);

            assert_eq!(markdown, expected);
            assert!(!markdown.ends_with('\n'));
            assert_eq!(
                parsed
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        MarkdownEvent::Start(crate::parser::MarkdownTag::Paragraph)
                    ))
                    .count(),
                2
            );
        }
    }

    #[test]
    fn split_text_ordered_list_marker_reparses_as_a_paragraph() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("1".into()),
            RichInline::Text(". item".into()),
        ])]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "1\\. item");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::List(..))
        )));
    }

    #[test]
    fn split_text_heading_marker_reparses_as_a_paragraph() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("  ".into()),
            RichInline::Text("# title".into()),
        ])]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "  \\# title");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Heading { .. })
        )));
    }

    #[test]
    fn empty_inline_wrapper_breaks_line_prefix_structure() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("  ".into()),
            RichInline::Strong(Vec::new()),
            RichInline::Text("# title".into()),
        ])]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "  \\# title");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Heading { .. })
        )));
    }

    #[test]
    fn zero_output_inline_forms_preserve_an_ordered_marker_prefix() {
        let documents = [
            (
                RichDocument::new(vec![RichBlock::Paragraph(vec![
                    RichInline::Text("1".into()),
                    RichInline::Strong(Vec::new()),
                    RichInline::Text(". item".into()),
                ])]),
                "1\\. item",
            ),
            (
                RichDocument::new(vec![RichBlock::Paragraph(vec![
                    RichInline::InlineCode(String::new()),
                    RichInline::Text("1".into()),
                    RichInline::InlineCode(String::new()),
                    RichInline::Text(". item".into()),
                ])]),
                "1\\. item",
            ),
            (
                RichDocument::new(vec![RichBlock::Paragraph(vec![
                    RichInline::Link {
                        destination: "javascript:alert(1)".into(),
                        title: None,
                        children: text("1"),
                    },
                    RichInline::Text(". item".into()),
                ])]),
                "1\\. item",
            ),
            (
                RichDocument::new(vec![RichBlock::Paragraph(vec![
                    RichInline::RemoteImage {
                        destination: "file:///private/image.png".into(),
                        title: None,
                        alt: "1".into(),
                    },
                    RichInline::Text(". item".into()),
                ])]),
                "1\\. item",
            ),
        ];

        for (document, expected) in documents {
            let markdown = write_markdown(&document);
            let parsed = crate::parser::parse_markdown(&markdown);

            assert_eq!(markdown, expected);
            assert!(parsed.events.iter().all(|event| !matches!(
                event,
                MarkdownEvent::Start(crate::parser::MarkdownTag::List(..))
            )));
        }
    }

    #[test]
    fn nonempty_inline_forms_interrupt_an_ordered_marker_prefix() {
        let documents = [
            (RichInline::Strong(text("bold")), "1**bold**. item"),
            (RichInline::Emphasis(text("italic")), "1*italic*. item"),
            (RichInline::InlineCode("code".into()), "1`code`. item"),
            (
                RichInline::Link {
                    destination: "https://example.com".into(),
                    title: None,
                    children: text("link"),
                },
                "1[link](https://example.com). item",
            ),
        ];

        for (inline, expected) in documents {
            let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
                RichInline::Text("1".into()),
                inline,
                RichInline::Text(". item".into()),
            ])]);
            let markdown = write_markdown(&document);
            let parsed = crate::parser::parse_markdown(&markdown);

            assert_eq!(markdown, expected);
            assert!(parsed.events.iter().all(|event| !matches!(
                event,
                MarkdownEvent::Start(crate::parser::MarkdownTag::List(..))
            )));
        }
    }

    #[test]
    fn ordered_parenthesis_markers_reparse_as_plain_text() {
        let documents = [
            (text("1) item"), "1\\) item", false),
            (
                vec![RichInline::Text("  1".into()), RichInline::Text(") item".into())],
                "  1\\) item",
                false,
            ),
            (
                vec![
                    RichInline::Text("   ".into()),
                    RichInline::Text("1".into()),
                    RichInline::Text(") item".into()),
                ],
                "   1\\) item",
                false,
            ),
            (text("    1) item"), "    1) item", true),
        ];

        for (content, expected, expects_code_block) in documents {
            let document = RichDocument::new(vec![RichBlock::Paragraph(content)]);
            let markdown = write_markdown(&document);
            let parsed = crate::parser::parse_markdown(&markdown);

            assert_eq!(markdown, expected);
            assert!(parsed.events.iter().all(|event| !matches!(
                event,
                MarkdownEvent::Start(crate::parser::MarkdownTag::List(..))
            )));
            assert_eq!(
                parsed.events.iter().any(|event| matches!(
                    event,
                    MarkdownEvent::Start(crate::parser::MarkdownTag::CodeBlock { .. })
                )),
                expects_code_block
            );
        }
    }

    #[test]
    fn split_text_strikethrough_marker_reparses_as_plain_text() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("~".into()),
            RichInline::Text("~gone".into()),
            RichInline::Text("~".into()),
            RichInline::Text("~".into()),
        ])]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "\\~\\~gone\\~\\~");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Strikethrough)
        )));
    }

    #[test]
    fn text_exclamation_before_link_does_not_become_an_image() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("!".into()),
            RichInline::Link {
                destination: "https://example.com".into(),
                title: None,
                children: text("link"),
            },
        ])]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "\\![link](https://example.com)");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Image { .. })
        )));
    }

    #[test]
    fn text_setext_underline_reparses_as_plain_text() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(text("title\n==="))]);
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert_eq!(markdown, "title\n\\===");
        assert!(parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::Heading { .. })
        )));
    }

    #[test]
    fn writes_inline_code_links_and_remote_images_without_mutating_code_contents() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::InlineCode("tick ` marker".into()),
            RichInline::Text(" ".into()),
            RichInline::Link {
                destination: "https://example.com/a_(b)".into(),
                title: Some("a \"title\"".into()),
                children: text("link"),
            },
            RichInline::Text(" ".into()),
            RichInline::RemoteImage {
                destination: "https://example.com/image.png".into(),
                title: Some("diagram".into()),
                alt: "diagram".into(),
            },
        ])]);

        assert_eq!(
            write_markdown(&document),
            "``tick ` marker`` [link](<https://example.com/a_(b)> \"a \\\"title\\\"\") ![diagram](https://example.com/image.png \"diagram\")"
        );
    }

    #[test]
    fn inline_code_reparses_with_boundary_backticks_and_spaces_intact() {
        for original in ["a`", "`", " leading", "trailing ", "  "] {
            let document =
                RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::InlineCode(
                    original.into(),
                )])]);
            let markdown = write_markdown(&document);
            let parsed_code: Vec<String> = crate::parser::parse_markdown(&markdown)
                .events
                .into_iter()
                .filter_map(|event| match event {
                    MarkdownEvent::Code(code) => Some(code),
                    _ => None,
                })
                .collect();

            assert_eq!(parsed_code, [original], "failed to preserve {original:?} in {markdown:?}");
        }
    }

    #[test]
    fn empty_inline_code_does_not_create_visible_markdown_text() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::InlineCode(
            String::new(),
        )])]);

        assert_eq!(write_markdown(&document), "");
    }

    #[test]
    fn normalizes_line_breaks_in_link_titles_without_changing_the_destination() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Link {
            destination: "https://example.com".into(),
            title: Some("line\r\ntitle".into()),
            children: text("link"),
        }])]);

        assert_eq!(write_markdown(&document), "[link](https://example.com \"line title\")");
    }

    #[test]
    fn table_cells_escape_pipes_and_flatten_line_breaks() {
        let document = RichDocument::new(vec![RichBlock::Table {
            header: vec![text("A|B")],
            rows: vec![vec![vec![
                RichInline::Text("first".into()),
                RichInline::LineBreak,
                RichInline::Text("second".into()),
            ]]],
        }]);

        assert_eq!(write_markdown(&document), "| A\\|B |\n| --- |\n| first<br>second |");
    }

    #[test]
    fn table_preserves_ragged_rows_when_the_header_is_empty() {
        let document = RichDocument::new(vec![RichBlock::Table {
            header: Vec::new(),
            rows: vec![vec![text("A"), Vec::new()], vec![text("B")], Vec::new()],
        }]);

        assert_eq!(
            write_markdown(&document),
            "|  |  |\n| --- | --- |\n| A |  |\n| B |  |\n|  |  |"
        );
        assert!(crate::parser::parse_markdown(&write_markdown(&document)).events.iter().any(
            |event| matches!(event, MarkdownEvent::Start(crate::parser::MarkdownTag::Table(_)))
        ));
    }

    #[test]
    fn preserves_ordered_list_start_numbers() {
        let document = RichDocument::new(vec![RichBlock::List {
            kind: ListKind::Ordered { start: 4 },
            items: vec![
                vec![RichBlock::Paragraph(text("first"))],
                vec![RichBlock::Paragraph(text("second"))],
            ],
        }]);

        assert_eq!(write_markdown(&document), "4. first\n5. second");
    }

    #[test]
    fn omits_empty_structures_without_creating_blank_output() {
        let document = RichDocument::new(vec![
            RichBlock::Paragraph(Vec::new()),
            RichBlock::BlockQuote(Vec::new()),
            RichBlock::List { kind: ListKind::Unordered, items: Vec::new() },
            RichBlock::Table { header: Vec::new(), rows: Vec::new() },
            RichBlock::Heading { level: HeadingLevel::H1, content: Vec::new() },
        ]);

        assert_eq!(write_markdown(&document), "");
    }

    #[test]
    fn unsafe_link_and_image_destinations_fall_back_to_visible_text() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Link {
                destination: "javascript:alert(1)".into(),
                title: None,
                children: text("link"),
            },
            RichInline::Text(" ".into()),
            RichInline::RemoteImage {
                destination: "file:///private/image.png".into(),
                title: None,
                alt: "diagram".into(),
            },
        ])]);

        assert_eq!(write_markdown(&document), "link diagram");
    }

    #[test]
    fn mailto_links_are_written_while_mailto_images_remain_rejected() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Link {
                destination: "mailto:a@example.com".into(),
                title: None,
                children: text("mail"),
            },
            RichInline::Text(" ".into()),
            RichInline::RemoteImage {
                destination: "mailto:image@example.com".into(),
                title: None,
                alt: "diagram".into(),
            },
        ])]);

        assert_eq!(write_markdown(&document), "[mail](mailto:a@example.com) diagram");
    }

    fn representative_document() -> RichDocument {
        RichDocument::new(vec![
            RichBlock::Heading { level: HeadingLevel::H2, content: text("Heading") },
            RichBlock::List {
                kind: ListKind::Unordered,
                items: vec![vec![
                    RichBlock::Paragraph(text("parent")),
                    RichBlock::List {
                        kind: ListKind::Ordered { start: 1 },
                        items: vec![vec![RichBlock::Paragraph(text("child"))]],
                    },
                ]],
            },
            RichBlock::BlockQuote(vec![RichBlock::Paragraph(text("quoted"))]),
            RichBlock::Table { header: vec![text("Name")], rows: vec![vec![text("A")]] },
            RichBlock::CodeBlock { language: Some("rust".into()), text: "let x = 1;".into() },
            RichBlock::Paragraph(vec![RichInline::Link {
                destination: "https://example.com".into(),
                title: None,
                children: text("link"),
            }]),
        ])
    }

    #[test]
    fn representative_writer_output_reparses_without_visible_text_loss() {
        let document = representative_document();
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert!(!parsed.events.is_empty());
        for visible in ["Heading", "parent", "child", "quoted", "Name", "let x = 1", "link"] {
            assert!(markdown.contains(visible), "writer output lost {visible:?}: {markdown:?}");
        }
    }
}

use super::{HeadingLevel, ListKind, RichBlock, RichDocument, RichInline};

const TOP_LEVEL_BLOCK_SEPARATOR: &str = "\n\n";
const MARKDOWN_HARD_LINE_BREAK: &str = "  \n";
const FENCED_CODE_MINIMUM_DELIMITER_LENGTH: usize = 3;
const INLINE_CODE_MINIMUM_DELIMITER_LENGTH: usize = 1;
const LIST_ITEM_MARKER: &str = "- ";
const TABLE_CELL_SEPARATOR: &str = " | ";
const TABLE_BORDER: &str = "| ";
const TABLE_ROW_END: &str = " |";
const TABLE_DELIMITER_CELL: &str = "---";
const TABLE_LINE_BREAK: &str = "<br>";
const MAXIMUM_LIST_MARKER_INDENTATION: usize = 3;

#[derive(Clone, Copy)]
struct NestingContext {
    list_indentation: usize,
}

impl NestingContext {
    const TOP_LEVEL: Self = Self { list_indentation: 0 };

    fn nested_list(self, marker_width: usize) -> Self {
        Self { list_indentation: self.list_indentation + marker_width }
    }
}

#[derive(Clone, Copy)]
struct InlineContext {
    table_cell: bool,
}

impl InlineContext {
    const FLOW: Self = Self { table_cell: false };
    const TABLE_CELL: Self = Self { table_cell: true };
}

#[derive(Clone, Copy)]
struct InlineTextState {
    line: InlineLineState,
}

#[derive(Clone, Copy)]
enum InlineLineState {
    Start { indentation: usize },
    OrderedListNumber { digit_count: usize },
    Content,
    StructuralBoundary,
}

#[derive(Clone, Copy)]
enum InlineWriteOutcome {
    Preserved,
    Interrupted,
}

impl InlineWriteOutcome {
    fn interrupts_text_continuity(self) -> bool {
        matches!(self, Self::Interrupted)
    }
}

impl InlineTextState {
    fn from_output(output: &str) -> Self {
        let line = if output.is_empty() || output.ends_with('\n') {
            InlineLineState::Start { indentation: 0 }
        } else {
            InlineLineState::Content
        };
        Self { line }
    }

    fn is_line_structure_start(self) -> bool {
        matches!(self.line, InlineLineState::Start { .. } | InlineLineState::StructuralBoundary)
    }

    fn starts_ordered_list_marker(self, character: char, next_character: Option<char>) -> bool {
        matches!(
            self.line,
            InlineLineState::OrderedListNumber { digit_count } if digit_count > 0
        ) && matches!(character, '.' | ')')
            && next_character.is_some_and(char::is_whitespace)
    }

    fn advance_plain_text(&mut self, character: char) {
        self.line = match self.line {
            InlineLineState::Start { indentation }
                if character == ' ' && indentation < MAXIMUM_LIST_MARKER_INDENTATION =>
            {
                InlineLineState::Start { indentation: indentation + 1 }
            }
            InlineLineState::Start { .. } if character.is_ascii_digit() => {
                InlineLineState::OrderedListNumber { digit_count: 1 }
            }
            InlineLineState::OrderedListNumber { digit_count } if character.is_ascii_digit() => {
                InlineLineState::OrderedListNumber { digit_count: digit_count + 1 }
            }
            _ => InlineLineState::Content,
        };
    }

    fn reset_after_line_break(&mut self, context: InlineContext) {
        self.line = if context.table_cell {
            InlineLineState::StructuralBoundary
        } else {
            InlineLineState::Start { indentation: 0 }
        };
    }

    fn break_structural_continuity(&mut self) {
        self.line = InlineLineState::StructuralBoundary;
    }
}

pub(crate) fn write_markdown(document: &RichDocument) -> String {
    let mut output = String::with_capacity(document.blocks().len().saturating_mul(32));
    write_blocks(document.blocks(), &mut output, NestingContext::TOP_LEVEL);
    output
}

fn write_blocks(blocks: &[RichBlock], output: &mut String, nesting: NestingContext) {
    let mut wrote_block = false;

    for block in blocks {
        let mut block_output = String::new();
        write_block(block, &mut block_output, nesting);
        trim_block_boundary_line_breaks(&mut block_output);
        if block_output.is_empty() {
            continue;
        }

        if wrote_block {
            output.push_str(TOP_LEVEL_BLOCK_SEPARATOR);
        }
        output.push_str(&block_output);
        wrote_block = true;
    }
}

fn trim_block_boundary_line_breaks(output: &mut String) {
    while output.ends_with('\n') {
        output.pop();
    }
}

fn write_block(block: &RichBlock, output: &mut String, nesting: NestingContext) {
    match block {
        RichBlock::Heading { level, content } => {
            let mut heading = String::new();
            write_inlines(content, &mut heading, InlineContext::FLOW);
            if heading.is_empty() {
                return;
            }
            output.push_str(heading_marker(*level));
            output.push(' ');
            output.push_str(&heading);
        }
        RichBlock::Paragraph(content) => write_inlines(content, output, InlineContext::FLOW),
        RichBlock::BlockQuote(blocks) => write_block_quote(blocks, output, nesting),
        RichBlock::List { kind, items } => write_list(*kind, items, output, nesting),
        RichBlock::CodeBlock { language, text } => {
            write_code_block(language.as_deref(), text, output)
        }
        RichBlock::Table { header, rows } => write_table(header, rows, output),
        RichBlock::HorizontalRule => output.push_str("---"),
    }
}

fn write_block_quote(blocks: &[RichBlock], output: &mut String, nesting: NestingContext) {
    let mut quoted = String::new();
    write_blocks(blocks, &mut quoted, nesting);
    if quoted.is_empty() {
        return;
    }

    for (line_index, line) in quoted.split('\n').enumerate() {
        if line_index > 0 {
            output.push('\n');
        }
        output.push_str("> ");
        output.push_str(line);
    }
}

fn write_list(
    kind: ListKind,
    items: &[Vec<RichBlock>],
    output: &mut String,
    nesting: NestingContext,
) {
    let mut wrote_item = false;

    for (item_index, item) in items.iter().enumerate() {
        let marker = list_item_marker(kind, item_index);
        let item_nesting = nesting.nested_list(marker.len());
        let mut item_output = String::new();
        write_list_item(item, &marker, &mut item_output, item_nesting, nesting.list_indentation);
        if item_output.is_empty() {
            continue;
        }

        if wrote_item {
            output.push('\n');
        }
        output.push_str(&item_output);
        wrote_item = true;
    }
}

fn write_list_item(
    blocks: &[RichBlock],
    marker: &str,
    output: &mut String,
    item_nesting: NestingContext,
    item_indentation: usize,
) {
    let mut wrote_block = false;

    for block in blocks {
        let mut block_output = String::new();
        write_block(block, &mut block_output, item_nesting);
        if block_output.is_empty() {
            continue;
        }

        if !wrote_block {
            output.push_str(&" ".repeat(item_indentation));
            output.push_str(marker);
            if matches!(block, RichBlock::List { .. }) {
                output.push('\n');
                output.push_str(&block_output);
            } else {
                append_with_continuation_indent(
                    output,
                    &block_output,
                    item_nesting.list_indentation,
                );
            }
            wrote_block = true;
            continue;
        }

        if matches!(block, RichBlock::List { .. }) {
            output.push('\n');
            output.push_str(&block_output);
        } else {
            output.push_str(TOP_LEVEL_BLOCK_SEPARATOR);
            output.push_str(&" ".repeat(item_nesting.list_indentation));
            append_with_continuation_indent(output, &block_output, item_nesting.list_indentation);
        }
    }
}

fn append_with_continuation_indent(output: &mut String, text: &str, indentation: usize) {
    for (line_index, line) in text.split('\n').enumerate() {
        if line_index > 0 {
            output.push('\n');
            output.push_str(&" ".repeat(indentation));
        }
        output.push_str(line);
    }
}

fn list_item_marker(kind: ListKind, item_index: usize) -> String {
    match kind {
        ListKind::Unordered => LIST_ITEM_MARKER.into(),
        ListKind::Ordered { start } => format!("{}. ", start.saturating_add(item_index as u64)),
    }
}

fn write_table(header: &[Vec<RichInline>], rows: &[Vec<Vec<RichInline>>], output: &mut String) {
    let column_count =
        rows.iter().fold(header.len(), |maximum_width, row| maximum_width.max(row.len()));
    if column_count == 0 {
        return;
    }

    write_table_row_with_width(header, column_count, output);
    output.push('\n');
    write_table_delimiter_row(column_count, output);
    for row in rows {
        output.push('\n');
        write_table_row_with_width(row, column_count, output);
    }
}

fn write_table_row_with_width(cells: &[Vec<RichInline>], width: usize, output: &mut String) {
    output.push_str(TABLE_BORDER);
    for cell_index in 0..width {
        if cell_index > 0 {
            output.push_str(TABLE_CELL_SEPARATOR);
        }
        if let Some(cell) = cells.get(cell_index) {
            write_inlines(cell, output, InlineContext::TABLE_CELL);
        }
    }
    output.push_str(TABLE_ROW_END);
}

fn write_table_delimiter_row(column_count: usize, output: &mut String) {
    output.push_str(TABLE_BORDER);
    for column_index in 0..column_count {
        if column_index > 0 {
            output.push_str(TABLE_CELL_SEPARATOR);
        }
        output.push_str(TABLE_DELIMITER_CELL);
    }
    output.push_str(TABLE_ROW_END);
}

fn write_code_block(language: Option<&str>, text: &str, output: &mut String) {
    let delimiter_length =
        longest_backtick_run(text).saturating_add(1).max(FENCED_CODE_MINIMUM_DELIMITER_LENGTH);
    let delimiter = "`".repeat(delimiter_length);
    output.push_str(&delimiter);
    if let Some(language) = language
        && is_safe_fence_language(language)
    {
        output.push_str(language);
    }
    output.push('\n');
    output.push_str(text);
    if !text.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&delimiter);
}

fn is_safe_fence_language(language: &str) -> bool {
    !language.is_empty()
        && language.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '+' | '-')
        })
}

fn write_inlines(inlines: &[RichInline], output: &mut String, context: InlineContext) {
    let mut text_state = InlineTextState::from_output(output);
    write_inlines_with_state(inlines, output, context, &mut text_state);
}

fn write_inlines_with_state(
    inlines: &[RichInline],
    output: &mut String,
    context: InlineContext,
    text_state: &mut InlineTextState,
) {
    for inline in inlines {
        write_inline(inline, output, context, text_state);
    }
}

fn write_inline(
    inline: &RichInline,
    output: &mut String,
    context: InlineContext,
    text_state: &mut InlineTextState,
) {
    let outcome = match inline {
        RichInline::Text(text) => {
            write_plain_text(text, output, context, text_state);
            return;
        }
        RichInline::Strong(children) => write_styled_inlines(children, "**", output, context),
        RichInline::Emphasis(children) => write_styled_inlines(children, "*", output, context),
        RichInline::Strikethrough(children) => {
            write_styled_inlines(children, "~~", output, context)
        }
        RichInline::InlineCode(text) => write_inline_code(text, output),
        RichInline::Link { destination, title, children } => {
            write_link(destination, title.as_deref(), children, output, context, text_state)
        }
        RichInline::RemoteImage { destination, title, alt } => {
            write_remote_image(destination, title.as_deref(), alt, output, context, text_state)
        }
        RichInline::LineBreak => {
            write_explicit_line_break(output, context, text_state);
            return;
        }
    };

    if outcome.interrupts_text_continuity() {
        text_state.break_structural_continuity();
    }
}

fn write_styled_inlines(
    children: &[RichInline],
    marker: &str,
    output: &mut String,
    context: InlineContext,
) -> InlineWriteOutcome {
    let mut child_output = String::new();
    write_inlines(children, &mut child_output, context);
    if child_output.is_empty() {
        return InlineWriteOutcome::Preserved;
    }
    output.push_str(marker);
    output.push_str(&child_output);
    output.push_str(marker);
    InlineWriteOutcome::Interrupted
}

fn write_plain_text(
    text: &str,
    output: &mut String,
    context: InlineContext,
    state: &mut InlineTextState,
) {
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        if matches!(character, '\r' | '\n') {
            consume_line_break(&mut characters, character);
            write_source_line_break(output, context, state);
            continue;
        }

        write_text_character(character, characters.peek().copied(), output, context, state);
    }
}

fn consume_line_break(characters: &mut std::iter::Peekable<std::str::Chars<'_>>, character: char) {
    if character == '\r' && matches!(characters.peek(), Some('\n')) {
        characters.next();
    }
}

fn write_source_line_break(
    output: &mut String,
    context: InlineContext,
    state: &mut InlineTextState,
) {
    if context.table_cell {
        output.push_str(TABLE_LINE_BREAK);
    } else {
        output.push('\n');
    }
    state.reset_after_line_break(context);
}

fn write_explicit_line_break(
    output: &mut String,
    context: InlineContext,
    state: &mut InlineTextState,
) {
    if context.table_cell {
        output.push_str(TABLE_LINE_BREAK);
    } else {
        output.push_str(MARKDOWN_HARD_LINE_BREAK);
    }
    state.reset_after_line_break(context);
}

fn write_text_character(
    character: char,
    next_character: Option<char>,
    output: &mut String,
    context: InlineContext,
    state: &mut InlineTextState,
) {
    if should_escape_text_character(character, next_character, context, *state) {
        output.push('\\');
    }
    output.push(character);
    state.advance_plain_text(character);
}

fn should_escape_text_character(
    character: char,
    next_character: Option<char>,
    context: InlineContext,
    state: InlineTextState,
) -> bool {
    match character {
        '\\' | '`' | '*' | '_' | '[' | ']' | '!' | '~' => true,
        '|' => context.table_cell,
        '.' | ')' => state.starts_ordered_list_marker(character, next_character),
        '=' | '#' | '+' | '-' | '>' => state.is_line_structure_start(),
        _ => false,
    }
}

fn write_inline_code(text: &str, output: &mut String) -> InlineWriteOutcome {
    if text.is_empty() {
        return InlineWriteOutcome::Preserved;
    }

    let delimiter_length =
        longest_backtick_run(text).saturating_add(1).max(INLINE_CODE_MINIMUM_DELIMITER_LENGTH);
    let delimiter = "`".repeat(delimiter_length);
    let needs_boundary_padding = inline_code_needs_boundary_padding(text);
    output.push_str(&delimiter);
    if needs_boundary_padding {
        output.push(' ');
    }
    output.push_str(text);
    if needs_boundary_padding {
        output.push(' ');
    }
    output.push_str(&delimiter);
    InlineWriteOutcome::Interrupted
}

fn inline_code_needs_boundary_padding(text: &str) -> bool {
    if text.is_empty() || text.chars().all(char::is_whitespace) {
        return false;
    }

    let first_character = text.chars().next();
    let last_character = text.chars().next_back();
    matches!(first_character, Some('`' | ' ')) || matches!(last_character, Some('`' | ' '))
}

fn write_link(
    destination: &str,
    title: Option<&str>,
    children: &[RichInline],
    output: &mut String,
    context: InlineContext,
    text_state: &mut InlineTextState,
) -> InlineWriteOutcome {
    if !is_safe_link_url(destination) {
        write_inlines_with_state(children, output, context, text_state);
        return InlineWriteOutcome::Preserved;
    }

    let mut label = String::new();
    write_inlines(children, &mut label, context);
    output.push('[');
    output.push_str(&label);
    output.push_str("](");
    write_destination(destination, output);
    write_title(title, output);
    output.push(')');
    InlineWriteOutcome::Interrupted
}

fn write_remote_image(
    destination: &str,
    title: Option<&str>,
    alt: &str,
    output: &mut String,
    context: InlineContext,
    state: &mut InlineTextState,
) -> InlineWriteOutcome {
    if !is_safe_remote_image_url(destination) {
        write_plain_text(alt, output, context, state);
        return InlineWriteOutcome::Preserved;
    }

    let mut alt_output = String::new();
    write_inlines(&[RichInline::Text(alt.into())], &mut alt_output, context);
    output.push_str("![");
    output.push_str(&alt_output);
    output.push_str("](");
    write_destination(destination, output);
    write_title(title, output);
    output.push(')');
    InlineWriteOutcome::Interrupted
}

fn is_safe_link_url(destination: &str) -> bool {
    url::Url::parse(destination)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

fn is_safe_remote_image_url(destination: &str) -> bool {
    url::Url::parse(destination).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn write_destination(destination: &str, output: &mut String) {
    if destination
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '(' | ')'))
    {
        output.push('<');
        output.push_str(destination);
        output.push('>');
    } else {
        output.push_str(destination);
    }
}

fn write_title(title: Option<&str>, output: &mut String) {
    let Some(title) = title.filter(|title| !title.is_empty()) else {
        return;
    };

    output.push_str(" \"");
    let mut characters = title.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\\' | '\"' => {
                output.push('\\');
                output.push(character);
            }
            '\r' => {
                if matches!(characters.peek(), Some('\n')) {
                    characters.next();
                }
                output.push(' ');
            }
            '\n' => output.push(' '),
            _ => output.push(character),
        }
    }
    output.push('\"');
}

fn heading_marker(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "#",
        HeadingLevel::H2 => "##",
        HeadingLevel::H3 => "###",
        HeadingLevel::H4 => "####",
        HeadingLevel::H5 => "#####",
        HeadingLevel::H6 => "######",
    }
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest_run = 0;
    let mut current_run = 0;

    for character in text.chars() {
        if character == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }

    longest_run
}
