use super::*;

#[cfg(feature = "rich-markdown")]
#[test]
fn inline_math_is_one_projected_object_with_full_source_range() {
    let source = "中$x^2$文";
    let markdown = MarkdownDoc::build(
        &crate::parser::parse_markdown(source),
        &crate::test_utils::default_style(),
    );
    let paragraph = &markdown.blocks[0];
    assert_eq!(paragraph.text_lines, vec!["中\u{FFFC}文"]);
    let math = paragraph.text_styles[0]
        .iter()
        .find(|span| matches!(span.style, InlineStyle::Math))
        .expect("math span");
    assert_eq!(math.len, '\u{FFFC}'.len_utf8());
    assert_eq!(&source[math.source_range.clone()], "$x^2$");
}

#[cfg(feature = "rich-markdown")]
#[test]
fn same_line_display_math_splits_surrounding_paragraph() {
    let source = "before $$x^2$$ after";
    let markdown = MarkdownDoc::build(
        &crate::parser::parse_markdown(source),
        &crate::test_utils::default_style(),
    );
    assert_eq!(markdown.blocks.len(), 3);
    assert!(matches!(markdown.blocks[0].kind, BlockKind::Paragraph));
    assert!(matches!(markdown.blocks[1].kind, BlockKind::DisplayMath));
    assert!(matches!(markdown.blocks[2].kind, BlockKind::Paragraph));
    assert_eq!(&source[markdown.blocks[1].block_range.clone()], "$$x^2$$");
}
use crate::layout::BlockSource;
use crate::parser::parse_markdown;

use crate::test_utils::default_style;

#[test]
fn build_simple_paragraph() {
    let parsed = parse_markdown("hello world");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    // Should have a Paragraph block with text
    let has_paragraph = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Paragraph));
    assert!(has_paragraph, "should have a Paragraph block");
    // Check that text content is stored
    let string_doc = textora_core::document::StringDocView::new("hello world");
    let text: Vec<std::borrow::Cow<'_, str>> =
        doc.blocks.iter().flat_map(|b| b.lines(&string_doc)).collect();
    assert!(
        text.iter().any(|t| t.contains("hello world")),
        "text 'hello world' should be extracted via doc"
    );
}

#[test]
fn build_heading() {
    let parsed = parse_markdown("# Hello");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    assert!(!doc.blocks.is_empty());
    assert!(matches!(doc.blocks[0].kind, BlockKind::Heading { level: 1 }));
}

#[test]
fn build_bold_text() {
    let parsed = parse_markdown("**bold text**");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    // Should have a paragraph with bold text content
    let has_paragraph = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Paragraph));
    assert!(has_paragraph, "should have a Paragraph block for bold text");
}

#[test]
fn build_code_block() {
    let parsed = parse_markdown("```rust\nlet x = 1;\n```");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    let has_code = doc
        .blocks
        .iter()
        .any(|b| matches!(b.kind, BlockKind::CodeBlock { language: Some(ref l) } if l == "rust"));
    assert!(has_code);
}

#[test]
fn build_unordered_list() {
    let parsed = parse_markdown("- one\n- two\n- three");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    // Should have at least 3 ListItem blocks (nested under Container)
    fn count_items(blocks: &[BlockNode]) -> usize {
        let mut count = 0;
        for b in blocks {
            if matches!(b.kind, BlockKind::ListItem { .. }) {
                count += 1;
            }
            count += count_items(&b.children);
        }
        count
    }
    assert_eq!(count_items(&doc.blocks), 3);
}

#[test]
fn build_ordered_list_preserves_source_start_for_preview() {
    for (source, expected_numbers) in [
        ("4. one\n5. two\n6. three", vec![4, 5, 6]),
        ("0. one\n1. two\n2. three", vec![0, 1, 2]),
        ("42) one\n1) two\n9) three", vec![42, 43, 44]),
        ("1. one\n1. two\n1. three", vec![1, 2, 3]),
        ("1. one\n\nparagraph\n\n3. three\n4. four", vec![1, 3, 4]),
    ] {
        let parsed = parse_markdown(source);
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let rendered_numbers = doc
            .blocks
            .iter()
            .filter_map(|block| match block.kind {
                BlockKind::ListItem { bullet: ListBullet::Ordered(number), .. } => Some(number),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered_numbers, expected_numbers, "source: {source:?}");
    }
}

#[test]
fn build_nested_ordered_lists_preserve_independent_source_starts() {
    let parsed = parse_markdown("3. parent\n\n   7. child\n   1. child\n\n1. parent");
    let doc = MarkdownDoc::build(&parsed, &default_style());

    fn ordered_numbers(blocks: &[BlockNode]) -> Vec<u64> {
        let mut numbers = Vec::new();
        for block in blocks {
            if let BlockKind::ListItem { bullet: ListBullet::Ordered(number), .. } = block.kind {
                numbers.push(number);
            }
            numbers.extend(ordered_numbers(&block.children));
        }
        numbers
    }

    assert_eq!(ordered_numbers(&doc.blocks), [3, 7, 8, 4]);
}

#[test]
fn build_table() {
    let md = "| A | B |\n|---|---|\n| 1 | 2 |";
    let parsed = parse_markdown(md);
    let doc = MarkdownDoc::build(&parsed, &default_style());
    let has_table = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::TableWrapper { .. }));
    assert!(has_table);
}

#[test]
fn build_link() {
    let parsed = parse_markdown("[example](https://example.com)");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    // Should have a paragraph block with link text
    let has_paragraph = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Paragraph));
    assert!(has_paragraph, "should have a Paragraph block for link");
}

#[test]
fn build_blockquote() {
    let parsed = parse_markdown("> quoted text");
    let doc = MarkdownDoc::build(&parsed, &default_style());
    let has_bq = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::BlockQuote));
    assert!(has_bq);
}

#[test]
fn builder_collapses_explicit_blockquote_softbreak_and_preserves_source_jump() {
    let source = "> first\n> second";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let paragraph = &doc.blocks[0].children[0];
    assert_eq!(paragraph.projected_lines.len(), 1);
    assert_eq!(paragraph.projected_lines[0].text, "first second");
    assert_eq!(
        paragraph.projected_lines[0].boundaries.last().expect("collapsed line boundary").byte.get(),
        source.len()
    );
    assert!(paragraph.projected_lines[0].boundaries.iter().any(|boundary| {
        boundary.byte.get() == source.find("second").expect("fixture must contain second")
    }));
    assert!(paragraph.projected_lines[0].boundaries.iter().any(|boundary| {
        boundary.byte.get() == source.find('\n').expect("fixture must contain newline")
    }));
    assert_eq!(
        paragraph.projected_lines[0].source_extent().end,
        source.find("second").expect("fixture must contain second") + "second".len()
    );
}

#[test]
fn builder_collapses_each_explicit_blockquote_softbreak() {
    let source = "> 日期：2026-07-20\n> 状态：待评审\n> 目标：加载 Wiki";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let paragraph = &doc.blocks[0].children[0];

    assert_eq!(paragraph.text_lines, ["日期：2026-07-20 状态：待评审 目标：加载 Wiki"]);
}

#[test]
fn builder_keeps_lazy_blockquote_continuation_as_one_line() {
    let source = "> first\nsecond";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let paragraph = &doc.blocks[0].children[0];

    assert_eq!(paragraph.text_lines, ["first second"]);
}

#[test]
fn builder_preserves_hard_break_lines() {
    let source = "first\\\nsecond";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());

    assert_eq!(doc.blocks[0].text_lines, ["first", "second"]);
}

#[test]
fn builder_renders_inline_html_break_as_a_hard_break() {
    for html_break in ["<br>", "<br/>", "<br />", "<BR            />"] {
        let source = format!("# first{html_break}second");
        let doc = MarkdownDoc::build(&parse_markdown(&source), &default_style());

        assert_eq!(
            doc.blocks[0].text_lines,
            ["first", "second"],
            "inline HTML break {html_break:?} must create two visual lines"
        );
    }
}

#[test]
fn builder_keeps_inline_style_across_an_html_break() {
    let source = "前**before<br            />after**后";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let paragraph = &doc.blocks[0];

    assert_eq!(paragraph.text_lines, ["前before", "after后"]);
    for (line, bold) in paragraph.text_styles.iter().zip(["before", "after"]) {
        let span = line
            .iter()
            .find(|span| span.style == InlineStyle::Bold)
            .expect("each line must retain its bold span");
        assert_eq!(span.len, bold.len());
    }
    for line in &paragraph.projected_lines {
        line.validate(source).expect("both projections must respect UTF-8 boundaries");
    }
}

#[test]
fn builder_keeps_non_break_html_and_code_literal() {
    for (source, expected) in [
        ("before<br class=\"x\">after", vec!["before<br class=\"x\">after"]),
        ("before`<br>`after", vec!["before<br>after"]),
        ("<div>\n<br>\n</div>", vec!["<div>", "<br>", "</div>"]),
    ] {
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
        assert_eq!(doc.blocks[0].text_lines, expected, "source={source:?}");
    }
}

#[test]
fn autolink_projection_preserves_technical_text_but_not_prose_labels() {
    let source = "前文<https://a.test/中文A>后文";
    let document = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let projected = &document.blocks[0].projected_lines[0];
    assert!(projected.spans.iter().any(|span| {
        span.kind == crate::projection::ProjectionSpanKind::Verbatim
            && &projected.text[span.visual_range.clone()] == "https://a.test/中文A"
    }));
    let source = "前文[中文A](https://a.test)后文";
    let document = MarkdownDoc::build(&parse_markdown(source), &default_style());
    assert!(
        document.blocks[0].projected_lines[0]
            .spans
            .iter()
            .all(|span| { span.kind != crate::projection::ProjectionSpanKind::Verbatim })
    );
}

#[test]
fn html_projection_preserves_literal_and_prose_boundaries() {
    let html = "<span title=\"中A\">";
    let source = format!("前文{html}后文");
    let document = MarkdownDoc::build(&parse_markdown(&source), &default_style());
    let projected = &document.blocks[0].projected_lines[0];
    let html_start = "前文".len();
    let html_end = html_start + html.len();
    assert_eq!(projected.text, source);
    assert!(
        projected.spans.iter().any(|span| {
            span.source_range == (html_start..html_end)
                && span.visual_range == (html_start..html_end)
        }),
        "HTML must retain a distinct source-to-visual span for spacing protection"
    );
    projected.validate(&source).expect("literal HTML must retain valid cursor boundaries");
}

#[test]
fn builder_keeps_non_break_inline_html_as_literal_text() {
    let source = "# first<span>second</span>";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());

    assert_eq!(doc.blocks[0].text_lines, ["first<span>second</span>"]);
}

#[test]
fn builder_nested_list_parent_projection_ends_at_parent_content() {
    let source = "- outer\n  - inner wrapped content";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let parent = doc.blocks.first().expect("fixture must produce a parent list item");
    let projection =
        parent.projected_lines.first().expect("parent list item must retain its text projection");
    let parent_content_end = source.find('\n').expect("fixture must contain nested list line");

    assert_eq!(projection.spans[0].source_range.end, parent_content_end);
    assert_eq!(
        projection.boundaries.last().expect("projection needs a terminal anchor").byte.get(),
        parent_content_end,
        "parent terminal anchor must not include nested child source"
    );
}

#[test]
fn builder_plain_softbreak_maps_newline_to_one_visual_space() {
    let source = "first\nsecond";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let projected = &doc.blocks[0].projected_lines[0];
    assert_eq!(projected.text, "first second");
    assert_eq!(projected.boundaries[6].byte.get(), 6);
}

#[test]
fn code_line_source_starts_follow_parser_content_after_indentation() {
    for (opening, indentation, closing) in
        [("", "    ", ""), ("", "\t", ""), ("> ```\n", "> ", "> ```\n"), ("```\n", "", "```\n")]
    {
        for newline in ["\n", "\r\n"] {
            let source = format!("{opening}{indentation}第一行\n{indentation}第二行\n{closing}")
                .replace('\n', newline);
            let document = MarkdownDoc::build(&parse_markdown(&source), &default_style());
            let block = document.blocks.first().expect("fixture contains a code container");
            let code = if matches!(block.kind, BlockKind::BlockQuote) {
                block.children.first().expect("quote contains a code block")
            } else {
                block
            };
            let starts = code
                .code_line_source_starts
                .as_ref()
                .expect("code content must retain source line starts");
            for (line_index, content) in ["第一行", "第二行"].into_iter().enumerate() {
                assert_eq!(
                    starts[line_index],
                    source.find(content).expect("fixture contains code content"),
                    "code content start must skip {indentation:?} with {newline:?}",
                );
            }
        }
    }
}

#[test]
fn builder_trims_fenced_code_projection_trailing_newline() {
    let source = "```\nhello\n```";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let code_block = &doc.blocks[0];
    let projected = &code_block.projected_lines[0];

    assert_eq!(code_block.text_lines, ["hello"]);
    assert_eq!(projected.text, "hello");
    assert_eq!(
        projected.boundaries.last().expect("projection needs a sentinel").byte.get(),
        source.find("\n```").expect("fixture must contain closing fence")
    );
}

#[test]
fn builder_empty_heading_has_an_empty_projection_line() {
    let source = "#";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let heading = &doc.blocks[0];

    assert_eq!(heading.text_lines, [""]);
    assert_eq!(heading.projected_lines.len(), heading.text_lines.len());
    assert_eq!(heading.projected_lines[0].text, "");
    assert_eq!(heading.projected_lines[0].boundaries[0].byte.get(), source.len());
}

#[test]
fn build_horizontal_rule() {
    // "---" after text is still a horizontal rule
    let parsed = parse_markdown(
        "text

---",
    );
    let doc = MarkdownDoc::build(&parsed, &default_style());
    let has_rule = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::HorizontalRule));
    assert!(has_rule);
}

#[test]
fn build_yaml_metadata_block() {
    let parsed = parse_markdown(
        "---
title: hello
---",
    );
    let doc = MarkdownDoc::build(&parsed, &default_style());
    let has_metadata = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::MetadataBlock));
    assert!(has_metadata, "YAML --- at document start should produce MetadataBlock");
}

#[test]
fn source_range_captures_bold_marker() {
    let src = "hello **world** here";
    let parsed = parse_markdown(src);
    let doc = MarkdownDoc::build(&parsed, &default_style());

    // 找到 Bold span
    let bold_span = doc
        .blocks
        .iter()
        .flat_map(|b| b.text_styles.iter().flatten())
        .find(|s| s.style == InlineStyle::Bold)
        .expect("should have a bold span");

    // source_range 应包含 ** 标记: "**world**" 在 src[6..15]
    assert_eq!(bold_span.source_range, 6..15);
    assert_eq!(&src[bold_span.source_range.clone()], "**world**");
}

#[test]
fn source_range_captures_italic_marker() {
    let src = "hello *world* here";
    let parsed = parse_markdown(src);
    let doc = MarkdownDoc::build(&parsed, &default_style());

    let italic_span = doc
        .blocks
        .iter()
        .flat_map(|b| b.text_styles.iter().flatten())
        .find(|s| s.style == InlineStyle::Italic)
        .expect("should have an italic span");

    // "*world*" 在 src[6..13]
    assert_eq!(italic_span.source_range, 6..13);
    assert_eq!(&src[italic_span.source_range.clone()], "*world*");
}

#[test]
fn source_range_inline_code() {
    let src = "use `println!` here";
    let parsed = parse_markdown(src);
    let doc = MarkdownDoc::build(&parsed, &default_style());

    let code_span = doc
        .blocks
        .iter()
        .flat_map(|b| b.text_styles.iter().flatten())
        .find(|s| s.style == InlineStyle::InlineCode)
        .expect("should have an inline code span");

    // "`println!`" 在 src[4..14]
    assert_eq!(code_span.source_range, 4..14);
    assert_eq!(&src[code_span.source_range.clone()], "`println!`");
}

#[test]
fn plain_text_has_no_style_spans() {
    let src = "plain text";
    let parsed = parse_markdown(src);
    let doc = MarkdownDoc::build(&parsed, &default_style());

    let all_spans: Vec<&StyleSpan> =
        doc.blocks.iter().flat_map(|b| b.text_styles.iter().flatten()).collect();
    assert!(all_spans.is_empty(), "plain text should produce no style spans");
}

#[test]
fn block_source_range_covers_paragraph() {
    let src = "hello world";
    let parsed = parse_markdown(src);
    let doc = MarkdownDoc::build(&parsed, &default_style());

    let para = doc
        .blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Paragraph))
        .expect("should have a Paragraph block");

    // 整段文本在 src[0..11]
    assert_eq!(para.block_range, 0..11);
    assert_eq!(&src[para.block_range.clone()], "hello world");
}

#[test]
fn block_source_range_covers_heading() {
    let src = "# Title";
    let parsed = parse_markdown(src);
    let doc = MarkdownDoc::build(&parsed, &default_style());

    let heading = doc
        .blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Heading { level: 1 }))
        .expect("should have a Heading block");

    assert_eq!(heading.block_range, 0..7);
    assert_eq!(&src[heading.block_range.clone()], "# Title");
}

// ===== NovelStructure 测试 =====

fn novel_scan(text: &str) -> NovelStructure {
    NovelStructure::scan(&textora_core::document::StringDocView::new(text))
}

#[test]
fn novel_scan_empty_input() {
    let ns = novel_scan("");
    assert!(ns.blocks().is_empty());
    assert!(ns.headings().is_empty());
}

#[test]
fn novel_scan_single_paragraph() {
    let ns = novel_scan("hello world");
    assert_eq!(ns.blocks().len(), 1);
    assert!(matches!(ns.blocks()[0].kind, BlockKind::Paragraph));
}

#[test]
fn novel_scan_heading_h1() {
    let ns = novel_scan("第1章 开始");
    assert_eq!(ns.blocks().len(), 1);
    assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 2 }));
}

#[test]
fn novel_scan_heading_book_title() {
    let ns = novel_scan("《我的小说》 作者：某某");
    assert_eq!(ns.blocks().len(), 1);
    assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 1 }));
}

#[test]
fn novel_scan_special_marker() {
    let ns = novel_scan("楔子");
    assert_eq!(ns.blocks().len(), 1);
    assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 2 }));
}

#[test]
fn novel_scan_multiple_paragraphs() {
    let text = "第一段文字。\n\n第二段文字！\n\n第三段文字";
    let ns = novel_scan(text);
    assert_eq!(ns.blocks().len(), 3);
    for block in ns.blocks() {
        assert!(matches!(block.kind, BlockKind::Paragraph));
    }
}

#[test]
fn novel_scan_heading_and_paragraph() {
    let text = "第1章 序\n\n这是正文内容。";
    let ns = novel_scan(text);
    assert_eq!(ns.blocks().len(), 2);
    assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 2 }));
    assert!(matches!(ns.blocks()[1].kind, BlockKind::Paragraph));
}

#[test]
fn novel_scan_headings_populated() {
    let text = "《书名》\n\n第1章 开始\n\n正文";
    let ns = novel_scan(text);
    assert_eq!(ns.headings().len(), 2);
    assert_eq!(ns.headings()[0].text, "《书名》");
    assert_eq!(ns.headings()[0].level, 1);
    assert_eq!(ns.headings()[1].text, "第1章 开始");
    assert_eq!(ns.headings()[1].level, 2);
}

#[test]
fn novel_scan_body_text_not_heading() {
    let text = "这是一段普通的中文正文，不应被识别为标题。";
    let ns = novel_scan(text);
    assert_eq!(ns.blocks().len(), 1);
    assert!(matches!(ns.blocks()[0].kind, BlockKind::Paragraph));
}

#[test]
fn novel_scan_paragraph_split_by_end_chars() {
    // 句号、感叹号、问号等结束符作为行末字符时产生段落分割
    let text = "第一句话。\n第二句话！\n第三句话？";
    let ns = novel_scan(text);
    assert_eq!(ns.blocks().len(), 3);
    for block in ns.blocks() {
        assert!(matches!(block.kind, BlockKind::Paragraph));
    }
}

#[test]
fn novel_scan_classifies_blocks_and_preserves_source_ranges() {
    let title = "《小说》";
    let first_chapter = "第1章 序";
    let first_paragraph = "这是第一节内容。\n";
    let continuation = "还有更多。\n";
    let second_chapter = "第2章 开篇";
    let second_paragraph = "第二节的内容！";
    let source = format!(
        "{title}\n\n{first_chapter}\n\n{first_paragraph}{continuation}\n{second_chapter}\n\n{second_paragraph}"
    );
    let scanned = novel_scan(&source);
    let expected = [
        (BlockKind::Heading { level: 1 }, title),
        (BlockKind::Heading { level: 2 }, first_chapter),
        (BlockKind::Paragraph, first_paragraph),
        (BlockKind::Paragraph, continuation),
        (BlockKind::Heading { level: 2 }, second_chapter),
        (BlockKind::Paragraph, second_paragraph),
    ];
    assert_eq!(scanned.blocks().len(), expected.len());
    for (block, (kind, text)) in scanned.blocks().iter().zip(expected) {
        assert_eq!(block.kind, kind);
        assert_eq!(&source[block.block_range.clone()], text);
    }
}

#[test]
fn novel_scan_implements_block_source() {
    let text = "第1章 标题\n\n正文内容。";
    let ns = novel_scan(text);
    // 通过 BlockSource trait 访问
    let blocks: &[BlockNode] = crate::layout::BlockSource::blocks(&ns);
    let headings: &[crate::layout::HeadingEntry] = crate::layout::BlockSource::headings(&ns);
    assert_eq!(blocks.len(), 2);
    assert_eq!(headings.len(), 1);
}

#[test]
fn top_level_html_block_is_preserved_for_rendering_and_editing() {
    let single = "<div>hello</div>";
    let single_doc = MarkdownDoc::build(&parse_markdown(single), &default_style());
    assert_eq!(single_doc.blocks.len(), 1);
    assert_eq!(single_doc.blocks[0].text_lines, vec!["<div>hello</div>"]);
    single_doc.blocks[0].projected_lines[0].validate(single).expect("valid html block projection");

    let followed = "<div>hello</div>\n\npara";
    let followed_doc = MarkdownDoc::build(&parse_markdown(followed), &default_style());
    assert_eq!(followed_doc.blocks.len(), 2);
    assert_eq!(followed_doc.blocks[0].text_lines, vec!["<div>hello</div>"]);
    assert_eq!(followed_doc.blocks[1].text_lines, vec!["para"]);

    let multiline = "<div>\n  line\n</div>\n\npara";
    let multiline_doc = MarkdownDoc::build(&parse_markdown(multiline), &default_style());
    assert_eq!(multiline_doc.blocks.len(), 2);
    assert_eq!(multiline_doc.blocks[0].text_lines, vec!["<div>", "  line", "</div>"]);
    for projected in &multiline_doc.blocks[0].projected_lines {
        projected.validate(multiline).expect("multiline html projection must be valid");
    }
}

#[test]
fn image_span_expands_full_markdown_source_when_focused() {
    let source = "before ![alt](img.png) after";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let block = &doc.blocks[0];
    let folded = crate::edit::materialize_projected_line(
        &block.projected_lines[0],
        &block.text_styles[0],
        source,
        None,
        None,
    );
    assert_eq!(folded.text, "before [Image: img.png] after");

    let edit_ctx = crate::edit::EditContext {
        cursor_byte: source.find("alt").expect("fixture contains image alt"),
        preedit_text: None,
        preedit_cursor: None,
    };
    let expanded = crate::edit::materialize_projected_line(
        &block.projected_lines[0],
        &block.text_styles[0],
        source,
        Some(&edit_ctx),
        None,
    );
    assert_eq!(expanded.text, source);
    expanded.validate(source).expect("expanded image projection must be valid");
}

#[test]
fn nested_inline_styles_expand_outer_and_inner_delimiters() {
    for (source, cursor_needle) in [
        ("**_text_**", "text"),
        ("***text***", "text"),
        ("**[link](https://x.test)**", "link"),
        ("[**bold**](https://x.test)", "bold"),
        ("**outer _inner_ rest**", "inner"),
        ("**outer _inner_ rest**", "outer"),
        ("**outer _inner_ rest**", "rest"),
        ("**outer _inner_**", "outer"),
        ("**outer _inner_**", "inner"),
        ("**_first_ second**", "first"),
        ("**_first_ second**", "second"),
        ("**bold `code`**", "code"),
    ] {
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
        let block = &doc.blocks[0];
        let base = &block.projected_lines[0];
        let styles = &block.text_styles[0];
        let ctx = crate::edit::EditContext {
            cursor_byte: source.find(cursor_needle).expect("fixture contains cursor needle"),
            preedit_text: None,
            preedit_cursor: None,
        };
        let expanded =
            crate::edit::materialize_projected_line(base, styles, source, Some(&ctx), None);
        assert_eq!(
            expanded.text, source,
            "nested delimiters must expand cleanly for {source:?} at {cursor_needle:?}"
        );
        expanded.validate(source).expect("nested expanded projection must validate");
        assert!(
            expanded.boundaries.windows(2).all(|pair| pair[0].byte.get() <= pair[1].byte.get()),
            "nested expanded boundaries must remain non-decreasing for {source:?}"
        );
    }

    // Sibling non-nested spans touching at the boundary must stay independent.
    let sibling_source = "`ab`**cd**";
    let sibling_doc = MarkdownDoc::build(&parse_markdown(sibling_source), &default_style());
    let block = &sibling_doc.blocks[0];
    let ctx = crate::edit::EditContext {
        cursor_byte: sibling_source.find("cd").expect("fixture has cd"),
        preedit_text: None,
        preedit_cursor: None,
    };
    let expanded = crate::edit::materialize_projected_line(
        &block.projected_lines[0],
        &block.text_styles[0],
        sibling_source,
        Some(&ctx),
        None,
    );
    assert_eq!(expanded.text, "ab**cd**");
}

#[test]
fn soft_break_before_hard_break_or_br_keeps_text_and_projection_in_sync() {
    for source in ["hello\n<br>\nworld", "hello\n\\\nworld"] {
        let parsed = parse_markdown(source);
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let block = &doc.blocks[0];
        assert_eq!(
            block.text_lines.len(),
            block.projected_lines.len(),
            "line count mismatch for {source:?}"
        );
        for (line_idx, (text_line, projected)) in
            block.text_lines.iter().zip(&block.projected_lines).enumerate()
        {
            assert_eq!(
                projected.text, *text_line,
                "line {line_idx} text/projection mismatch for {source:?}"
            );
            projected
                .validate(source)
                .expect("soft break before hard break must retain valid projection");
        }
    }
}

#[test]
fn tight_list_item_preserves_inline_html_break_as_logical_line_break() {
    for br in ["<br>", "<BR/>", "<br />"] {
        let source = format!("- first{br}second");
        let doc = MarkdownDoc::build(&parse_markdown(&source), &default_style());
        assert_eq!(doc.blocks.len(), 1, "tight list must remain a single ListItem for {source:?}");
        let item = &doc.blocks[0];
        assert!(matches!(item.kind, BlockKind::ListItem { .. }));
        assert!(item.children.is_empty(), "tight list item must not be split into paragraphs");
        assert_eq!(item.text_lines, ["first", "second"]);
        for projected in &item.projected_lines {
            projected.validate(&source).expect("tight list line projection must validate");
        }
    }

    let leading_br = "- <br>second";
    let doc = MarkdownDoc::build(&parse_markdown(leading_br), &default_style());
    assert_eq!(doc.blocks[0].text_lines, ["", "second"]);
    for projected in &doc.blocks[0].projected_lines {
        projected.validate(leading_br).expect("leading br in tight list must validate");
    }

    let with_nested = "- first<br>\n  - nested";
    let nested_doc = MarkdownDoc::build(&parse_markdown(with_nested), &default_style());
    assert_eq!(nested_doc.blocks[0].text_lines, ["first", ""]);
    assert_eq!(nested_doc.blocks[0].children.len(), 1);
}

#[test]
fn consecutive_hard_breaks_and_br_preserve_empty_logical_lines() {
    fn leaf_lines(block: &BlockNode) -> &[String] {
        if !block.text_lines.is_empty() {
            &block.text_lines
        } else {
            leaf_lines(&block.children[0])
        }
    }
    fn leaf_projections(block: &BlockNode) -> &[crate::projection::ProjectedText] {
        if !block.projected_lines.is_empty() {
            &block.projected_lines
        } else {
            leaf_projections(&block.children[0])
        }
    }

    for (source, expected) in [
        ("a<br><br>b", vec!["a", "", "b"]),
        ("a\\\n\\\nb", vec!["a", "", "b"]),
        ("a<br>  \nb", vec!["a", "", "b"]),
        ("> a\\\n> \\\n> b", vec!["a", "", "b"]),
        ("- a\\\n  \\\n  b", vec!["a", "", "b"]),
        ("a<br>\nb", vec!["a", "b"]),
        ("a<br>\n<br>b", vec!["a", "", "b"]),
    ] {
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
        assert_eq!(leaf_lines(&doc.blocks[0]), expected.as_slice(), "source={source:?}");
        let projections = leaf_projections(&doc.blocks[0]);
        assert_eq!(projections.len(), expected.len(), "source={source:?}");
        let mut all_bytes = Vec::new();
        for (line_idx, projected) in projections.iter().enumerate() {
            assert_eq!(projected.text, expected[line_idx], "source={source:?}");
            projected.validate(source).expect("projected line must be valid");
            all_bytes.extend(projected.boundaries.iter().map(|anchor| anchor.byte.get()));
        }
        assert!(
            all_bytes.windows(2).all(|pair| pair[0] <= pair[1]),
            "anchors across consecutive break lines must be non-decreasing for {source:?}: {all_bytes:?}"
        );
    }
}

#[test]
fn trailing_inline_html_break_retains_editable_continuation_line() {
    let heading = "# Title<br>";
    let heading_doc =
        MarkdownDoc::build_for_editing(&parse_markdown(heading), &default_style(), heading);
    assert_eq!(heading_doc.blocks[0].text_lines, ["Title", ""]);
    assert_eq!(heading_doc.blocks[0].projected_lines.len(), 2);
    heading_doc.blocks[0].projected_lines[1].validate(heading).expect("heading tail validates");

    let empty_heading = "# <br>";
    let empty_heading_doc = MarkdownDoc::build_for_editing(
        &parse_markdown(empty_heading),
        &default_style(),
        empty_heading,
    );
    assert_eq!(empty_heading_doc.blocks[0].text_lines, ["", ""]);
    assert_eq!(empty_heading_doc.blocks[0].projected_lines.len(), 2);

    let table = "| a |\n| --- |\n| cell<br> |";
    let table_doc = MarkdownDoc::build_for_editing(&parse_markdown(table), &default_style(), table);
    let body_row = &table_doc.blocks[0].children[1];
    let body_cell = &body_row.children[0];
    assert_eq!(body_cell.text_lines, ["cell", ""]);
    assert_eq!(body_cell.projected_lines.len(), 2);
    body_cell.projected_lines[1].validate(table).expect("table cell tail validates");
}
