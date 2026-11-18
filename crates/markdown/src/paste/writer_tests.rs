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
    let document =
        RichDocument::new(vec![RichBlock::Paragraph(text("release 1.2 (stable) | C# and e-mail"))]);

    assert_eq!(write_markdown(&document), "release 1.2 (stable) | C# and e-mail");
}

#[test]
fn rich_text_reparses_literal_html_and_entities_as_text() {
    let documents = [
        RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Strong(text(
            "<br> &copy;",
        ))])]),
        RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Link {
            destination: "https://example.com".into(),
            title: None,
            children: text("<span> &amp;"),
        }])]),
        RichDocument::new(vec![RichBlock::Table {
            header: vec![text("<head>")],
            rows: vec![vec![text("&copy;")]],
        }]),
    ];
    let expected_text = ["<br> &copy;", "<span> &amp;", "<head>&copy;"];

    for (document, expected) in documents.into_iter().zip(expected_text) {
        let markdown = write_markdown(&document);
        let reparsed_text: String = crate::parser::parse_markdown(&markdown)
            .events
            .into_iter()
            .filter_map(|event| match event {
                MarkdownEvent::Text(text) => Some(text),
                _ => None,
            })
            .collect();

        assert_eq!(reparsed_text, expected, "generated Markdown: {markdown:?}");
    }
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
    let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Strikethrough(
        text("gone"),
    )])]);
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

    assert_eq!(write_markdown(&document), "## Heading\n\n> first  \n> second\n> \n> - item\n\n---");
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
                RichBlock::Paragraph(vec![RichInline::Text("first".into()), RichInline::LineBreak]),
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
    assert!(
        parsed.events.iter().all(|event| !matches!(
            event,
            MarkdownEvent::Start(crate::parser::MarkdownTag::List(..))
        ))
    );
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
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::InlineCode(
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
    let document =
        RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::InlineCode(String::new())])]);

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

    assert_eq!(write_markdown(&document), "|  |  |\n| --- | --- |\n| A |  |\n| B |  |\n|  |  |");
    assert!(
        crate::parser::parse_markdown(&write_markdown(&document)).events.iter().any(
            |event| matches!(event, MarkdownEvent::Start(crate::parser::MarkdownTag::Table(_)))
        )
    );
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
