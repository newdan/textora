use super::*;
use crate::builder::InlineStyle;
use crate::layout::types::{CollapsedBoundary, VisualLineProjection};
use crate::projection::SourceByte;
use crate::projection::{ProjectionError, ProjectionOwnerId, TextProjectionBuilder};
use crate::selection::GraphemeIdx;

#[cfg(feature = "rich-markdown")]
#[test]
fn editing_inline_math_replaces_entire_object_with_direct_source() {
    let source = "中$x^2$文";
    let document = crate::builder::MarkdownDoc::build(
        &crate::parser::parse_markdown(source),
        &crate::test_utils::default_style(),
    );
    let paragraph = &document.blocks[0];
    let base = &paragraph.projected_lines[0];
    let styles = &paragraph.text_styles[0];
    let preview = materialize_projected_line(base, styles, source, None, None);
    assert_eq!(preview.text, "中\u{FFFC}文");

    let edit_context = EditContext {
        cursor_byte: source.find('x').expect("fixture has math"),
        preedit_text: None,
        preedit_cursor: None,
    };
    let expanded = materialize_projected_line(base, styles, source, Some(&edit_context), None);
    assert_eq!(expanded.text, source);
    assert!(expanded.spans.iter().all(|span| !matches!(span.kind, ProjectionSpanKind::Collapsed)));
}

fn make_span(
    start: usize,
    len: usize,
    source_start: usize,
    source_end: usize,
    style: InlineStyle,
) -> StyleSpan {
    StyleSpan { start, len, style, source_range: source_start..source_end }
}

fn slice_visual_line_linear_reference(
    projected: &ProjectedText,
    logical_line: usize,
    visual_byte_range: Range<usize>,
) -> Result<VisualLineProjection, ProjectionError> {
    if visual_byte_range.start > visual_byte_range.end
        || visual_byte_range.end > projected.text.len()
        || !projected.text.is_char_boundary(visual_byte_range.start)
        || !projected.text.is_char_boundary(visual_byte_range.end)
    {
        return Err(ProjectionError::InvalidVisualBoundary {
            visual_byte: visual_byte_range.start,
        });
    }

    let start_grapheme =
        crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_byte_range.start);
    let end_grapheme =
        crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_byte_range.end);
    let boundaries = projected.boundaries[start_grapheme..=end_grapheme].to_vec();
    let mut source_extent = boundaries
        .first()
        .expect("a projected visual line always has a sentinel boundary")
        .byte
        .get()
        ..boundaries
            .last()
            .expect("a projected visual line always has a sentinel boundary")
            .byte
            .get();
    let mut collapsed = Vec::new();

    for span in &projected.spans {
        let collapsed_outside_visual_line = span.visual_range.end < visual_byte_range.start
            || span.visual_range.start > visual_byte_range.end;
        if !matches!(span.kind, ProjectionSpanKind::Collapsed) || collapsed_outside_visual_line {
            continue;
        }

        let upstream_grapheme = crate::grapheme_map::grapheme_index_at_byte(
            &projected.text,
            span.visual_range.start.max(visual_byte_range.start),
        ) - start_grapheme;
        let downstream_grapheme = crate::grapheme_map::grapheme_index_at_byte(
            &projected.text,
            span.visual_range.end.min(visual_byte_range.end),
        ) - start_grapheme;
        source_extent.start = source_extent.start.min(span.source_range.start);
        source_extent.end = source_extent.end.max(span.source_range.end);
        collapsed.push(CollapsedBoundary {
            source_range: span.source_range.clone(),
            upstream_grapheme,
            downstream_grapheme,
        });
    }

    Ok(VisualLineProjection {
        owner: ProjectionOwnerId::Block {
            block_start: projected.source_extent().start,
            logical_line,
        },
        boundaries,
        source_extent,
        collapsed,
    })
}

#[test]
fn indexed_visual_line_slice_matches_linear_unicode_reference() {
    let unicode_segments = ["e\u{0301}", "👩‍💻", "❤\u{fe0f}", "中文"];
    let mut builder = TextProjectionBuilder::default();
    let mut source_byte = 0usize;

    for (segment_index, segment) in unicode_segments.iter().enumerate() {
        if segment_index > 0 {
            builder.push_soft_break(source_byte..source_byte + 1);
            source_byte += 1;
        }
        let source_end = source_byte + segment.len();
        builder.push_direct(segment, source_byte..source_end);
        source_byte = source_end;
    }

    let projected = builder.finish(source_byte);
    let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&projected.text);
    let character_boundaries: Vec<usize> =
        (0..=projected.text.len()).filter(|byte| projected.text.is_char_boundary(*byte)).collect();

    for &visual_start in &character_boundaries {
        for &visual_end in character_boundaries.iter().filter(|&&byte| byte >= visual_start) {
            let visual_range = visual_start..visual_end;
            let linear = slice_visual_line_linear_reference(&projected, 7, visual_range.clone());
            let indexed = projected.slice_visual_line_indexed(
                &visual_grapheme_bytes,
                7,
                visual_range.clone(),
            );
            assert_eq!(
                indexed, linear,
                "indexed projection diverged for visual range {visual_range:?}"
            );
        }
    }
}

#[test]
fn cursor_in_span_inclusive_end() {
    // source: "hello **world** here"
    // Bold span: line "world" at [6..11], source "**world**" at [6..15]
    let span = make_span(6, 5, 6, 15, InlineStyle::Bold);
    assert!(cursor_in_span(&span, 6)); // 光标在起始边界
    assert!(cursor_in_span(&span, 10)); // 光标在中间
    assert!(cursor_in_span(&span, 15)); // 光标在结束边界 (闭区间)
    assert!(!cursor_in_span(&span, 5)); // 光标在 span 前
    assert!(!cursor_in_span(&span, 16)); // 光标在 span 后
}

#[test]
fn materialize_no_edit_ctx_returns_line_text() {
    let source = "hello **world** here";
    let line_text = "hello world here";
    let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
    let result = materialize_text(line_text, &spans, source, None);
    assert_eq!(result, "hello world here");
}

#[test]
fn materialize_unfolds_cursor_span() {
    let source = "hello **world** here";
    let line_text = "hello world here";
    let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
    // 光标在 Bold span 内 (source byte 10)
    let ctx = EditContext { cursor_byte: 10, preedit_text: None, preedit_cursor: None };
    let result = materialize_text(line_text, &spans, source, Some(&ctx));
    assert_eq!(result, "hello **world** here");
}

#[test]
fn materialize_no_matching_span_returns_line_text() {
    let source = "hello **world** here";
    let line_text = "hello world here";
    let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
    // 光标不在任何 span 内
    let ctx = EditContext { cursor_byte: 2, preedit_text: None, preedit_cursor: None };
    let result = materialize_text(line_text, &spans, source, Some(&ctx));
    assert_eq!(result, "hello world here");
}

#[test]
fn materialize_multiple_spans_unfolds_only_cursor() {
    let source = "**bold** and *italic*";
    let line_text = "bold and italic";
    let spans = vec![
        make_span(0, 4, 0, 8, InlineStyle::Bold),
        make_span(9, 6, 13, 21, InlineStyle::Italic),
    ];
    // 光标在 Bold span 内
    let ctx = EditContext { cursor_byte: 3, preedit_text: None, preedit_cursor: None };
    let result = materialize_text(line_text, &spans, source, Some(&ctx));
    assert_eq!(result, "**bold** and italic");
}

#[test]
fn materialize_cursor_at_italic_span() {
    let source = "**bold** and *italic*";
    let line_text = "bold and italic";
    let spans = vec![
        make_span(0, 4, 0, 8, InlineStyle::Bold),
        make_span(9, 6, 13, 21, InlineStyle::Italic),
    ];
    // 光标在 Italic span 内
    let ctx = EditContext { cursor_byte: 17, preedit_text: None, preedit_cursor: None };
    let result = materialize_text(line_text, &spans, source, Some(&ctx));
    assert_eq!(result, "bold and *italic*");
}

#[test]
fn materialize_empty_line_text() {
    let result = materialize_text("", &[], "", None);
    assert_eq!(result, "");
}

#[test]
fn materialize_cursor_at_boundary() {
    // 光标恰好在 span 的 source_range.end（闭区间包含）
    let source = "**bold** text";
    let line_text = "bold text";
    let spans = vec![make_span(0, 4, 0, 8, InlineStyle::Bold)];
    let ctx = EditContext { cursor_byte: 8, preedit_text: None, preedit_cursor: None }; // source_range.end = 8
    let result = materialize_text(line_text, &spans, source, Some(&ctx));
    assert_eq!(result, "**bold** text");
}

#[test]
fn materialize_inline_code_span() {
    let source = "use `println!` here";
    let line_text = "use println! here";
    let spans = vec![make_span(4, 8, 4, 14, InlineStyle::InlineCode)];
    let ctx = EditContext { cursor_byte: 8, preedit_text: None, preedit_cursor: None };
    let result = materialize_text(line_text, &spans, source, Some(&ctx));
    assert_eq!(result, "use `println!` here");
}

#[test]
fn inline_code_preedit_at_content_boundaries_keeps_markers_separate() {
    let source = "before `ab` after";
    let code_start = source.find("ab").expect("fixture contains inline code");
    let delimiter_len = "`".len();
    let spans = [make_span(
        code_start - delimiter_len,
        "ab".len(),
        code_start - delimiter_len,
        code_start + "ab`".len(),
        InlineStyle::InlineCode,
    )];
    let source_line = SourceLineContext { range: 0..source.len(), text_start: 0 };
    for (cursor_byte, expected_code) in [
        (code_start - delimiter_len, "ab"),
        (code_start, "中文ab"),
        (code_start + "ab".len(), "ab中文"),
        (code_start + "ab`".len(), "ab"),
    ] {
        let context = EditContext {
            cursor_byte,
            preedit_text: Some("中文".to_owned()),
            preedit_cursor: Some(("中文".len(), "中文".len())),
        };
        let line = materialize_line_with_source_context(
            "before ab after",
            &spans,
            source,
            Some(&context),
            Some(&source_line),
        );
        assert_eq!(
            line.spans
                .iter()
                .map(|span| (&span.style, &line.text[span.start..span.start + span.len]))
                .collect::<Vec<_>>(),
            [
                (&InlineStyle::SourceMarker, "`"),
                (&InlineStyle::InlineCode, expected_code),
                (&InlineStyle::SourceMarker, "`"),
            ],
            "preedit at source byte {cursor_byte} must inherit the code content style",
        );
    }
}

#[test]
fn inline_code_preedit_outside_closing_marker_does_not_extend_adjacent_style() {
    let source = "`ab`**cd**";
    let spans = [
        make_span(0, "ab".len(), 0, "`ab`".len(), InlineStyle::InlineCode),
        make_span("ab".len(), "cd".len(), "`ab`".len(), source.len(), InlineStyle::Bold),
    ];
    let source_line = SourceLineContext { range: 0..source.len(), text_start: 0 };
    let context = EditContext {
        cursor_byte: "`ab`".len(),
        preedit_text: Some("中文".to_owned()),
        preedit_cursor: Some(("中文".len(), "中文".len())),
    };
    let line = materialize_line_with_source_context(
        "abcd",
        &spans,
        source,
        Some(&context),
        Some(&source_line),
    );
    assert_eq!(line.text, "`ab`中文cd");
    let bold = line
        .spans
        .iter()
        .find(|span| span.style == InlineStyle::Bold)
        .expect("adjacent text must remain bold");
    assert_eq!(&line.text[bold.start..bold.start + bold.len], "cd");
}

#[test]
fn materialized_line_maps_expanded_bold_markers_to_source_bytes() {
    let source = "hello **world** here";
    let line_text = "hello world here";
    let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
    let ctx = EditContext { cursor_byte: 10, preedit_text: None, preedit_cursor: None };

    let line = materialize_line(line_text, &spans, source, Some(&ctx));

    assert_eq!(line.text, "hello **world** here");
    assert_eq!(line.visual_grapheme_to_source_byte(6), Some(6));
    assert_eq!(line.visual_grapheme_to_source_byte(8), Some(8));
    assert_eq!(line.source_byte_to_visual_grapheme(10), Some(10));
}

#[test]
fn materialized_multiline_bold_keeps_softbreak_collapsed() {
    let source = "**first\n  second**";
    let second_start = source.find("second").expect("fixture contains continuation text");
    let mut projection_builder = TextProjectionBuilder::default();
    projection_builder.push_direct("first", 2..7);
    projection_builder.push_soft_break(7..8);
    projection_builder.push_direct("second", second_start..second_start + "second".len());
    let base = projection_builder.finish(source.len() - 2);
    let spans = vec![make_span(0, "first second".len(), 0, source.len(), InlineStyle::Bold)];
    let ctx = EditContext { cursor_byte: second_start, preedit_text: None, preedit_cursor: None };

    let projected = materialize_projected_line(&base, &spans, source, Some(&ctx), None);
    let materialized_spans =
        materialized_spans_for_projected_line(&base, &projected, &spans, source, Some(&ctx));

    assert_eq!(projected.text, "**first second**");
    assert!(projected.spans.iter().any(|span| {
        matches!(span.kind, ProjectionSpanKind::Collapsed)
            && span.source_range.start == 7
            && span.source_range.end == second_start
    }));
    assert_eq!(
        materialized_spans
            .iter()
            .map(|span| (&projected.text[span.start..span.start + span.len], &span.style))
            .collect::<Vec<_>>(),
        vec![
            ("**", &InlineStyle::SourceMarker),
            ("first second", &InlineStyle::Bold),
            ("**", &InlineStyle::SourceMarker),
        ]
    );
    projected.validate(source).expect("materialized projection must remain valid");
}

#[test]
fn materialized_line_keeps_folded_text_when_cursor_outside_span() {
    let source = "hello **world** here";
    let line_text = "hello world here";
    let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
    let ctx = EditContext { cursor_byte: 2, preedit_text: None, preedit_cursor: None };

    let line = materialize_line(line_text, &spans, source, Some(&ctx));

    assert_eq!(line.text, "hello world here");
    assert_eq!(line.visual_grapheme_to_source_byte(6), Some(8));
    assert_eq!(line.source_byte_to_visual_grapheme(10), Some(8));
}

#[test]
fn materialized_line_maps_combining_grapheme_to_single_source_position() {
    let source = "**e\u{0301}**";
    let line_text = "e\u{0301}";
    // line_text is 2 chars (e + combining acute), 3 bytes total
    let spans = vec![make_span(0, line_text.len(), 0, source.len(), InlineStyle::Bold)];
    let ctx = EditContext { cursor_byte: 3, preedit_text: None, preedit_cursor: None };

    let line = materialize_line(line_text, &spans, source, Some(&ctx));

    assert_eq!(line.text, source);
    // ** markers at grapheme 0 and 1; é at grapheme 2; ** at 3 and 4
    assert_eq!(line.visual_grapheme_to_source_byte(0), Some(0));
    assert_eq!(line.visual_grapheme_to_source_byte(2), Some(2));
    // Next grapheme after é starts at byte 2 + "e\u{0301}".len() = 2 + 3 = 5
    assert_eq!(line.visual_grapheme_to_source_byte(3), Some(2 + "e\u{0301}".len()));
}

#[test]
fn materialized_heading_marker_keeps_absolute_source_anchors() {
    let base = crate::projection::ProjectedText::direct("Title", 2);
    let marker = ActiveBlockMarker { marker_text: "# ".to_string(), marker_source_range: 0..2 };

    let projected = materialize_block_marker(base, &marker);

    assert_eq!(projected.text, "# Title");
    assert_eq!(
        projected.boundaries.iter().map(|anchor| anchor.byte.get()).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5, 6, 7]
    );
}

#[test]
fn preedit_visual_text_anchors_every_boundary_to_cursor_byte() {
    let base = crate::projection::ProjectedText::direct("ab", 10);

    let projected = base.insert_virtual(GraphemeIdx::new(1), "中文", SourceByte::new(11));

    assert_eq!(projected.text, "a中文b");
    assert_eq!(projected.boundaries[1].byte.get(), 11);
    assert_eq!(projected.boundaries[2].byte.get(), 11);
    assert_eq!(projected.boundaries[3].byte.get(), 11);
}

// ===== ActiveBlockMarker tests =====

use crate::builder::{BlockKind, BlockNode, BlockSourceSpan, ListBullet};

fn heading_block(text: &str) -> BlockNode {
    BlockNode {
        kind: BlockKind::Heading { level: 1 },
        children: vec![],
        block_range: 0..text.len(),
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..text.len()),
        text_lines: vec![text[2..].to_string()], // content after "# "
        projected_lines: vec![],
        text_styles: vec![vec![]],
    }
}

#[test]
fn active_marker_for_heading() {
    let block = heading_block("# Title");
    let marker = active_block_marker(&block, 3).expect("cursor in heading should produce marker");
    assert_eq!(marker.marker_text, "# ");
    assert_eq!(marker.marker_source_range, 0..2);
}

#[test]
fn active_marker_none_when_cursor_outside_block_range() {
    let block = heading_block("# Title");
    assert!(active_block_marker(&block, 8).is_none(), "cursor past block end");
    assert!(active_block_marker(&block, 100).is_none(), "cursor way past block");
}

#[test]
fn active_marker_for_h2() {
    let block = BlockNode {
        kind: BlockKind::Heading { level: 2 },
        children: vec![],
        block_range: 0..9,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..9),
        text_lines: vec!["SubTitle".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    let marker = active_block_marker(&block, 5).expect("cursor in h2");
    assert_eq!(marker.marker_text, "## ");
    assert_eq!(marker.marker_source_range, 0..3);
}

#[test]
fn active_marker_for_list_item_bullet() {
    let block = BlockNode {
        kind: BlockKind::ListItem {
            bullet: ListBullet::Bullet,
            tight: true,
            blank_line_before: false,
        },
        children: vec![],
        block_range: 0..7,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..7),
        text_lines: vec!["item".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    let marker = active_block_marker(&block, 3).expect("cursor in list");
    assert_eq!(marker.marker_text, "- ");
    assert_eq!(marker.marker_source_range, 0..2);
}

#[test]
fn active_marker_for_ordered_list_item() {
    let block = BlockNode {
        kind: BlockKind::ListItem {
            bullet: ListBullet::Ordered(3),
            tight: true,
            blank_line_before: false,
        },
        children: vec![],
        block_range: 0..6,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..6),
        text_lines: vec!["item".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    let marker = active_block_marker(&block, 2).expect("cursor in ordered list");
    assert_eq!(marker.marker_text, "3. ");
    assert_eq!(marker.marker_source_range, 0..3);
}

#[test]
fn active_marker_for_task_list() {
    let block = BlockNode {
        kind: BlockKind::ListItem {
            bullet: ListBullet::TaskList(false),
            tight: true,
            blank_line_before: false,
        },
        children: vec![],
        block_range: 0..9,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..9),
        text_lines: vec!["todo".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    let marker = active_block_marker(&block, 3).expect("cursor in task list");
    assert_eq!(marker.marker_text, "- [ ] ");
    assert_eq!(marker.marker_source_range, 0..6);
}

#[test]
fn active_marker_for_blockquote() {
    let block = BlockNode {
        kind: BlockKind::BlockQuote,
        children: vec![],
        block_range: 0..10,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..10),
        text_lines: vec!["quoted".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    let marker = active_block_marker(&block, 5).expect("cursor in blockquote");
    assert_eq!(marker.marker_text, "> ");
    assert_eq!(marker.marker_source_range, 0..2);
}

#[test]
fn block_marker_len_heading() {
    let h1 = BlockNode {
        kind: BlockKind::Heading { level: 1 },
        children: vec![],
        block_range: 0..7,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..7),
        text_lines: vec!["Title".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    assert_eq!(block_marker_len(&h1), 2);
}

#[test]
fn block_marker_len_h3() {
    let h3 = BlockNode {
        kind: BlockKind::Heading { level: 3 },
        children: vec![],
        block_range: 0..9,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..9),
        text_lines: vec!["Small".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    assert_eq!(block_marker_len(&h3), 4); // "### " = 4 bytes
}

#[test]
fn block_marker_len_ordered_list_accepts_zero_and_digit_boundaries() {
    for (number, expected_length) in [(0, 3), (1, 3), (9, 3), (10, 4), (42, 4), (999_999_999, 11)] {
        let source = format!("{number}. item");
        let parsed = crate::parser::parse_markdown(&source);
        let document =
            crate::builder::MarkdownDoc::build(&parsed, &crate::test_utils::default_style());
        let block = document.blocks.first().expect("fixture must contain an ordered list item");

        assert_eq!(block_marker_len(block), expected_length, "source: {source:?}");
    }
}

#[test]
fn block_marker_len_paragraph_is_zero() {
    let para = BlockNode {
        kind: BlockKind::Paragraph,
        children: vec![],
        block_range: 0..5,
        code_line_source_starts: None,
        source_range: BlockSourceSpan::Continuous(0..5),
        text_lines: vec!["text".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
    };
    assert_eq!(block_marker_len(&para), 0);
}

#[test]
fn cross_hard_break_bold_and_br_expand_delimiters_per_logical_line() {
    for (source, line1_cursor_needle, expected_line1) in [
        ("**line1\\\nline2**", "line2", "line2**"),
        ("**line1<br>line2**", "line2", "line2**"),
        ("[a\\\nb](https://x.test)", "b", "b](https://x.test)"),
    ] {
        let parsed = crate::parser::parse_markdown(source);
        let doc = crate::builder::MarkdownDoc::build(&parsed, &crate::test_utils::default_style());
        let block = &doc.blocks[0];
        assert_eq!(block.projected_lines.len(), 2, "expected 2 lines in {source:?}");

        let line1_cursor = source.find(line1_cursor_needle).expect("fixture contains line2");
        let ctx =
            EditContext { cursor_byte: line1_cursor, preedit_text: None, preedit_cursor: None };
        let line0 = materialize_projected_line(
            &block.projected_lines[0],
            &block.text_styles[0],
            source,
            Some(&ctx),
            None,
        );
        let line1 = materialize_projected_line(
            &block.projected_lines[1],
            &block.text_styles[1],
            source,
            Some(&ctx),
            None,
        );

        assert_eq!(line1.text, expected_line1, "line1 expansion mismatch for {source:?}");
        line0.validate(source).expect("line0 projection must be valid");
        line1.validate(source).expect("line1 projection must be valid");

        let mut all_bytes = Vec::new();
        all_bytes.extend(line0.boundaries.iter().map(|anchor| anchor.byte.get()));
        all_bytes.extend(line1.boundaries.iter().map(|anchor| anchor.byte.get()));
        assert!(
            all_bytes.windows(2).all(|pair| pair[0] <= pair[1]),
            "cross-line boundaries must be monotonically non-decreasing in {source:?}: {all_bytes:?}"
        );
    }

    let three_line = "**l1\\\nl2\\\nl3**";
    let parsed = crate::parser::parse_markdown(three_line);
    let doc = crate::builder::MarkdownDoc::build(&parsed, &crate::test_utils::default_style());
    let block = &doc.blocks[0];
    assert_eq!(block.projected_lines.len(), 3);
    let middle_cursor = three_line.find("l2").expect("fixture contains l2");
    let ctx = EditContext { cursor_byte: middle_cursor, preedit_text: None, preedit_cursor: None };
    let middle = materialize_projected_line(
        &block.projected_lines[1],
        &block.text_styles[1],
        three_line,
        Some(&ctx),
        None,
    );
    assert_eq!(middle.text, "l2");
}

#[test]
fn cross_hard_break_preedit_preserves_monotonic_anchors_and_caret() {
    use crate::projection::SourceProjectionIndex;

    let source = "**line1\\\nline2**";
    let parsed = crate::parser::parse_markdown(source);
    let doc = crate::builder::MarkdownDoc::build(&parsed, &crate::test_utils::default_style());
    let block = &doc.blocks[0];
    let line2_start = source.find("line2").expect("fixture has line2");
    let cursor_byte = line2_start + 2;
    let ctx = EditContext {
        cursor_byte,
        preedit_text: Some("中文".to_owned()),
        preedit_cursor: Some(("中".len(), "中".len())),
    };
    let line0 = materialize_projected_line(
        &block.projected_lines[0],
        &block.text_styles[0],
        source,
        Some(&ctx),
        None,
    );
    let line1 = materialize_projected_line(
        &block.projected_lines[1],
        &block.text_styles[1],
        source,
        Some(&ctx),
        Some(&SourceLineContext { range: line2_start..source.len(), text_start: line2_start }),
    );
    assert_eq!(line1.text, "li中文ne2**");

    let mut visual0 =
        line0.slice_visual_line(0, 0..line0.text.len()).expect("line0 slice must build");
    visual0.owner = ProjectionOwnerId::Block { block_start: 0, logical_line: 0 };
    let mut visual1 =
        line1.slice_visual_line(1, 0..line1.text.len()).expect("line1 slice must build");
    visual1.owner = ProjectionOwnerId::Block { block_start: line2_start, logical_line: 1 };
    let index = SourceProjectionIndex::build(1, 1, vec![visual0, visual1])
        .expect("cross-line preedit index must satisfy monotonically non-decreasing anchors");
    assert!(index.virtual_position_for_source(SourceByte::new(cursor_byte), 1).is_some());
}
