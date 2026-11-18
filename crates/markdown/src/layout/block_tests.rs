use super::*;
use crate::builder::InlineStyle;
use crate::builder::MarkdownDoc;
use crate::layout::LazyLayout;
use crate::parser::parse_markdown;
use crate::test_utils::default_style;

#[cfg(feature = "rich-markdown")]
#[test]
fn inline_math_cluster_uses_rendered_image_width() {
    let source = "left $x^2$ right";
    let parsed = parse_markdown(source);
    let style = default_style();
    let document = MarkdownDoc::build(&parsed, &style);
    assert!(
        document.blocks[0]
            .text_styles
            .iter()
            .flatten()
            .any(|span| matches!(span.style, InlineStyle::Math))
    );
    let text_document = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("test environment has a font");
    let layout = layout_doc_with_shaper_for_rendering(
        &document.blocks,
        &style,
        400.0,
        Some(&mut shaper),
        None,
        &text_document,
    );
    let LaidOutBlockKind::Text { lines } = &layout.document().blocks[0].kind else {
        panic!("math fixture has one paragraph");
    };
    let line = &lines[0];
    let math_span =
        line.styles.iter().find(|span| matches!(span.style, InlineStyle::Math)).expect("math span");
    let cluster = line
        .shaped
        .as_ref()
        .expect("shaped line")
        .clusters
        .iter()
        .find(|cluster| cluster.byte_range == (math_span.start..math_span.start + math_span.len))
        .expect("math cluster");
    let expected = crate::embedded::render_embedded(crate::embedded::EmbeddedRequest {
        kind: crate::embedded::EmbeddedKind::InlineMath,
        source: "x^2",
        font_size: style.body_font_size,
        foreground: [0, 0, 0, 255],
        background: [0, 0, 0, 0],
    })
    .expect("valid inline math");
    assert!((cluster.advance - expected.image.width() as f32).abs() < 1.0);
}

#[cfg(feature = "rich-markdown")]
#[test]
fn tall_inline_math_expands_its_row_and_following_block_position() {
    let source = "left $\\frac{1}{2}$ right\n\nnext";
    let parsed = parse_markdown(source);
    let style = default_style();
    let document = MarkdownDoc::build(&parsed, &style);
    let text_document = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("test font");
    let layout = layout_doc_with_shaper_for_rendering(
        &document.blocks,
        &style,
        400.0,
        Some(&mut shaper),
        None,
        &text_document,
    );
    let LaidOutBlockKind::Text { lines } = &layout.document().blocks[0].kind else {
        panic!("first block is paragraph");
    };
    let math_range = lines[0]
        .styles
        .iter()
        .find(|span| matches!(span.style, InlineStyle::Math))
        .expect("math span")
        .source_range
        .clone();
    let image = layout.embedded_images().image_for(&math_range).expect("math image");
    let ascent = image.baseline.max(style.body_font_size);
    let descent = (image.image.height() as f32 - image.baseline)
        .max(style.line_height - style.body_font_size);
    assert!(lines[0].rect.h >= ascent + descent);
    assert!(
        layout.document().blocks[1].rect.y >= layout.document().blocks[0].rect.y + lines[0].rect.h
    );
}

#[cfg(feature = "rich-markdown")]
#[test]
fn display_math_and_mermaid_become_image_blocks() {
    for source in ["$$\\frac{1}{2}$$", "```mermaid\nflowchart LR\nA --> B\n```"] {
        let parsed = parse_markdown(source);
        let style = default_style();
        let document = MarkdownDoc::build(&parsed, &style);
        let text_document = core::document::StringDocView::new(source);
        let mut shaper = shaping::Shaper::new().expect("test font");
        let layout = layout_doc_with_shaper_for_rendering(
            &document.blocks,
            &style,
            400.0,
            Some(&mut shaper),
            None,
            &text_document,
        );
        assert!(format!("{:?}", layout.document().blocks[0].kind).contains("Embedded"), "{source}");
        assert!(
            layout.embedded_images().image_for(&document.blocks[0].block_range).is_some(),
            "{source}"
        );
    }
}

#[cfg(feature = "rich-markdown")]
#[test]
fn invalid_inline_math_falls_back_to_readable_source() {
    let source = "before $\\noSuchMathCommand$ after";
    let parsed = parse_markdown(source);
    let style = default_style();
    let document = MarkdownDoc::build(&parsed, &style);
    assert!(
        document.blocks[0]
            .text_styles
            .iter()
            .flatten()
            .any(|span| matches!(span.style, InlineStyle::Math))
    );
    let text_document = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("test font");
    let layout = layout_doc_with_shaper_for_rendering(
        &document.blocks,
        &style,
        400.0,
        Some(&mut shaper),
        None,
        &text_document,
    );
    let LaidOutBlockKind::Text { lines } = &layout.document().blocks[0].kind else {
        panic!("paragraph expected");
    };
    assert_eq!(lines.iter().map(|line| line.text.as_str()).collect::<String>(), source);
}

#[cfg(feature = "rich-markdown")]
#[test]
fn invalid_mermaid_shows_complete_fenced_source() {
    let source = "```mermaid\nnot a diagram\n```";
    let parsed = parse_markdown(source);
    let style = default_style();
    let document = MarkdownDoc::build(&parsed, &style);
    let text_document = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("test font");
    let layout = layout_doc_with_shaper_for_rendering(
        &document.blocks,
        &style,
        400.0,
        Some(&mut shaper),
        None,
        &text_document,
    );
    let LaidOutBlockKind::CodeBlock { lines, .. } = &layout.document().blocks[0].kind else {
        panic!("invalid mermaid falls back to code block");
    };
    assert!(lines.iter().any(|line| line.text == "```mermaid"));
    assert!(lines.iter().any(|line| line.text == "```"));
}

#[cfg(feature = "rich-markdown")]
#[test]
fn table_column_demand_includes_inline_math_image_width() {
    let source = "| x | y |\n|---|---|\n| $a+b+c+d+e+f+g+h+i+j+k+l$ | z |";
    let style = default_style();
    let document = MarkdownDoc::build(&parse_markdown(source), &style);
    let table = &document.blocks[0];
    let text_document = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("test font");
    let demand = measure_column_demand(
        table,
        2,
        style.body_font_size,
        style.text_spacing_mode,
        style.body_font_family.first().map(String::as_str),
        Some(&mut shaper),
        &text_document,
    );
    let image = crate::embedded::render_embedded(crate::embedded::EmbeddedRequest {
        kind: crate::embedded::EmbeddedKind::InlineMath,
        source: "a+b+c+d+e+f+g+h+i+j+k+l",
        font_size: style.body_font_size,
        foreground: [0, 0, 0, 255],
        background: [0, 0, 0, 0],
    })
    .expect("valid math");
    assert!(
        demand[0] >= image.image.width() as f32,
        "demand={} image={}",
        demand[0],
        image.image.width()
    );
}

#[test]
fn bare_layout_keeps_inline_math_source_readable_without_image_sidecar() {
    let source = "before $x^2$ after";
    let style = default_style();
    let document = MarkdownDoc::build(&parse_markdown(source), &style);
    let text_document = core::document::StringDocView::new(source);
    let layout = layout_doc(&document.blocks, &style, 400.0, &text_document);
    let LaidOutBlockKind::Text { lines, .. } = &layout.blocks[0].kind else {
        panic!("expected paragraph");
    };
    assert_eq!(lines[0].text, source);
}

const ASCII_DIAGRAM_SOURCE: &str = "```\n┌────┐\n│中文│\n└────┘\n```";
const INDENTED_ASCII_DIAGRAM_SOURCE: &str = "    ┌────┐\n    │中文│\n    └────┘";

fn make_doc(md: &str) -> (&str, MarkdownDoc) {
    let parsed = parse_markdown(md);
    (md, MarkdownDoc::build(&parsed, &default_style()))
}

#[test]
fn paragraph_first_line_indent_positions_estimated_and_precise_lines() {
    let source = "一二三四五六七八九十一二三四五六七八九十";
    let mut style = default_style();
    style.paragraph_first_line_indent = style.body_font_size * 2.0;
    let (_, markdown) = make_doc(source);
    let document = core::document::StringDocView::new(source);
    let viewport_width = 100.0;
    for precise in [false, true] {
        let mut shaper = Shaper::new().expect("layout test requires a system font shaper");
        let layout = layout_doc_with_shaper(
            &markdown.blocks,
            &style,
            viewport_width,
            precise.then_some(&mut shaper),
            None,
            &document,
        );
        let LaidOutBlockKind::Text { lines } = &layout.blocks[0].kind else {
            panic!("fixture must produce paragraph text");
        };
        assert!(lines.len() > 1);
        assert_eq!(lines[0].rect.x, style.body_font_size * 2.0);
        assert_eq!(lines[0].rect.x + lines[0].rect.w, viewport_width);
        assert!(lines[1..].iter().all(|line| line.rect.x == 0.0 && line.rect.w == viewport_width));
        let projections: String = lines
            .iter()
            .map(|line| {
                line.source_projection
                    .as_ref()
                    .expect("every visual line must retain projection")
                    .source_extent
                    .clone()
            })
            .map(|range| &source[range])
            .collect();
        assert_eq!(projections, source);
    }
}

#[test]
fn paragraph_first_line_indent_preserves_manual_whitespace_and_structural_blocks() {
    for source in [
        "  正文",
        "\u{a0}正文",
        "\u{3000}正文",
        "# 标题",
        "- 列表",
        "> 引用",
        "```\n代码\n```",
        "    代码",
    ] {
        let mut style = default_style();
        let (_, markdown) = make_doc(source);
        let document = core::document::StringDocView::new(source);
        let baseline = layout_doc(&markdown.blocks, &style, 200.0, &document);
        style.paragraph_first_line_indent = style.body_font_size * 2.0;
        let indented = layout_doc(&markdown.blocks, &style, 200.0, &document);
        assert_eq!(
            format!("{baseline:?}"),
            format!("{indented:?}"),
            "unexpected indentation for {source:?}"
        );
    }
}

#[test]
fn novel_tab_line_preserves_verbatim_geometry() {
    let source = "中文A\t继续(说明)";
    let document = core::document::StringDocView::new(source);
    let structure = crate::builder::NovelStructure::scan(&document);
    let mut shaper = Shaper::new().expect("novel tab layout requires fonts");
    let mut widths = Vec::new();
    for mode in
        [ui::typography::TextSpacingMode::Natural, ui::typography::TextSpacingMode::Verbatim]
    {
        let mut style = MarkdownStyle::novel(&ui::theme::test_theme(), 15.0, 24.0);
        style.text_spacing_mode = mode;
        let layout = layout_doc_with_shaper_for_rendering(
            crate::layout::BlockSource::blocks(&structure),
            &style,
            800.0,
            Some(&mut shaper),
            None,
            &document,
        );
        let LaidOutBlockKind::Text { lines } = &layout.document().blocks[0].kind else {
            panic!("novel fixture must be a text block");
        };
        widths.push(lines[0].shaped.as_ref().expect("novel line must shape").width);
    }
    assert!((widths[0] - widths[1]).abs() < 0.01, "tab line widths: {widths:?}");
}

#[test]
fn final_markdown_line_geometry_reuses_natural_spacing_for_styles() {
    let source = "中文**Word**正文";
    let document = core::document::StringDocView::new(source);
    let parsed = parse_markdown(source);
    let mut natural_style = default_style();
    natural_style.text_spacing_mode = ui::typography::TextSpacingMode::Natural;
    let markdown = MarkdownDoc::build(&parsed, &natural_style);
    let mut shaper = Shaper::new().expect("styled geometry needs a text shaper");
    let natural = layout_doc_with_shaper_for_rendering(
        &markdown.blocks,
        &natural_style,
        800.0,
        Some(&mut shaper),
        None,
        &document,
    );
    let natural_line = match &natural.document().blocks[0].kind {
        LaidOutBlockKind::Text { lines } => lines.first().expect("paragraph line exists"),
        _ => panic!("fixture must produce a paragraph"),
    };
    let natural_layout = natural_line.text_layout.as_ref().expect("line layout exists");
    assert!(natural_line.styles.iter().any(|span| span.style == InlineStyle::Bold));
    assert!(!natural_line.style_segments.is_empty());

    let mut verbatim_style = natural_style.clone();
    verbatim_style.text_spacing_mode = ui::typography::TextSpacingMode::Verbatim;
    let verbatim_markdown = MarkdownDoc::build(&parse_markdown(source), &verbatim_style);
    let mut verbatim_shaper = Shaper::new().expect("verbatim geometry needs a text shaper");
    let verbatim = layout_doc_with_shaper_for_rendering(
        &verbatim_markdown.blocks,
        &verbatim_style,
        800.0,
        Some(&mut verbatim_shaper),
        None,
        &document,
    );
    let verbatim_line = match &verbatim.document().blocks[0].kind {
        LaidOutBlockKind::Text { lines } => lines.first().expect("paragraph line exists"),
        _ => panic!("fixture must produce a paragraph"),
    };
    let verbatim_layout = verbatim_line.text_layout.as_ref().expect("line layout exists");
    assert_eq!(natural_line.text, verbatim_line.text);
    assert!(natural_layout.shaped.width > verbatim_layout.shaped.width);
    for segment in &natural_line.style_segments {
        let start = segment.start;
        let end = start + segment.len;
        let segment_width = natural_line
            .shaped
            .as_ref()
            .expect("line retains final shape")
            .clusters
            .iter()
            .filter(|cluster| cluster.byte_range.start >= start && cluster.byte_range.end <= end)
            .map(|cluster| cluster.advance)
            .sum::<f32>();
        assert!((segment.width - segment_width).abs() < 0.01);
    }
}

fn line_widths_for_mode(source: &str, mode: ui::typography::TextSpacingMode) -> Vec<f32> {
    let mut style = default_style();
    style.text_spacing_mode = mode;
    let markdown = MarkdownDoc::build(&parse_markdown(source), &style);
    let document = core::document::StringDocView::new(source);
    let mut shaper = Shaper::new().expect("spacing fixture needs a text shaper");
    let layout = layout_doc_with_shaper_for_rendering(
        &markdown.blocks,
        &style,
        800.0,
        Some(&mut shaper),
        None,
        &document,
    );
    let mut widths = Vec::new();
    fn collect(block: &LaidOutBlock, widths: &mut Vec<f32>) {
        match &block.kind {
            LaidOutBlockKind::Text { lines }
            | LaidOutBlockKind::ListItem { lines, .. }
            | LaidOutBlockKind::MetadataBlock { lines } => {
                widths.extend(lines.iter().filter_map(|line| {
                    line.text_layout.as_ref().map(|layout| layout.shaped.width)
                }));
            }
            LaidOutBlockKind::BlockQuote { blocks } => {
                for child in blocks {
                    collect(child, widths);
                }
            }
            LaidOutBlockKind::Table { header, rows, .. } => {
                for cell in header {
                    collect_lines(cell, widths);
                }
                for row in rows {
                    for cell in row {
                        collect_lines(cell, widths);
                    }
                }
            }
            LaidOutBlockKind::CodeBlock { .. }
            | LaidOutBlockKind::HorizontalRule
            | LaidOutBlockKind::Embedded { .. } => {}
        }
    }
    fn collect_lines(lines: &[LaidOutLine], widths: &mut Vec<f32>) {
        widths.extend(
            lines
                .iter()
                .filter_map(|line| line.text_layout.as_ref().map(|layout| layout.shaped.width)),
        );
    }
    for block in &layout.document().blocks {
        collect(block, &mut widths);
    }
    widths
}

fn cumulative_width(line: &LaidOutLine, byte: usize) -> f32 {
    line.shaped
        .as_ref()
        .expect("spacing fixture must retain final shape")
        .clusters
        .iter()
        .filter(|cluster| cluster.byte_range.end <= byte)
        .map(|cluster| cluster.advance)
        .sum()
}

#[test]
fn natural_spacing_covers_list_table_code_and_autolink_layouts() {
    let list_natural =
        line_widths_for_mode("- 中文A\n- A中文", ui::typography::TextSpacingMode::Natural);
    let list_verbatim =
        line_widths_for_mode("- 中文A\n- A中文", ui::typography::TextSpacingMode::Verbatim);
    assert_eq!(list_natural.len(), list_verbatim.len());
    assert!(list_natural.iter().zip(&list_verbatim).any(|(natural, verbatim)| natural > verbatim));

    let table_source = "| 左A | B右 |\n| --- | --- |\n| 中A | A中 |";
    let table_natural =
        line_widths_for_mode(table_source, ui::typography::TextSpacingMode::Natural);
    let table_verbatim =
        line_widths_for_mode(table_source, ui::typography::TextSpacingMode::Verbatim);
    assert_eq!(table_natural.len(), table_verbatim.len());
    assert!(
        table_natural.iter().zip(&table_verbatim).any(|(natural, verbatim)| natural > verbatim)
    );

    let source = "中文`A中文B`正文<span title=\"中文A\">X</span><https://example.com>A中文";
    let mut natural_style = default_style();
    natural_style.text_spacing_mode = ui::typography::TextSpacingMode::Natural;
    let natural_doc = MarkdownDoc::build(&parse_markdown(source), &natural_style);
    let document = core::document::StringDocView::new(source);
    let mut natural_shaper = Shaper::new().expect("protected fixture needs a text shaper");
    let natural = layout_doc_with_shaper_for_rendering(
        &natural_doc.blocks,
        &natural_style,
        800.0,
        Some(&mut natural_shaper),
        None,
        &document,
    );
    let natural_line = match &natural.document().blocks[0].kind {
        LaidOutBlockKind::Text { lines } => lines.first().expect("paragraph line exists"),
        _ => panic!("fixture must produce a paragraph"),
    };
    let code = natural_line
        .styles
        .iter()
        .find(|span| span.style == InlineStyle::InlineCode)
        .expect("inline code style exists");
    let natural_code_width = cumulative_width(natural_line, code.start + code.len)
        - cumulative_width(natural_line, code.start);
    let mut verbatim_style = natural_style.clone();
    verbatim_style.text_spacing_mode = ui::typography::TextSpacingMode::Verbatim;
    let verbatim_doc = MarkdownDoc::build(&parse_markdown(source), &verbatim_style);
    let mut verbatim_shaper = Shaper::new().expect("protected fixture needs a shaper");
    let verbatim = layout_doc_with_shaper_for_rendering(
        &verbatim_doc.blocks,
        &verbatim_style,
        800.0,
        Some(&mut verbatim_shaper),
        None,
        &document,
    );
    let verbatim_line = match &verbatim.document().blocks[0].kind {
        LaidOutBlockKind::Text { lines } => lines.first().expect("paragraph line exists"),
        _ => panic!("fixture must produce a paragraph"),
    };
    let verbatim_code_width = cumulative_width(verbatim_line, code.start + code.len)
        - cumulative_width(verbatim_line, code.start);
    assert!((natural_code_width - verbatim_code_width).abs() < 0.01);
    assert!(natural_line.text.contains("<span title=\"中文A\">X</span>"));
    assert!(natural_line.text.contains("https://example.com"));
    let html_start = natural_line.text.find("<span").expect("HTML literal starts");
    let html_end = natural_line.text.find("</span>").expect("HTML literal ends") + "</span>".len();
    let natural_html_width =
        cumulative_width(natural_line, html_end) - cumulative_width(natural_line, html_start);
    let verbatim_html_width =
        cumulative_width(verbatim_line, html_end) - cumulative_width(verbatim_line, html_start);
    assert!((natural_html_width - verbatim_html_width).abs() < 0.01);
    let url_start = natural_line.text.find("https://example.com").expect("autolink text");
    let url_end = url_start + "https://example.com".len();
    let natural_url_width =
        cumulative_width(natural_line, url_end) - cumulative_width(natural_line, url_start);
    let verbatim_url_width =
        cumulative_width(verbatim_line, url_end) - cumulative_width(verbatim_line, url_start);
    assert!((natural_url_width - verbatim_url_width).abs() < 0.01);
    let html_block = "<script>\nconst label = \"<span title=\\\"中文A\\\">\";\n</script>";
    let html_block_natural =
        line_widths_for_mode(html_block, ui::typography::TextSpacingMode::Natural);
    let html_block_verbatim =
        line_widths_for_mode(html_block, ui::typography::TextSpacingMode::Verbatim);
    assert_eq!(html_block_natural.len(), html_block_verbatim.len());
    for (natural, verbatim) in html_block_natural.iter().zip(&html_block_verbatim) {
        assert!((natural - verbatim).abs() < 0.01);
    }
    assert!(
        natural_line.text_layout.as_ref().expect("line layout exists").shaped.width
            > verbatim_line.text_layout.as_ref().expect("line layout exists").shaped.width
    );
}

#[test]
fn paragraph_first_line_indent_applies_once_across_soft_break_and_to_empty_paragraph() {
    for source in ["第一行\n第二行", "\n"] {
        let mut style = default_style();
        style.paragraph_first_line_indent = style.body_font_size * 2.0;
        let parsed = parse_markdown(source);
        let markdown = MarkdownDoc::build_for_editing(&parsed, &style, source);
        let document = core::document::StringDocView::new(source);
        let layout = layout_doc(&markdown.blocks, &style, 400.0, &document);
        let LaidOutBlockKind::Text { lines } = &layout.blocks[0].kind else {
            panic!("fixture must produce editable paragraph");
        };
        assert_eq!(lines[0].rect.x, style.paragraph_first_line_indent);
        assert!(lines[1..].iter().all(|line| line.rect.x == 0.0));
    }
}

#[test]
fn paragraph_first_line_indent_leaves_room_for_a_character_in_narrow_viewports() {
    let source = "一二三四五六";
    let mut style = default_style();
    style.paragraph_first_line_indent = style.body_font_size * 2.0;
    let (_, markdown) = make_doc(source);
    let document = core::document::StringDocView::new(source);
    let mut shaper = Shaper::new().expect("layout test requires a system font shaper");
    let viewport_width = style.body_font_size * 2.0;
    let layout = layout_doc_with_shaper(
        &markdown.blocks,
        &style,
        viewport_width,
        Some(&mut shaper),
        None,
        &document,
    );
    let LaidOutBlockKind::Text { lines } = &layout.blocks[0].kind else {
        panic!("fixture must produce paragraph text");
    };
    assert!(lines[0].rect.w >= style.body_font_size);
    assert!(lines.iter().all(|line| line.rect.x + line.rect.w <= viewport_width));
    assert_eq!(lines.iter().map(|line| line.text.as_str()).collect::<String>(), source);
}

#[test]
fn list_item_text_and_child_paragraph_have_paragraph_spacing() {
    // 手工构造“项文本 + 子段落”的 loose 项：当前 parser 会把 loose 项的
    // 段落全部归入 children，该形状只可能由其他 BlockSource 产生；一旦出现，
    // 项文本与子块之间必须有 paragraph_spacing，而不是零间距粘连。
    let style = default_style();
    let (_, child_doc) = make_doc("child");
    let child =
        child_doc.blocks.into_iter().next().expect("fixture must produce a paragraph block");
    let item = BlockNode {
        kind: BlockKind::ListItem {
            bullet: crate::builder::ListBullet::Bullet,
            tight: false,
            blank_line_before: false,
        },
        children: vec![child],
        text_lines: vec!["a".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
        source_range: crate::builder::BlockSource::Continuous(0..2),
        block_range: 0..2,
        code_line_source_starts: None,
    };
    let doc_view = core::document::StringDocView::new("a");
    let mut ctx = LayoutCtx::new(&doc_view, &style, 400.0, None, None, None, None);

    layout_block(&item, &mut ctx);

    let Some(item_block) = ctx.output.into_iter().next() else {
        panic!("list item must produce a laid-out block");
    };
    let LaidOutBlockKind::ListItem { lines, blocks, .. } = &item_block.kind else {
        panic!("laid-out block must be a list item");
    };
    let text_bottom = lines
        .last()
        .map(|line| line.rect.y + line.rect.h)
        .expect("fixture item must lay out its own text");
    let child_top = blocks.first().expect("fixture item must contain the child").rect.y;
    assert!(
        (child_top - text_bottom - style.paragraph_spacing).abs() < 0.01,
        "gap between item text and child block {} must equal paragraph_spacing ({})",
        child_top - text_bottom,
        style.paragraph_spacing
    );
}

fn find_line_in_block<'a>(block: &'a LaidOutBlock, needle: &str) -> Option<&'a LaidOutLine> {
    match &block.kind {
        LaidOutBlockKind::Text { lines }
        | LaidOutBlockKind::MetadataBlock { lines }
        | LaidOutBlockKind::CodeBlock { lines, .. } => {
            lines.iter().find(|line| line.text.contains(needle))
        }
        LaidOutBlockKind::BlockQuote { blocks } => {
            blocks.iter().find_map(|child| find_line_in_block(child, needle))
        }
        LaidOutBlockKind::ListItem { lines, blocks, .. } => lines
            .iter()
            .find(|line| line.text.contains(needle))
            .or_else(|| blocks.iter().find_map(|child| find_line_in_block(child, needle))),
        LaidOutBlockKind::Table { header, rows, .. } => header
            .iter()
            .flatten()
            .chain(rows.iter().flatten().flatten())
            .find(|line| line.text.contains(needle)),
        LaidOutBlockKind::HorizontalRule | LaidOutBlockKind::Embedded { .. } => None,
    }
}

fn nested_line_displayed_y(source: &str, needle: &str) -> f32 {
    let cursor_byte = source.find(needle).expect("fixture must contain the target line");
    let layout = layout_with_cursor_and_width(source, cursor_byte, 400.0);
    layout
        .laid_out
        .iter()
        .enumerate()
        .find_map(|(slot, block)| {
            let line = find_line_in_block(block.as_ref()?, needle)?;
            Some(line.rect.y + layout.y_delta[slot])
        })
        .expect("fixture must lay out the target line")
}

#[test]
fn nested_list_blockquote_blank_run_is_reserved_once() {
    let style = default_style();
    let fixtures = [("- a\n\n  > b", "- a\n\n\n  > b"), ("> a\n>\n> - b", "> a\n>\n>\n> - b")];

    for (single_blank_source, double_blank_source) in fixtures {
        let single_blank_y = nested_line_displayed_y(single_blank_source, "b");
        let double_blank_y = nested_line_displayed_y(double_blank_source, "b");
        let editable_paragraph_extent = style.line_height + style.paragraph_spacing;
        assert!(
            (double_blank_y - single_blank_y - editable_paragraph_extent).abs() < 0.01,
            "one additional editable blank paragraph must move b by its line height and \
             trailing spacing for {double_blank_source:?}: single={single_blank_y}, \
             double={double_blank_y}"
        );
    }
}

#[test]
fn separated_nested_blank_runs_are_each_reserved_once() {
    let style = default_style();
    let full_source = "> intro\n>\n>\n> - first\n>\n> separator\n>\n>\n> - second";
    let first_run_shortened = "> intro\n>\n> - first\n>\n> separator\n>\n>\n> - second";
    let second_run_shortened = "> intro\n>\n>\n> - first\n>\n> separator\n>\n> - second";
    let parsed = parse_markdown(full_source);
    let document = MarkdownDoc::build(&parsed, &style);

    assert_eq!(document.blocks.len(), 1, "fixture must use one top-level LayoutCtx");
    assert!(
        matches!(document.blocks[0].kind, BlockKind::BlockQuote),
        "fixture must keep both nested blank runs inside one top-level blockquote"
    );

    let full_first_y = nested_line_displayed_y(full_source, "first");
    let shortened_first_y = nested_line_displayed_y(first_run_shortened, "first");
    let editable_paragraph_extent = style.line_height + style.paragraph_spacing;
    assert!(
        (full_first_y - shortened_first_y - editable_paragraph_extent).abs() < 0.01,
        "the first nested blank run must reserve its editable paragraph"
    );

    let full_second_y = nested_line_displayed_y(full_source, "second");
    let shortened_second_y = nested_line_displayed_y(second_run_shortened, "second");
    assert!(
        (full_second_y - shortened_second_y - editable_paragraph_extent).abs() < 0.01,
        "the second nested blank run must reserve its editable paragraph independently"
    );
}

fn layout_with_cursor_and_width(
    source: &str,
    cursor_byte: usize,
    width: f32,
) -> LazyLayout<crate::builder::MarkdownDoc> {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build_for_editing(&parsed, &style, source);
    let doc_view = core::document::StringDocView::new(source);
    let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
    lazy.set_edit_source(Some(source.to_string()));
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    let mut shaper = Shaper::new().expect("list projection test needs a shaper");
    lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
    lazy.build_flat_lines(&doc_view);
    lazy
}

fn layout_with_inline_code_preedit(source: &str, width: f32) -> LazyLayout<MarkdownDoc> {
    let cursor_byte = source.find("b`").expect("fixture contains code suffix");
    let mut layout = layout_with_cursor_and_width(source, cursor_byte, width);
    layout.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: Some("中文".to_owned()),
        preedit_cursor: Some(("中文".len(), "中文".len())),
    }));
    layout.invalidate_lines_for_source_bytes([cursor_byte]);
    let mut shaper = Shaper::new().expect("inline code preedit test needs a shaper");
    layout.ensure_precise_range(
        0.0,
        600.0,
        &default_style(),
        &mut shaper,
        None,
        &core::document::StringDocView::new(source),
    );
    layout
}

#[test]
fn focused_inline_code_preedit_keeps_content_and_marker_styles_aligned() {
    use crate::builder::InlineStyle;

    for prefix in ["", "# ", "- "] {
        let source = format!("{prefix}before `ab` after");
        let layout = layout_with_inline_code_preedit(&source, 400.0);
        let line = layout
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::Text { lines } | LaidOutBlockKind::ListItem { lines, .. } => {
                    lines.first()
                }
                _ => None,
            })
            .expect("fixture must lay out an editable text line");
        let visible_prefix = if prefix == "# " { "" } else { prefix };
        assert_eq!(line.text, format!("{visible_prefix}before `a中文b` after"));
        let styled_text = line
            .styles
            .iter()
            .map(|span| (&span.style, &line.text[span.start..span.start + span.len]))
            .filter(|(style, text)| **style == InlineStyle::InlineCode || *text == "`")
            .collect::<Vec<_>>();
        assert_eq!(
            styled_text,
            [
                (&InlineStyle::SourceMarker, "`"),
                (&InlineStyle::InlineCode, "a中文b"),
                (&InlineStyle::SourceMarker, "`"),
            ]
        );
        for marker in line.styles.iter().filter(|span| span.style == InlineStyle::SourceMarker) {
            let segment = line
                .style_segments
                .iter()
                .find(|segment| segment.start == marker.start && segment.len == marker.len)
                .expect("source marker must retain a final geometry segment");
            let shaped_width = cumulative_width(line, marker.start + marker.len)
                - cumulative_width(line, marker.start);
            assert!((segment.width - shaped_width).abs() < 0.01);
        }
    }
}

#[test]
fn wrapped_table_inline_code_preedit_stays_in_the_focused_cell() {
    let source = "| left | right |\n| --- | --- |\n| many words `ab` here | `other` |";
    let mut layout = layout_with_inline_code_preedit(source, 160.0);
    layout.build_flat_lines(&core::document::StringDocView::new(source));
    let rows = layout
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::Table { rows, .. } => Some(rows),
            _ => None,
        })
        .expect("fixture must lay out a table");
    let focused_lines = &rows[0][0];
    assert!(focused_lines.len() > 1, "narrow cell must exercise wrapping");
    let code_text = focused_lines
        .iter()
        .flat_map(|line| {
            line.styles
                .iter()
                .filter(|span| span.style == crate::builder::InlineStyle::InlineCode)
                .map(|span| &line.text[span.start..span.start + span.len])
        })
        .collect::<String>();
    assert_eq!(code_text, "a中文b");
    for line in focused_lines {
        assert_eq!(
            line.source_projection.as_ref().expect("wrapped cell requires a projection").owner,
            crate::projection::ProjectionOwnerId::TableCell {
                table_start: 0,
                row: 1,
                column: 0,
                logical_line: 0,
            },
        );
    }
    let adjacent_text = rows[0][1].iter().map(|line| line.text.as_str()).collect::<String>();
    assert_eq!(adjacent_text, "other", "adjacent code must remain folded without preedit");
}

#[test]
fn table_width_and_plain_cell_wrap_stay_stable_when_focus_moves() {
    let source = "| code | ordinary text |\n| --- | --- |\n| `alpha` | several ordinary words that should wrap consistently across the cell |";
    let first_cursor = source.find("alpha").expect("fixture contains inline code");
    let second_cursor = source.find("ordinary words").expect("fixture contains plain text");
    let style = default_style();
    let mut layout = layout_with_cursor_and_width(source, first_cursor, 220.0);
    let doc_view = core::document::StringDocView::new(source);
    let table_layout = |layout: &LazyLayout<MarkdownDoc>| {
        layout
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::Table { column_widths, rows, .. } => Some((
                    column_widths.clone(),
                    rows[0][1].iter().map(|line| line.text.clone()).collect::<Vec<_>>(),
                )),
                _ => None,
            })
            .expect("fixture lays out a table")
    };
    let (initial_widths, initial_plain_lines) = table_layout(&layout);

    layout.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: second_cursor,
        preedit_text: None,
        preedit_cursor: None,
    }));
    layout.invalidate_lines_for_source_bytes([first_cursor, second_cursor]);
    let mut shaper = Shaper::new().expect("table focus test requires a system font shaper");
    layout.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
    layout.build_flat_lines(&doc_view);
    let (focused_widths, focused_plain_lines) = table_layout(&layout);

    assert_eq!(focused_widths, initial_widths, "focus movement must preserve table columns");
    assert_eq!(
        focused_plain_lines, initial_plain_lines,
        "ordinary text in the table must keep the same wrapping after focus movement"
    );
}

#[test]
fn empty_quote_marker_preserves_physical_source_boundaries() {
    for newline in ["\n", "\r\n"] {
        for prefix in [">", "> ", ">  ", ">\t"] {
            let source = format!("> first{newline}{prefix}{newline}{prefix}{newline}> second");
            let marker_start = "> first".len() + newline.len() * 2 + prefix.len();
            let cursor = marker_start + prefix.len();
            let layout = layout_with_cursor_and_width(&source, cursor, 400.0);
            let line = layout
                .flat_lines
                .iter()
                .find(|line| {
                    line.source_projection.as_ref().is_some_and(|projection| {
                        projection.owner
                            == crate::projection::ProjectionOwnerId::EmptyLine {
                                source_byte: cursor,
                            }
                    })
                })
                .expect("the second quoted blank line must be an editable paragraph");
            assert_eq!(line.text, prefix, "source marker must not invent whitespace: {source:?}");
            let projection = line.source_projection.as_ref().expect("empty paragraph projection");
            assert_eq!(
                projection.source_extent,
                marker_start..cursor,
                "quote marker must never consume the preceding newline: {source:?}"
            );
            let index =
                layout.source_projection_index.as_ref().expect("valid source projection index");
            for source_byte in marker_start..=cursor {
                let position = index
                    .visual_position_for_source(
                        source_byte,
                        crate::projection::CursorAffinity::Downstream,
                    )
                    .expect("every actual quote marker boundary must have a visual position");
                assert_eq!(
                    index
                        .source_anchor_at(0, position)
                        .expect("current projection generation must resolve")
                        .byte,
                    source_byte
                );
            }
        }
    }
}

fn layout_with_selection_and_width(
    source: &str,
    cursor_byte: usize,
    selection_range: std::ops::Range<usize>,
    width: f32,
) -> LazyLayout<crate::builder::MarkdownDoc> {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
    lazy.set_edit_source(Some(source.to_string()));
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    lazy.set_selection_range(Some(selection_range));
    let mut shaper = Shaper::new().expect("selection layout test needs a shaper");
    lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
    lazy.build_flat_lines(&doc_view);
    lazy
}

fn layout_doc_with_width(source: &str, width: f32) -> LaidOutDoc {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut shaper = Shaper::new().expect("table projection test needs a shaper");
    layout_doc_with_shaper(&doc.blocks, &style, width, Some(&mut shaper), None, &doc_view)
}

fn layout_doc_with_registry_and_width(source: &str, width: f32) -> MarkdownLayout {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut shaper = Shaper::new().expect("ASCII diagram test needs a shaper");
    layout_doc_with_shaper_for_rendering(
        &doc.blocks,
        &style,
        width,
        Some(&mut shaper),
        None,
        &doc_view,
    )
}

#[test]
fn layout_marks_non_active_box_diagram_code_block() {
    let laid_out = layout_doc_with_registry_and_width(ASCII_DIAGRAM_SOURCE, 400.0);
    let block = laid_out.doc.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };
    assert_eq!(
        laid_out.ascii_diagrams.diagram_for(lines).map(|diagram| diagram.column_count),
        Some(6)
    );
}

#[test]
fn layout_does_not_mark_indented_box_diagram_code_block() {
    let laid_out = layout_doc_with_registry_and_width(INDENTED_ASCII_DIAGRAM_SOURCE, 400.0);
    let block = laid_out.doc.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };
    assert!(
        laid_out.ascii_diagrams.diagram_for(lines).is_none(),
        "only fenced code blocks may enable the grid path"
    );
}

#[test]
fn active_box_diagram_code_block_keeps_normal_layout_path() {
    let cursor_byte = ASCII_DIAGRAM_SOURCE.find("中文").expect("fixture has CJK label");
    let layout = layout_with_cursor_and_width(ASCII_DIAGRAM_SOURCE, cursor_byte, 400.0);
    let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };
    assert!(
        layout.ascii_diagrams().diagram_for(lines).is_none(),
        "active code blocks must keep the existing path"
    );
}

#[test]
fn active_indented_code_with_blank_lines_aligns_content() {
    let source = "    2400w出货, 小米有1.3kw出货量???\n    \n    国际版没上,设备大头是联想.年底有1300w出货.\n    \n    三星国际预装拿下.";
    let cursor_byte = source.find("2400w").expect("source contains the first code line");
    let layout = layout_with_cursor_and_width(source, cursor_byte, 800.0);
    let block = layout.laid_out[0].as_ref().expect("code block must be visible");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("source must produce an indented code block");
    };
    assert_eq!(
        lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
        [
            "2400w出货, 小米有1.3kw出货量???",
            "",
            "国际版没上,设备大头是联想.年底有1300w出货.",
            "",
            "三星国际预装拿下.",
        ]
    );
}

#[test]
fn selection_first_materialization_keeps_fence_lines_out_of_diagram_and_restores_grid() {
    let source = format!("{ASCII_DIAGRAM_SOURCE}\noutside");
    let selection_start = source.find("中文").expect("fixture has diagram text");
    let parsed = crate::parser::parse_markdown(&source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(&source);
    let mut layout = LazyLayout::new(doc, &style, 400.0, &doc_view);
    layout.set_edit_source(Some(source.clone()));
    layout.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: source.len(),
        preedit_text: None,
        preedit_cursor: None,
    }));
    layout.set_selection_range(Some(selection_start..source.len()));
    let mut shaper = Shaper::new().expect("selection materialization test needs a shaper");
    layout.ensure_visible(0.0, 600.0, &style, 400.0, &mut shaper, None, &doc_view);

    {
        let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };

        assert_ne!(lines.first().map(|line| line.text.as_str()), Some("```"));
        assert!(
            layout.ascii_diagrams().diagram_for(lines).is_none(),
            "selection must disable only grid rendering"
        );
    }

    layout.set_selection_range(None);

    let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };

    assert_eq!(
        layout.ascii_diagrams().diagram_for(lines).map(|diagram| diagram.rows.len()),
        Some(3),
        "clearing selection must restore the original three-content-line diagram"
    );
}

#[test]
fn selection_starting_inside_diagram_keeps_code_block_on_text_path() {
    let source = format!("{ASCII_DIAGRAM_SOURCE}\noutside");
    let selection_end = source.find("中文").expect("fixture has diagram text");
    let layout =
        layout_with_selection_and_width(&source, selection_end, selection_end..source.len(), 400.0);
    let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };

    assert_eq!(lines.first().map(|line| line.text.as_str()), Some("```"));
}

#[test]
fn zero_width_selection_keeps_non_active_diagram_grid_path() {
    let source = format!("{ASCII_DIAGRAM_SOURCE}\noutside");
    let selection_start = source.find("中文").expect("fixture has diagram text");
    let layout = layout_with_selection_and_width(
        &source,
        source.len(),
        selection_start..selection_start,
        400.0,
    );
    let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };

    assert_ne!(lines.first().map(|line| line.text.as_str()), Some("```"));
    assert!(
        layout.ascii_diagrams().diagram_for(lines).is_some(),
        "zero-width selection must preserve the grid path"
    );
}

fn collect_laid_out_text(block: &LaidOutBlock, out: &mut String) {
    match &block.kind {
        LaidOutBlockKind::Text { lines } => {
            for line in lines {
                out.push_str(&line.text);
            }
        }
        LaidOutBlockKind::CodeBlock { lines, .. } => {
            for line in lines {
                out.push_str(&line.text);
            }
        }
        LaidOutBlockKind::Table { header, rows, .. } => {
            for cell in header {
                for line in cell {
                    out.push_str(&line.text);
                }
            }
            for row in rows {
                for cell in row {
                    for line in cell {
                        out.push_str(&line.text);
                    }
                }
            }
        }
        LaidOutBlockKind::ListItem { lines, blocks, .. } => {
            for line in lines {
                out.push_str(&line.text);
            }
            for b in blocks {
                collect_laid_out_text(b, out);
            }
        }
        LaidOutBlockKind::BlockQuote { blocks } => {
            for b in blocks {
                collect_laid_out_text(b, out);
            }
        }
        LaidOutBlockKind::HorizontalRule | LaidOutBlockKind::Embedded { .. } => {}
        LaidOutBlockKind::MetadataBlock { lines } => {
            for l in lines {
                out.push_str(&l.text);
                out.push('\n');
            }
        }
    }
}

#[test]
fn layout_paragraph_has_rect() {
    let (src, doc) = make_doc("hello world");
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    assert!(!laid_out.blocks.is_empty());
    assert!(laid_out.blocks[0].rect.w > 0.0);
    assert!(laid_out.blocks[0].rect.h > 0.0);
}

#[test]
fn plain_shaped_text_keeps_projection_without_legacy_source_map() {
    let source = "plain paragraph";
    let (_, doc) = make_doc(source);
    let mut shaper = Shaper::new().expect("plain layout regression test needs a shaper");
    let laid_out = layout_doc_with_shaper(
        &doc.blocks,
        &default_style(),
        400.0,
        Some(&mut shaper),
        None,
        &core::document::StringDocView::new(source),
    );
    let lines = match &laid_out.blocks[0].kind {
        LaidOutBlockKind::Text { lines } => lines,
        _ => panic!("plain paragraph must lay out as text"),
    };

    assert!(
        lines.iter().all(|line| line.source_projection.is_some()),
        "plain shaped text must retain its canonical source projection"
    );
}

#[test]
fn wrapped_table_cells_keep_distinct_source_extents() {
    let source = "| left header | right header |\n| --- | --- |\n| left body wraps here | right body wraps here |";
    let laid = layout_doc_with_width(source, 220.0);
    let left = laid
        .blocks
        .iter()
        .find_map(|block| find_line_in_block(block, "left body"))
        .expect("left body must be laid out");
    let right = laid
        .blocks
        .iter()
        .find_map(|block| find_line_in_block(block, "right body"))
        .expect("right body must be laid out");
    let left_projection = left.source_projection.as_ref().expect("left projection");
    let right_projection = right.source_projection.as_ref().expect("right projection");

    assert_ne!(left_projection.owner, right_projection.owner);
    assert!(left_projection.source_extent.end <= right_projection.source_extent.start);
}

#[test]
fn multiline_list_item_projection_collapses_continuation_indent() {
    let source = "- first line\n  continuation line";
    let continuation = source.find("continuation").expect("fixture contains continuation");
    let lazy = layout_with_cursor_and_width(source, continuation, 400.0);
    let list_lines = lazy
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::ListItem { lines, .. } => Some(lines),
            _ => None,
        })
        .expect("fixture must have a laid-out list item");
    assert_eq!(
        list_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
        ["- first line continuation line"],
        "a list softbreak must remain in one logical line"
    );
    let marker_projection = list_lines[0].source_projection.as_ref().expect("marker projection");
    assert_eq!(
        marker_projection.boundaries[0].byte, 0,
        "the active list marker must anchor to its source byte"
    );
    let continuation_boundary = marker_projection
        .collapsed
        .first()
        .expect("the logical line must retain the collapsed softbreak");
    assert!(
        continuation_boundary.source_range.start < continuation
            && continuation_boundary.source_range.end <= continuation,
        "collapsed source range must cover the newline and continuation indent"
    );
    let line = lazy
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| find_line_in_block(block, "continuation"))
        .expect("continuation must have a laid-out line");

    assert!(
        line.source_projection
            .as_ref()
            .expect("projection")
            .boundaries
            .iter()
            .any(|anchor| anchor.byte == continuation)
    );
}

#[test]
fn nested_list_cursor_projection_belongs_to_inner_item() {
    let source = "- outer\n  - inner wrapped content wrapped content";
    let inner = source.find("inner").expect("fixture contains inner");
    let lazy = layout_with_cursor_and_width(source, inner, 140.0);

    assert!(
        lazy.flat_lines
            .iter()
            .filter(|line| line.text.contains("inner"))
            .all(|line| line.source_projection.is_some())
    );
}

#[test]
fn nested_list_continuation_stays_logical_and_retains_collapsed_indent() {
    let source = "- outer\n  - **first\n    second**";
    let first = source.find("first").expect("fixture contains first line text");
    let second = source.find("second").expect("fixture contains continuation text");
    let continuation_newline =
        source[..second].rfind('\n').expect("fixture contains the continuation newline");
    let lazy = layout_with_cursor_and_width(source, second, 400.0);
    let nested_lines = lazy
        .flat_lines
        .iter()
        .filter(|line| line.text.contains("first") || line.text.contains("second"))
        .collect::<Vec<_>>();

    assert_eq!(
        nested_lines.len(),
        1,
        "nested-list softbreaks must stay in one logical line, got {:?}",
        nested_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>()
    );
    assert!(
        nested_lines.iter().all(|line| !line.text.is_empty() && line.source_projection.is_some()),
        "the nested-list logical line must keep its source projection"
    );

    let first_line = nested_lines
        .iter()
        .find(|line| line.text.contains("first"))
        .expect("the nested list line must be present");
    let collapsed = first_line
        .source_projection
        .as_ref()
        .expect("the nested list line needs a projection")
        .collapsed
        .first()
        .expect("the nested list line must retain its collapsed continuation");

    assert!(
        collapsed.source_range.start <= continuation_newline
            && collapsed.source_range.end >= second,
        "the collapsed range must cover the physical newline and continuation indent"
    );
    assert!(
        source[collapsed.source_range.clone()].starts_with("\n    "),
        "the collapsed range must retain the newline plus nested-list continuation indent"
    );
    assert!(first < second, "the fixture must keep the first line before its continuation");
}

#[test]
fn styled_list_continuation_retains_collapsed_source_boundary() {
    let source = "- **first\n  second**";
    let second = source.find("second").expect("fixture contains continuation text");
    let lazy = layout_with_cursor_and_width(source, second, 400.0);
    let list_lines = lazy
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::ListItem { lines, .. } => Some(lines),
            _ => None,
        })
        .expect("fixture must have a laid-out list item");
    let continuation = list_lines
        .iter()
        .find(|line| line.text.contains("second"))
        .expect("continuation must have a laid-out line");

    assert_eq!(
        list_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
        ["- **first second**"],
        "active inline markers must preserve the collapsed logical line"
    );
    assert!(
        continuation
            .source_projection
            .as_ref()
            .expect("continuation projection")
            .boundaries
            .iter()
            .any(|anchor| anchor.byte == second),
        "continuation projection must retain the continuation source anchor"
    );
    let continuation_projection =
        continuation.source_projection.as_ref().expect("continuation projection");
    assert!(
        continuation_projection.source_extent.start <= second,
        "continuation projection must cover its collapsed source boundary"
    );
    assert!(
        continuation_projection
            .collapsed
            .iter()
            .any(|collapsed| collapsed.source_range.start < second
                && collapsed.source_range.end <= second),
        "continuation projection must retain the newline and indentation mapping"
    );
    let bold = continuation
        .styles
        .iter()
        .find(|span| matches!(span.style, crate::builder::InlineStyle::Bold))
        .expect("continuation text must retain its bold style");
    assert_eq!(
        &continuation.text[bold.start..bold.start + bold.len],
        "first second",
        "the complete projected text must remain bold"
    );
    assert_eq!(
        continuation
            .styles
            .iter()
            .filter(|span| matches!(span.style, crate::builder::InlineStyle::SourceMarker))
            .map(|span| &continuation.text[span.start..span.start + span.len])
            .collect::<Vec<_>>(),
        ["- ", "**", "**"],
        "materialization must classify the list and inline delimiters as source markers"
    );
}

#[test]
fn metadata_projection_starts_at_content_event_range() {
    let source = "---\ntitle: hello\n---";
    let (_, doc) = make_doc(source);
    let laid_out = layout_doc(
        &doc.blocks,
        &default_style(),
        400.0,
        &core::document::StringDocView::new(source),
    );
    let metadata_lines = laid_out
        .blocks
        .iter()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::MetadataBlock { lines } => Some(lines),
            _ => None,
        })
        .expect("frontmatter fixture must produce a metadata block");
    let projection = metadata_lines[0]
        .source_projection
        .as_ref()
        .expect("metadata lines require a source projection");

    assert_eq!(
        projection.boundaries[0].byte,
        source.find("title: hello").expect("fixture must contain metadata content"),
    );
}

#[test]
fn metadata_projection_uses_each_physical_line_event_range() {
    let source = "---\ntitle: hello\nauthor: textora\n---";
    let (_, doc) = make_doc(source);
    let laid_out = layout_doc(
        &doc.blocks,
        &default_style(),
        400.0,
        &core::document::StringDocView::new(source),
    );
    let metadata_lines = laid_out
        .blocks
        .iter()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::MetadataBlock { lines } => Some(lines),
            _ => None,
        })
        .expect("frontmatter fixture must produce a metadata block");

    for line in metadata_lines {
        let projection =
            line.source_projection.as_ref().expect("metadata lines require a source projection");
        assert_eq!(
            projection.boundaries[0].byte,
            source.find(&line.text).expect("metadata line must occur in source"),
        );
    }
}

#[test]
fn layout_heading_larger_than_paragraph() {
    let (h1_src, h1) = make_doc("# Big Title");
    let (p_src, p) = make_doc("small text");
    let h1_layout = layout_doc(
        &h1.blocks,
        &default_style(),
        400.0,
        &core::document::StringDocView::new(h1_src),
    );
    let p_layout =
        layout_doc(&p.blocks, &default_style(), 400.0, &core::document::StringDocView::new(p_src));
    assert!(h1_layout.blocks[0].rect.h > p_layout.blocks[0].rect.h);
}

#[test]
fn h1_ascii_text_wraps_at_correct_width() {
    let long_h1 =
        "# This is a very long heading that should wrap into multiple lines in a narrow viewport";
    let style = default_style();
    let h1_font_size = style.heading_font_sizes[0];
    let body_font_size = style.body_font_size;
    assert!(h1_font_size > body_font_size * 1.5, "H1 must be significantly larger than body");

    let (src, doc) = make_doc(long_h1);
    let laid_out = layout_doc(&doc.blocks, &style, 200.0, &core::document::StringDocView::new(src));
    let block = &laid_out.blocks[0];
    if let LaidOutBlockKind::Text { lines } = &block.kind {
        assert!(
            lines.len() >= 3,
            "H1 long ASCII text should wrap to 3+ lines at 200px viewport, got {} lines.                  Likely ascii_widths not scaled for heading font size.",
            lines.len()
        );
    } else {
        panic!("expected Text block for heading");
    }
}

#[test]
fn layout_vertical_positions_increase() {
    let (src, doc) = make_doc("# A\n\n## B\n\nhello");
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 3);
    for i in 1..laid_out.blocks.len() {
        assert!(
            laid_out.blocks[i].rect.y >= laid_out.blocks[i - 1].rect.y,
            "block {} should be below block {}",
            i,
            i - 1
        );
    }
}

#[test]
fn layout_total_height() {
    let (src, doc) = make_doc("# Title\n\nparagraph text here\n\n## Another heading");
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.total_height > 0.0);
}

#[test]
fn style_segments_empty_for_plain_text() {
    let (src, doc) = make_doc("hello world");
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let block = &laid_out.blocks[0];
    if let LaidOutBlockKind::Text { lines } = &block.kind {
        // Plain paragraph has no styled spans
        assert!(lines[0].style_segments.is_empty(), "plain text should have no style segments");
    }
}

#[test]
fn style_segments_computed_for_inline_code() {
    let (src, doc) = make_doc("use `code` here");
    let style = default_style();
    let laid_out = layout_doc_with_shaper(
        &doc.blocks,
        &style,
        400.0,
        None,
        None,
        &core::document::StringDocView::new(src),
    );
    let block = &laid_out.blocks[0];
    if let LaidOutBlockKind::Text { lines } = &block.kind {
        // Without shaper, style_segments should be empty (fallback)
        assert!(lines[0].style_segments.is_empty(), "no shaper -> no segments");
    }
}

#[test]
fn layout_cjk_text_no_panic() {
    let (src, doc) = make_doc(
        "需保留 SidebarPersistent 机制（inject_persistent 在生产中使用，用于跨帧保持隐藏/显示状态）",
    );
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 200.0, &core::document::StringDocView::new(src));
    assert!(!laid_out.blocks.is_empty());
    assert!(laid_out.total_height > 0.0);
}

#[test]
fn layout_long_cjk_line_wraps() {
    let long = "这是一段很长的中文文本用来测试自动换行功能是否正常工作不会产生恐慌";
    let (src, doc) = make_doc(long);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 150.0, &core::document::StringDocView::new(src));
    // Should wrap into multiple lines
    let text_block =
        laid_out.blocks.iter().find(|b| matches!(&b.kind, LaidOutBlockKind::Text { .. }));
    assert!(text_block.is_some());
}

#[test]
fn heading_spacing_collapses_between_adjacent_headings() {
    let (src, doc) = make_doc("## First\n### Second");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2, "should have 2 heading blocks");
    let h1_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
    let h2_top = laid_out.blocks[1].rect.y;
    let gap = h2_top - h1_bottom;
    // Adjacent headings: gap should be heading_spacing_bottom, not
    // heading_spacing_bottom + heading_spacing_top
    let expected_max = style.heading_spacing_bottom + 1.0; // small tolerance
    assert!(
        gap <= expected_max,
        "adjacent heading gap {} should collapse to ~{}, not {}",
        gap,
        style.heading_spacing_bottom,
        style.heading_spacing_bottom + style.heading_spacing_top
    );
}

#[test]
fn first_heading_top_spacing_halved() {
    let (src, doc) = make_doc("# Title");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(!laid_out.blocks.is_empty());
    let block = &laid_out.blocks[0];
    // First heading's y should be heading_spacing_top * 0.5
    let expected_y = style.heading_spacing_top * 0.5;
    assert!(
        (block.rect.y - expected_y).abs() < 1.0,
        "first heading y={} should be ~{}",
        block.rect.y,
        expected_y
    );
}

#[test]
fn non_first_heading_has_full_top_spacing() {
    let (src, doc) = make_doc("paragraph\n\n# Heading");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2);
    let heading = &laid_out.blocks[1];
    let para_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
    let gap = heading.rect.y - para_bottom;
    // Should include heading_spacing_top (not collapsed, not halved)
    assert!(
        gap >= style.heading_spacing_top - 1.0,
        "non-first heading gap {} should include full top spacing {}",
        gap,
        style.heading_spacing_top
    );
}

#[test]
fn blockquote_text_color_is_faded() {
    let (src, doc) = make_doc("> quoted text");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(!laid_out.blocks.is_empty());
    // The blockquote block should contain child text blocks with faded color
    if let LaidOutBlockKind::BlockQuote { blocks } = &laid_out.blocks[0].kind {
        assert!(!blocks.is_empty(), "blockquote should have child blocks");
        if let LaidOutBlockKind::Text { lines } = &blocks[0].kind {
            let faded = lines[0].color_override.unwrap();
            let base = style.text_color;
            // Faded color should differ from base text color
            assert_ne!(faded, base, "blockquote text should be faded");
            // Alpha must remain 1.0
            assert_eq!(faded[3], 1.0, "alpha must be 1.0 for subpixel rendering");
            // RGB should be shifted toward background
            let bg = style.background_color;
            // Faded R should be between base R and bg R
            if base[0] < bg[0] {
                assert!(faded[0] > base[0] && faded[0] <= bg[0]);
            } else {
                assert!(faded[0] < base[0] && faded[0] >= bg[0]);
            }
        } else {
            panic!("blockquote child should be Text");
        }
    } else {
        panic!("first block should be BlockQuote");
    }
}

#[test]
fn blockquote_nested_in_list_preserves_color_fade() {
    let (src, doc) = make_doc("- > quoted in list");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    // Find the blockquote inside the list item
    fn find_blockquote(blocks: &[LaidOutBlock]) -> Option<&LaidOutBlock> {
        for b in blocks {
            if matches!(&b.kind, LaidOutBlockKind::BlockQuote { .. }) {
                return Some(b);
            }
            if let LaidOutBlockKind::ListItem { blocks: sub, .. } = &b.kind
                && let Some(found) = find_blockquote(sub)
            {
                return Some(found);
            }
        }
        None
    }
    let bq = find_blockquote(&laid_out.blocks).expect("should find blockquote in list");
    if let LaidOutBlockKind::BlockQuote { blocks } = &bq.kind
        && let LaidOutBlockKind::Text { lines } = &blocks[0].kind
    {
        let faded = lines[0].color_override.unwrap();
        let base = style.text_color;
        assert_ne!(faded, base, "blockquote-in-list text should be faded");
        assert_eq!(faded[3], 1.0);
    }
}

#[test]
fn nested_list_has_increasing_depth() {
    let (src, doc) = make_doc("- top\n  - nested\n    - deep");
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    // Find all ListItem blocks and check their depth
    fn find_depths(blocks: &[LaidOutBlock], depths: &mut Vec<usize>) {
        for b in blocks {
            if let LaidOutBlockKind::ListItem { depth, blocks: sub, .. } = &b.kind {
                depths.push(*depth);
                find_depths(sub, depths);
            }
        }
    }
    let mut depths = Vec::new();
    find_depths(&laid_out.blocks, &mut depths);
    assert!(depths.len() >= 3, "should have at least 3 list items");
    assert_eq!(depths[0], 0, "top-level depth should be 0");
    assert_eq!(depths[1], 1, "nested depth should be 1");
    assert_eq!(depths[2], 2, "deeply nested depth should be 2");
}

#[test]
fn layout_table_has_rows() {
    let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let table = laid_out.blocks.iter().find_map(|b| {
        if let LaidOutBlockKind::Table { rows, .. } = &b.kind { Some(rows.len()) } else { None }
    });
    assert_eq!(table, Some(2), "table should have 2 body rows");
}

#[test]
fn layout_table_row_heights_match_row_count() {
    let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let table = laid_out.blocks.iter().find_map(|b| {
        if let LaidOutBlockKind::Table { rows, row_heights, .. } = &b.kind {
            Some((rows.len(), row_heights.len()))
        } else {
            None
        }
    });
    assert_eq!(table, Some((2, 2)), "row_heights must have one entry per row");
}

#[test]
fn layout_table_header_height_nonzero() {
    let md = "| Name | Value |\n| --- | --- |\n| x | 1 |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let header_h = laid_out.blocks.iter().find_map(|b| {
        if let LaidOutBlockKind::Table { header_height, .. } = &b.kind {
            Some(*header_height)
        } else {
            None
        }
    });
    assert!(header_h.unwrap_or(0.0) > 0.0, "header should have nonzero height");
}

#[test]
fn layout_table_row_height_reflects_long_content() {
    // Single-column table with a very long cell that should wrap
    let md = "| Long text |\n| --- |\n| this is a very long piece of text that should wrap to multiple lines in a narrow column |";
    let (src, doc) = make_doc(md);
    // Use a narrow viewport to force wrapping
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 200.0, &core::document::StringDocView::new(src));
    let row_h = laid_out.blocks.iter().find_map(|b| {
        if let LaidOutBlockKind::Table { row_heights, .. } = &b.kind {
            row_heights.first().copied()
        } else {
            None
        }
    });
    // Single line_height is 24px; wrapped content should be taller
    assert!(
        row_h.unwrap_or(0.0) > 30.0,
        "wrapped cell row height ({}) should exceed single line",
        row_h.unwrap_or(0.0)
    );
}

#[test]
fn layout_table_column_widths_sum_to_available() {
    let md = "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 | 3 |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let total_w: f32 = laid_out
        .blocks
        .iter()
        .filter_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.iter().sum::<f32>())
            } else {
                None
            }
        })
        .sum();
    // Available width is 400. Columns may not exactly equal 400 due to
    // min/max clamping and padding, but should be within 10px.
    assert!(
        (total_w - 400.0).abs() < 10.0,
        "column widths ({}) should approximately fill available width (400)",
        total_w
    );
}

#[test]
fn layout_table_wide_column_gets_more_space() {
    // Column 0: short, Column 1: very long
    let md = "| id | description |\n| --- | --- |\n| 1 | this is a very long description that needs more horizontal room |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let widths: Vec<f32> = laid_out
        .blocks
        .iter()
        .filter_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect();
    assert!(widths.len() >= 2, "expected at least 2 columns, got {}", widths.len());
    if widths.len() >= 2 {
        assert!(
            widths[1] > widths[0],
            "wide-content column 1 ({}px) should be wider than narrow column 0 ({}px)",
            widths[1],
            widths[0]
        );
    }
}

#[test]
fn table_short_cjk_cells_stay_on_one_line_beside_long_description() {
    const TABLE_WIDTH: f32 = 780.0;
    let source = "| 样式 | 笔画内部 | 笔画之间 | 用途 |\n| --- | --- | --- | --- |\n| 均匀墨迹 | 将每印记的有效覆盖率按最大值合并，限制同笔累积；整笔透明度在合成时应用一次 | 正常 source-over | 毛笔、硬笔，避免盖印结点发黑 |\n| 累积落墨 | 印记按流量累积，针对路径采样步长校正沉积量 | 正常 source-over | 铅笔、干墨颗粒 |";
    let (_, document) = make_doc(source);
    let source_view = core::document::StringDocView::new(source);
    let mut shaper = Shaper::new().expect("table layout test requires a system font shaper");
    let layout = layout_doc_with_shaper(
        &document.blocks,
        &default_style(),
        TABLE_WIDTH,
        Some(&mut shaper),
        None,
        &source_view,
    );
    let rows = layout
        .blocks
        .iter()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::Table { rows, .. } => Some(rows),
            _ => None,
        })
        .expect("fixture should produce a table");

    for row in rows {
        assert_eq!(row[0].len(), 1, "short CJK label should fit on one line");
        assert!(
            row.iter().all(|cell| cell.len() <= 2),
            "table cells should fit within two lines: {:?}",
            row.iter().map(Vec::len).collect::<Vec<_>>()
        );
    }
}

#[test]
fn table_column_demand_ignores_residual_shaper_font_state() {
    let source = "| label | description |\n| --- | --- |\n| WWWW | iiiiiiii |";
    let style = default_style();
    let (_, document) = make_doc(source);
    let table = document
        .blocks
        .iter()
        .find(|block| matches!(block.kind, BlockKind::TableWrapper { .. }))
        .expect("fixture contains a table");
    let source_view = core::document::StringDocView::new(source);
    let columns = match table.kind {
        BlockKind::TableWrapper { columns, .. } => columns,
        _ => unreachable!("selected block is a table"),
    };

    let mut clean_shaper = Shaper::new().expect("table width test requires a system font shaper");
    let clean_demand = measure_column_demand(
        table,
        columns,
        style.body_font_size,
        style.text_spacing_mode,
        style.body_font_family.first().map(String::as_str),
        Some(&mut clean_shaper),
        &source_view,
    );
    let mut residual_shaper =
        Shaper::new().expect("table width test requires a system font shaper");
    residual_shaper.set_font_family(Some("monospace"));
    residual_shaper.set_font_weight(Weight::BOLD);
    residual_shaper.set_font_style(shaping::Style::Italic);
    let residual_demand = measure_column_demand(
        table,
        columns,
        style.body_font_size,
        style.text_spacing_mode,
        style.body_font_family.first().map(String::as_str),
        Some(&mut residual_shaper),
        &source_view,
    );

    assert_eq!(
        residual_demand, clean_demand,
        "column demand must depend on the table style, not the shaper's prior state"
    );
}

#[test]
fn layout_table_ascii_art_not_broken_arbitrarily() {
    let md = "| File tree |\n| --- |\n| ├── mod.rs        # Widget trait |\n| └── state.rs      # State management |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let max_demand: f32 = laid_out
        .blocks
        .iter()
        .filter_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                column_widths.first().copied()
            } else {
                None
            }
        })
        .sum();
    assert!(
        max_demand > 300.0,
        "single wide column should get most of the 400px viewport, got {}",
        max_demand
    );
}

#[test]
fn layout_table_header_only_no_body() {
    let md = "| A | B |
| --- | --- |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let table = laid_out.blocks.iter().find_map(|b| {
        if let LaidOutBlockKind::Table { rows, header_height, .. } = &b.kind {
            Some((rows.len(), *header_height))
        } else {
            None
        }
    });
    let (body_count, hdr_h) = table.unwrap_or((99, 0.0));
    assert_eq!(body_count, 0, "header-only table should have 0 body rows");
    assert!(hdr_h > 0.0, "header height should be nonzero");
}

#[test]
fn layout_table_cjk_content() {
    let md = "| 名前 | 説明 |
| --- | --- |
| 太郎 | これは長い日本語の説明文です。テーブル内で正しく折り返されるべきです。 |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let table = laid_out.blocks.iter().find_map(|b| {
        if let LaidOutBlockKind::Table { rows, row_heights, column_widths, .. } = &b.kind {
            Some((rows.len(), row_heights.len(), column_widths.len()))
        } else {
            None
        }
    });
    let (body_rows, rh_count, col_count) = table.unwrap_or((0, 0, 0));
    assert!(body_rows > 0, "should have body rows");
    assert_eq!(body_rows, rh_count, "row_heights count must match body rows");
    assert!(col_count >= 2, "should have at least 2 columns");
}

#[test]
fn layout_table_many_columns() {
    let md = "| A | B | C | D | E |
| --- | --- | --- | --- | --- |
| 1 | 2 | 3 | 4 | 5 |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let widths = laid_out
        .blocks
        .iter()
        .find_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();
    assert_eq!(widths.len(), 5, "should have 5 columns");
    let total: f32 = widths.iter().sum();
    assert!(
        (total - 400.0).abs() < 10.0,
        "5-column widths ({}) should approximately fill available width",
        total
    );
    for (i, w) in widths.iter().enumerate() {
        assert!(*w > 5.0, "column {} width {} is too small", i, w);
    }
}

#[test]
fn layout_table_cell_inner_w_minimum() {
    let md = "| a | b |
| --- | --- |
| x | y |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 30.0, &core::document::StringDocView::new(src));
    let has_table =
        laid_out.blocks.iter().any(|b| matches!(b.kind, LaidOutBlockKind::Table { .. }));
    assert!(has_table, "should produce a table even with narrow viewport");
}

#[test]
fn layout_table_header_cell_demand_affects_width() {
    let md = "| Very Long Header Title |
| --- |
| hi |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let width = laid_out
        .blocks
        .iter()
        .find_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                column_widths.first().copied()
            } else {
                None
            }
        })
        .unwrap_or(0.0);
    assert!(
        width > 350.0,
        "single column with long header should get most of 400px, got {}",
        width
    );
}

#[test]
fn layout_table_two_columns_unequal_content() {
    let md = "| Name | Description \n| --- | --- \n| Alice | This is a fairly long description that should ideally get more horizontal space so it does not wrap excessively |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let widths = laid_out
        .blocks
        .iter()
        .find_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    assert!(widths.len() >= 2, "expected 2 columns, got {}", widths.len());
    let total: f32 = widths.iter().sum();
    let narrow_frac = widths[0] / total;
    assert!(
        narrow_frac < 0.30,
        "narrow column took {:.0}% of space ({}px / {}px); expected < 30%",
        narrow_frac * 100.0,
        widths[0],
        total
    );
    assert!(
        widths[1] > widths[0] * 2.5,
        "wide column ({}px) should be >2.5x narrow column ({}px)",
        widths[1],
        widths[0]
    );
}

#[test]
fn layout_table_two_columns_equal_content() {
    let md = "| Key | Value
| --- | ---
| foo | bar
| baz | qux |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let widths = laid_out
        .blocks
        .iter()
        .find_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    assert!(widths.len() >= 2, "expected 2 columns");
    let total: f32 = widths.iter().sum();
    let ratio = widths[0] / total;
    assert!(
        ratio > 0.35 && ratio < 0.65,
        "equal-content columns should be ~50/50, got {:.0}% / {:.0}%",
        ratio * 100.0,
        (1.0 - ratio) * 100.0
    );
}

#[test]
fn layout_table_two_columns_extreme_ratio() {
    let md = "| Tag | Notes
| --- | ---
| x | This is an extremely long piece of text that should dominate the available horizontal space in the table layout |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let widths = laid_out
        .blocks
        .iter()
        .find_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    assert!(widths.len() >= 2, "expected 2 columns");
    let total: f32 = widths.iter().sum();
    assert!(
        widths[0] / total < 0.20,
        "near-empty column took {:.0}%; expected < 20%",
        widths[0] / total * 100.0
    );
}

#[test]
fn layout_table_three_columns_gives_long_text_remaining_space() {
    let md = "| A | B | C
| --- | --- | ---
| short | short | This is a very long description that benefits from using the space left by short columns |";
    let (src, doc) = make_doc(md);
    let laid_out =
        layout_doc(&doc.blocks, &default_style(), 400.0, &core::document::StringDocView::new(src));
    let widths = laid_out
        .blocks
        .iter()
        .find_map(|b| {
            if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                Some(column_widths.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    assert_eq!(widths.len(), 3, "expected 3 columns");
    let total: f32 = widths.iter().sum();
    assert!(
        widths[2] / total > 0.65,
        "long text should use more than 65% when the other columns are short: {widths:?}"
    );
    assert!((total - 400.0).abs() < 0.01, "table should use the available width: {widths:?}");
}

#[test]
fn table_column_allocation_uses_full_width_after_clamping() {
    let widths = allocate_column_widths(&[1.0, 1.0, 1000.0], 400.0, 8.0, 48.0);
    assert!((widths.iter().sum::<f32>() - 400.0).abs() < 0.01, "{widths:?}");
    assert!(widths[2] > 260.0, "long column should receive unused space: {widths:?}");
}

#[test]
fn table_column_allocation_fits_narrow_viewport() {
    let widths = allocate_column_widths(&[100.0, 100.0, 100.0], 90.0, 8.0, 48.0);
    assert!((widths.iter().sum::<f32>() - 90.0).abs() < 0.01, "{widths:?}");
    assert!(widths.iter().all(|width| *width > 0.0));
}

#[test]
fn layout_inline_code_double_space_no_panic() {
    let md = "| 状图（├── `mod.rs  # comment`）整行 |\n|---|\n| x |";
    let style = default_style();
    let dl = crate::render_markdown(md, &style, 400.0, 600.0, 0.0);
    assert!(!dl.cmds.is_empty());
}

#[test]
fn layout_text_preserves_all_chars_after_viewport_resize() {
    let md = "| 状图（├── `mod.rs  # comment`）整行 |\n|---|\n| x |";
    let style = default_style();
    for &w in &[100.0, 188.0, 300.0, 500.0, 800.0] {
        let doc = crate::builder::MarkdownDoc::build(&crate::parser::parse_markdown(md), &style);
        let laid = layout_doc(&doc.blocks, &style, w, &core::document::StringDocView::new(md));
        let mut all_text = String::new();
        for block in &laid.blocks {
            collect_laid_out_text(block, &mut all_text);
        }
        assert!(all_text.contains("状图（├──"), "missing CJK at w={}: {}", w, all_text);
        assert!(all_text.contains("mod.rs"), "missing mod.rs at w={}: {}", w, all_text);
        assert!(all_text.contains("# comment"), "missing comment at w={}: {}", w, all_text);
        assert!(all_text.contains("）整行"), "missing closing at w={}: {}", w, all_text);
    }
}

#[test]
fn list_spacing_not_added_across_paragraph() {
    let md = "- first\n\nparagraph\n\n- second";
    let parsed = crate::parser::parse_markdown(md);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
    let second_item = laid_out.blocks.iter().find(|b| {
        if let LaidOutBlockKind::ListItem { lines, .. } = &b.kind {
            lines.iter().any(|l| l.text.contains("second"))
        } else {
            false
        }
    });
    assert!(second_item.is_some(), "should find 'second' list item");
    let para_block = laid_out.blocks.iter().find(|b| {
        if let LaidOutBlockKind::Text { lines } = &b.kind {
            lines.iter().any(|l| l.text.contains("paragraph"))
        } else {
            false
        }
    });
    assert!(para_block.is_some(), "should find paragraph block");
    let gap =
        second_item.unwrap().rect.y - (para_block.unwrap().rect.y + para_block.unwrap().rect.h);
    let expected_gap = style.paragraph_spacing;
    assert!(
        (gap - expected_gap).abs() < 1.0,
        "gap between paragraph and non-adjacent list should be ~paragraph_spacing={}, got {}",
        expected_gap,
        gap
    );
}

#[test]
fn heading_followed_by_code_block_has_spacing() {
    let (src, doc) = make_doc(
        "### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count = display_map.tree.total_rows()\n```",
    );
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 600.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2, "should have heading and code block");

    let heading = &laid_out.blocks[0];
    let code_block = &laid_out.blocks[1];

    let heading_bottom = heading.rect.y + heading.rect.h;
    let code_top = code_block.rect.y;

    let gap = code_top - heading_bottom;
    assert!(
        gap >= style.heading_spacing_bottom - 1.0,
        "gap {} between heading bottom and code block top should be >= heading_spacing_bottom {}",
        gap,
        style.heading_spacing_bottom
    );
}

#[test]
fn heading_followed_by_list_spacing_not_too_large() {
    let (src, doc) = make_doc("## 标题\n\n- 列表项1\n- 列表项2");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 600.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2, "should have heading + list items");

    let heading = &laid_out.blocks[0];
    let heading_bottom = heading.rect.y + heading.rect.h;

    let first_item =
        laid_out.blocks.iter().find(|b| matches!(&b.kind, LaidOutBlockKind::ListItem { .. }));
    let first_item = first_item.expect("should have a list item");
    let item_top = first_item.rect.y;

    let gap = item_top - heading_bottom;
    let max_expected = style.heading_spacing_bottom + 2.0;
    assert!(
        gap <= max_expected,
        "gap {} between heading and list should be ~heading_spacing_bottom ({}), not {}",
        gap,
        style.heading_spacing_bottom,
        gap
    );
    assert!(
        gap >= style.heading_spacing_bottom - 1.0,
        "gap {} should be at least heading_spacing_bottom {}",
        gap,
        style.heading_spacing_bottom
    );
}

#[test]
fn blockquote_height_excludes_trailing_paragraph_spacing() {
    let (src, doc) = make_doc("> quoted text");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(!laid_out.blocks.is_empty(), "should have blockquote block");
    let bq = &laid_out.blocks[0];
    if let LaidOutBlockKind::BlockQuote { blocks } = &bq.kind {
        assert!(!blocks.is_empty(), "blockquote should have child blocks");
        let child_h: f32 = blocks.iter().map(|b| b.rect.h).sum();
        let expected_h = style.blockquote_padding + child_h + style.blockquote_padding;
        assert!(
            (bq.rect.h - expected_h).abs() < 1.0,
            "blockquote height {} should be ~{} (2*padding + child_h), diff={}",
            bq.rect.h,
            expected_h,
            bq.rect.h - expected_h
        );
    } else {
        panic!("expected BlockQuote");
    }
}

#[test]
fn blockquote_has_spacing_to_next_block() {
    let (src, doc) = make_doc("> quoted text\n\nNext paragraph.");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2, "need blockquote + paragraph");
    let bq = &laid_out.blocks[0];
    let next = &laid_out.blocks[1];
    let bq_bottom = bq.rect.y + bq.rect.h;
    let gap = next.rect.y - bq_bottom;
    assert!(
        gap >= style.paragraph_spacing - 1.0,
        "gap between blockquote bottom and next block {} should be >= paragraph_spacing {}",
        gap,
        style.paragraph_spacing
    );
}

// ===== Margin collapsing tests =====

#[test]
fn para_heading_margin_collapsing() {
    let (src, doc) = make_doc("paragraph\n\n# Heading");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2);
    let para_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
    let heading_top = laid_out.blocks[1].rect.y;
    let gap = heading_top - para_bottom;
    let expected = style.heading_spacing_top;
    assert!(
        (gap - expected).abs() < 1.0,
        "para→H1 gap {} should be ~heading_spacing_top ({}), not para+top ({})",
        gap,
        expected,
        style.paragraph_spacing + style.heading_spacing_top
    );
}

#[test]
fn code_heading_margin_collapsing() {
    let (src, doc) = make_doc("```\ncode\n```\n\n## Heading");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2);
    let code_block = laid_out
        .blocks
        .iter()
        .find(|b| matches!(&b.kind, LaidOutBlockKind::CodeBlock { .. }))
        .unwrap();
    let code_bottom = code_block.rect.y + code_block.rect.h;
    let heading =
        laid_out.blocks.iter().find(|b| matches!(&b.kind, LaidOutBlockKind::Text { .. })).unwrap();
    let gap = heading.rect.y - code_bottom;
    let expected = style.heading_spacing_top * 0.8;
    assert!(
        (gap - expected).abs() < 1.0,
        "code→H2 gap {} should be ~{} (margin collapsing), not {}",
        gap,
        expected,
        style.paragraph_spacing + expected
    );
}

#[test]
fn code_to_list_not_double_spaced() {
    let (src, doc) = make_doc("```\ncode\n```\n\n- item");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    let code_block = laid_out
        .blocks
        .iter()
        .find(|b| matches!(&b.kind, LaidOutBlockKind::CodeBlock { .. }))
        .unwrap();
    let code_bottom = code_block.rect.y + code_block.rect.h;
    let list_item = laid_out
        .blocks
        .iter()
        .find(|b| matches!(&b.kind, LaidOutBlockKind::ListItem { .. }))
        .unwrap();
    let gap = list_item.rect.y - code_bottom;
    let expected = style.paragraph_spacing;
    assert!(
        (gap - expected).abs() < 1.0,
        "code→list gap {} should be ~paragraph_spacing ({}), not 2x ({})",
        gap,
        expected,
        expected * 2.0
    );
}

#[test]
fn bq_to_para_not_double_spaced() {
    let (src, doc) = make_doc("> quote\n\nnext paragraph");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    assert!(laid_out.blocks.len() >= 2);
    let bq_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
    let next_top = laid_out.blocks[1].rect.y;
    let gap = next_top - bq_bottom;
    let expected = style.paragraph_spacing;
    assert!(
        (gap - expected).abs() < 1.0,
        "bq→para gap {} should be ~paragraph_spacing ({}), not 2x ({})",
        gap,
        expected,
        expected * 2.0
    );
}

#[test]
fn hr_to_list_not_double_spaced() {
    let (src, doc) = make_doc("---\n\n- item");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    let hr = laid_out
        .blocks
        .iter()
        .find(|b| matches!(&b.kind, LaidOutBlockKind::HorizontalRule))
        .unwrap();
    let list_item = laid_out
        .blocks
        .iter()
        .find(|b| matches!(&b.kind, LaidOutBlockKind::ListItem { .. }))
        .unwrap();
    let block_gap = list_item.rect.y - (hr.rect.y + hr.rect.h);
    let expected = style.rule_spacing.max(style.list_group_spacing);
    assert!(
        (block_gap - expected).abs() < 1.0,
        "gap from HR content to list {} should resolve to max(rule, group) ({})",
        block_gap,
        expected
    );
}

#[test]
fn heading_hr_heading_no_stale_heading_flag() {
    let (src, doc) = make_doc("## First\n\n---\n\n## Second");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
    let hr = laid_out
        .blocks
        .iter()
        .find(|b| matches!(&b.kind, LaidOutBlockKind::HorizontalRule))
        .unwrap();
    let second_heading = laid_out
        .blocks
        .iter()
        .find(|b| {
            matches!(&b.kind, LaidOutBlockKind::Text { .. }) && b.rect.y > hr.rect.y + hr.rect.h
        })
        .unwrap();
    let visual_gap = second_heading.rect.y - (hr.rect.y + hr.rect.h);
    let expected = style.rule_spacing.max(style.heading_spacing_top * 0.8);
    assert!(
        (visual_gap - expected).abs() < 1.5,
        "visual gap from HR rule line to H2 {} should be ~heading_spacing_top ({})",
        visual_gap,
        expected
    );
}

#[test]
fn horizontal_rule_rect_contains_only_the_rule() {
    let (src, doc) = make_doc("---");
    let style = default_style();
    let laid_out = layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));

    assert_eq!(laid_out.blocks.len(), 1);
    assert_eq!(laid_out.blocks[0].rect.h, style.rule_thickness);
    assert_eq!(laid_out.blocks[0].rect.y, style.rule_spacing);
    assert_eq!(laid_out.total_height, style.rule_spacing * 2.0 + style.rule_thickness);
}

#[test]
fn paragraph_rule_paragraph_uses_symmetric_external_gaps() {
    let source = "before\n\n---\n\nafter";
    let (_, doc) = make_doc(source);
    let mut style = default_style();
    style.paragraph_spacing = 30.0;
    style.rule_spacing = 12.0;
    let laid_out =
        layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(source));
    let before = &laid_out.blocks[0];
    let rule = &laid_out.blocks[1];
    let after = &laid_out.blocks[2];

    let gap_before = rule.rect.y - (before.rect.y + before.rect.h);
    let gap_after = after.rect.y - (rule.rect.y + rule.rect.h);
    assert_eq!(gap_before, 30.0);
    assert_eq!(gap_after, 30.0);
}

#[test]
fn list_item_content_to_nested_rule_uses_the_larger_internal_gap() {
    let mut style = default_style();
    style.paragraph_spacing = 8.0;
    style.rule_spacing = 20.0;
    let (_, rule_doc) = make_doc("---");
    let rule = rule_doc.blocks.into_iter().next().expect("fixture must produce a rule");
    let item = BlockNode {
        kind: BlockKind::ListItem {
            bullet: crate::builder::ListBullet::Bullet,
            tight: false,
            blank_line_before: false,
        },
        children: vec![rule],
        text_lines: vec!["item".to_string()],
        projected_lines: vec![],
        text_styles: vec![vec![]],
        source_range: crate::builder::BlockSource::Continuous(0..4),
        block_range: 0..4,
        code_line_source_starts: None,
    };
    let document = core::document::StringDocView::new("item");
    let mut ctx = LayoutCtx::new(&document, &style, 400.0, None, None, None, None);

    layout_block(&item, &mut ctx);

    let LaidOutBlockKind::ListItem { lines, blocks, .. } = &ctx.output[0].kind else {
        panic!("fixture must produce a list item")
    };
    let item_line = lines.last().expect("list item must retain its text line");
    let item_text_bottom = item_line.rect.y + item_line.rect.h;
    let rule = blocks
        .iter()
        .find(|block| matches!(block.kind, LaidOutBlockKind::HorizontalRule))
        .expect("fixture must produce a nested horizontal rule");

    assert!((rule.rect.y - item_text_bottom - 20.0).abs() < 0.01);
}

#[test]
fn content_origin_container_places_first_actual_output_at_given_y() {
    let source = "# First\n\n## Second";
    let (_, doc) = make_doc(source);
    let mut container = doc.blocks[0].clone();
    container.kind = BlockKind::Container;
    container.children = doc.blocks;
    let style = default_style();
    let document = core::document::StringDocView::new(source);
    let mut ctx = LayoutCtx::new(&document, &style, 400.0, None, None, None, None);
    ctx.y = 100.0;

    layout_block_at_content_origin(&container, &mut ctx);

    assert_eq!(ctx.output.len(), 2);
    assert_eq!(ctx.output[0].rect.y, 100.0);
    let first_bottom = ctx.output[0].rect.y + ctx.output[0].rect.h;
    assert!((ctx.output[1].rect.y - first_bottom - style.heading_spacing_bottom).abs() < 0.01);
}
