use textora_markdown::builder::{BlockKind, MarkdownDoc};
use textora_markdown::layout::{
    LaidOutBlock, LaidOutBlockKind, LaidOutDoc, LazyLayout, MarkdownLayout,
    layout_doc_with_shaper_for_rendering,
};
use textora_markdown::parser::parse_markdown;
use textora_markdown::render::{render_doc_with_offset, render_layout};
use textora_markdown::style::MarkdownStyle;
use ui::core::DrawCmd;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;

const VIEWPORT_WIDTH: f32 = 800.0;
const VIEWPORT_HEIGHT: f32 = 2_000.0;
const GEOMETRY_TOLERANCE: f32 = 0.01;
const H2_TOP_SPACING_SCALE: f32 = 0.8;

fn test_style(font_size: f32, line_height: f32) -> MarkdownStyle {
    MarkdownStyle::from_theme(&ui::theme::test_theme(), font_size, line_height)
}

struct RenderedFixture {
    layout: MarkdownLayout,
    rules: Vec<Rect>,
}

fn render_fixture(source: &str, style: &MarkdownStyle) -> RenderedFixture {
    let parsed = parse_markdown(source);
    let document = MarkdownDoc::build_for_editing(&parsed, style, source);
    let source_view = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("spacing fixture requires a text shaper");
    let layout = layout_doc_with_shaper_for_rendering(
        &document.blocks,
        style,
        VIEWPORT_WIDTH,
        Some(&mut shaper),
        None,
        &source_view,
    );
    let mut draw_list = DrawList::new();
    render_layout(&layout, style, &mut draw_list, 0.0, VIEWPORT_HEIGHT, Some(&mut shaper));

    let mut rules: Vec<Rect> = draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::FillRect { rect, color, .. }
                if *color == style.rule_color
                    && (rect.h - style.rule_thickness).abs() < GEOMETRY_TOLERANCE
                    && rect.w > VIEWPORT_WIDTH * 0.5 =>
            {
                Some(*rect)
            }
            _ => None,
        })
        .collect();
    rules.sort_by(|left, right| left.y.total_cmp(&right.y));

    RenderedFixture { layout, rules }
}

fn find_line_rect(blocks: &[LaidOutBlock], needle: &str) -> Option<Rect> {
    blocks.iter().find_map(|block| match &block.kind {
        LaidOutBlockKind::Text { lines }
        | LaidOutBlockKind::CodeBlock { lines, .. }
        | LaidOutBlockKind::MetadataBlock { lines } => {
            lines.iter().find(|line| line.text.contains(needle)).map(|line| line.rect)
        }
        LaidOutBlockKind::BlockQuote { blocks } => find_line_rect(blocks, needle),
        LaidOutBlockKind::ListItem { blocks, lines, .. } => lines
            .iter()
            .find(|line| line.text.contains(needle))
            .map(|line| line.rect)
            .or_else(|| find_line_rect(blocks, needle)),
        LaidOutBlockKind::Table { header, rows, .. } => header
            .iter()
            .flatten()
            .chain(rows.iter().flatten().flatten())
            .find(|line| line.text.contains(needle))
            .map(|line| line.rect),
        LaidOutBlockKind::HorizontalRule => None,
    })
}

fn line_rect(fixture: &RenderedFixture, needle: &str) -> Rect {
    find_line_rect(&fixture.layout.document().blocks, needle)
        .unwrap_or_else(|| panic!("fixture must lay out a content line containing {needle:?}"))
}

fn only_rule(fixture: &RenderedFixture) -> Rect {
    assert_eq!(fixture.rules.len(), 1, "fixture must render exactly one horizontal rule");
    fixture.rules[0]
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() < GEOMETRY_TOLERANCE,
        "{context}: actual={actual}, expected={expected}"
    );
}

fn assert_rule_gaps(
    source: &str,
    style: &MarkdownStyle,
    upper_text: &str,
    lower_text: &str,
    expected_upper: f32,
    expected_lower: f32,
) {
    let fixture = render_fixture(source, style);
    let rule = only_rule(&fixture);
    let upper_line = line_rect(&fixture, upper_text);
    let lower_line = line_rect(&fixture, lower_text);
    let upper_gap = rule.y - (upper_line.y + upper_line.h);
    let lower_gap = lower_line.y - (rule.y + rule.h);

    assert_close(upper_gap, expected_upper, "content line above the rule");
    assert_close(lower_gap, expected_lower, "content line below the rule");
}

fn assert_rule_outer_gaps(source: &str, style: &MarkdownStyle, expected_gap: f32) {
    let fixture = render_fixture(source, style);
    let rule = only_rule(&fixture);
    let blocks = &fixture.layout.document().blocks;
    let rule_index = blocks
        .iter()
        .position(|block| matches!(block.kind, LaidOutBlockKind::HorizontalRule))
        .expect("fixture must lay out a horizontal rule block");
    let previous = rule_index
        .checked_sub(1)
        .and_then(|index| blocks.get(index))
        .expect("fixture must have a block before the rule");
    let next = blocks.get(rule_index + 1).expect("fixture must have a block after the rule");

    assert_close(
        rule.y - (previous.rect.y + previous.rect.h),
        expected_gap,
        "block outer edge above the rule",
    );
    assert_close(next.rect.y - (rule.y + rule.h), expected_gap, "block outer edge below the rule");
}

#[test]
fn paragraph_rule_boundaries_use_the_larger_request_in_both_directions() {
    for (font_size, line_height) in [(12.0, 24.0), (30.0, 24.0), (15.0, 48.0)] {
        let style = test_style(font_size, line_height);
        let expected_gap = style.paragraph_spacing.max(style.rule_spacing);
        assert_rule_gaps(
            "上方正文\n\n---\n\n下方正文",
            &style,
            "上方正文",
            "下方正文",
            expected_gap,
            expected_gap,
        );
    }
}

#[test]
fn consecutive_rules_share_one_rule_spacing_boundary() {
    let style = test_style(15.0, 24.0);
    let fixture = render_fixture("---\n\n---", &style);

    assert_eq!(fixture.rules.len(), 2, "fixture must render both horizontal rules");
    let gap = fixture.rules[1].y - (fixture.rules[0].y + fixture.rules[0].h);
    assert_close(gap, style.rule_spacing, "consecutive rule boundary");
}

#[test]
fn heading_rule_boundaries_use_heading_bottom_and_scaled_heading_top_requests() {
    let style = test_style(15.0, 24.0);
    let expected_upper = style.heading_spacing_bottom.max(style.rule_spacing);
    let expected_lower = (style.heading_spacing_top * H2_TOP_SPACING_SCALE).max(style.rule_spacing);

    assert_rule_gaps(
        "## 上方标题\n\n---\n\n## 下方标题",
        &style,
        "上方标题",
        "下方标题",
        expected_upper,
        expected_lower,
    );
}

#[test]
fn list_groups_on_both_sides_of_a_rule_use_group_spacing() {
    let style = test_style(15.0, 24.0);
    let expected_gap = style.list_group_spacing.max(style.rule_spacing);

    assert_rule_gaps(
        "- 上方列表\n\n---\n\n- 下方列表",
        &style,
        "上方列表",
        "下方列表",
        expected_gap,
        expected_gap,
    );
}

#[test]
fn code_quote_and_table_rule_boundaries_use_paragraph_spacing() {
    let fixtures = [
        "```\n上方代码\n```\n\n---\n\n```\n下方代码\n```",
        "> 上方引用\n\n---\n\n> 下方引用",
        "| 上方表格 |\n| --- |\n| 上方单元格 |\n\n---\n\n| 下方表格 |\n| --- |\n| 下方单元格 |",
    ];

    for line_height in [24.0, 48.0] {
        let style = test_style(15.0, line_height);
        let expected_gap = style.paragraph_spacing.max(style.rule_spacing);
        for source in fixtures {
            assert_rule_outer_gaps(source, &style, expected_gap);
        }
    }
}

#[test]
fn blockquote_internal_rule_boundaries_stay_symmetric_when_paragraph_spacing_is_larger() {
    let style = test_style(15.0, 48.0);
    let expected_gap = style.paragraph_spacing.max(style.rule_spacing);

    assert_rule_gaps(
        "> 上方正文\n>\n> ---\n>\n> 下方正文",
        &style,
        "上方正文",
        "下方正文",
        expected_gap,
        expected_gap,
    );
}

#[test]
fn document_edges_keep_the_existing_rule_edge_spacing() {
    let style = test_style(15.0, 24.0);
    let leading = render_fixture("---\n\n正文", &style);
    let leading_rule = only_rule(&leading);
    assert_close(leading_rule.y, style.rule_spacing, "leading document edge");
    assert_close(
        line_rect(&leading, "正文").y - (leading_rule.y + leading_rule.h),
        style.paragraph_spacing.max(style.rule_spacing),
        "leading rule to paragraph",
    );

    let trailing = render_fixture("正文\n\n---", &style);
    let trailing_rule = only_rule(&trailing);
    let trailing_line = line_rect(&trailing, "正文");
    assert_close(
        trailing_rule.y - (trailing_line.y + trailing_line.h),
        style.paragraph_spacing.max(style.rule_spacing),
        "paragraph to trailing rule",
    );
    assert_close(
        trailing.layout.document().total_height - (trailing_rule.y + trailing_rule.h),
        style.rule_spacing,
        "trailing document edge",
    );
}

#[test]
fn explicit_empty_paragraphs_add_their_own_space_for_lf_and_crlf() {
    let style = test_style(15.0, 24.0);
    let expected_added_height = style.line_height + style.paragraph_spacing;

    for newline in ["\n", "\r\n"] {
        let baseline_source = format!("上方正文{newline}{newline}---{newline}{newline}下方正文");
        let before_source =
            format!("上方正文{newline}{newline}{newline}---{newline}{newline}下方正文");
        let after_source =
            format!("上方正文{newline}{newline}---{newline}{newline}{newline}下方正文");
        let baseline = render_fixture(&baseline_source, &style);
        let extra_before = render_fixture(&before_source, &style);
        let extra_after = render_fixture(&after_source, &style);
        let baseline_rule = only_rule(&baseline);
        let before_rule = only_rule(&extra_before);
        let baseline_lower = line_rect(&baseline, "下方正文");
        let after_rule = only_rule(&extra_after);
        let after_lower = line_rect(&extra_after, "下方正文");

        assert_close(
            before_rule.y - baseline_rule.y,
            expected_added_height,
            "empty paragraph before a rule",
        );
        assert_close(
            (after_lower.y - (after_rule.y + after_rule.h))
                - (baseline_lower.y - (baseline_rule.y + baseline_rule.h)),
            expected_added_height,
            "empty paragraph after a rule",
        );
    }
}

#[test]
fn non_rule_spacing_baseline_preserves_paragraph_heading_and_list_semantics() {
    let style = test_style(15.0, 24.0);

    let paragraphs = render_fixture("第一段\n\n第二段", &style);
    let first = line_rect(&paragraphs, "第一段");
    let second = line_rect(&paragraphs, "第二段");
    assert_close(
        second.y - (first.y + first.h),
        style.paragraph_spacing,
        "paragraph to paragraph baseline",
    );

    let paragraph_heading = render_fixture("正文\n\n## 标题", &style);
    let paragraph = line_rect(&paragraph_heading, "正文");
    let heading = line_rect(&paragraph_heading, "标题");
    assert_close(
        heading.y - (paragraph.y + paragraph.h),
        style.paragraph_spacing.max(style.heading_spacing_top * H2_TOP_SPACING_SCALE),
        "paragraph to H2 baseline",
    );

    let heading_paragraph = render_fixture("## 标题\n\n正文", &style);
    let heading = line_rect(&heading_paragraph, "标题");
    let paragraph = line_rect(&heading_paragraph, "正文");
    assert_close(
        paragraph.y - (heading.y + heading.h),
        style.heading_spacing_bottom,
        "H2 to paragraph baseline",
    );

    let list = render_fixture("- 第一项\n- 第二项\n\n列表后正文", &style);
    let first_item = line_rect(&list, "第一项");
    let second_item = line_rect(&list, "第二项");
    let following_paragraph = line_rect(&list, "列表后正文");
    assert_close(
        second_item.y - (first_item.y + first_item.h),
        style.list_item_spacing,
        "tight list item boundary baseline",
    );
    assert_close(
        following_paragraph.y - (second_item.y + second_item.h),
        style.list_group_spacing,
        "list group to paragraph baseline",
    );
}

fn assert_lazy_block_matches_full(
    lazy: &LazyLayout<MarkdownDoc>,
    full: &LaidOutDoc,
    block_index: usize,
) {
    let actual = lazy.laid_out[block_index]
        .as_ref()
        .unwrap_or_else(|| panic!("lazy block {block_index} must be materialized"));
    let expected = &full.blocks[block_index];
    let y_correction = lazy.y_delta[block_index];

    assert_close(actual.rect.x, expected.rect.x, "lazy block x");
    assert_close(actual.rect.y + y_correction, expected.rect.y, "lazy block y");
    assert_close(actual.rect.w, expected.rect.w, "lazy block width");
    assert_close(actual.rect.h, expected.rect.h, "lazy block height");
}

fn lazy_fixture_source() -> String {
    let mut blocks: Vec<String> = (0..24).map(|index| format!("前段 {index:02}")).collect();
    blocks.push("---".to_owned());
    blocks.extend((0..24).map(|index| format!("后段 {index:02}")));
    blocks.join("\n\n")
}

#[test]
fn lazy_materialization_precision_eviction_and_full_layout_share_geometry() {
    const SHORT_VIEWPORT_HEIGHT: f32 = 72.0;

    let source = lazy_fixture_source();
    let style = test_style(15.0, 24.0);
    let parsed = parse_markdown(&source);
    let full_document = MarkdownDoc::build_for_editing(&parsed, &style, &source);
    let lazy_document = full_document.clone();
    let source_view = core::document::StringDocView::new(&source);
    let mut full_shaper = shaping::Shaper::new().expect("full layout requires a text shaper");
    let full_layout = layout_doc_with_shaper_for_rendering(
        &full_document.blocks,
        &style,
        VIEWPORT_WIDTH,
        Some(&mut full_shaper),
        None,
        &source_view,
    );
    let full = full_layout.document();
    let mut lazy = LazyLayout::new(lazy_document, &style, VIEWPORT_WIDTH, &source_view);
    let rule_index = (0..lazy.laid_out.len())
        .find(|&laid_index| {
            lazy.laid_to_doc(laid_index)
                .and_then(|document_index| lazy.source.blocks.get(document_index))
                .is_some_and(|block| matches!(block.kind, BlockKind::HorizontalRule))
        })
        .expect("fixture must contain a horizontal rule");

    let mut precise_shaper =
        shaping::Shaper::new().expect("lazy precision pass requires a text shaper");
    let rule_scroll_y = lazy.estimated_positions[rule_index];
    lazy.ensure_precise_range(rule_scroll_y, 1.0, &style, &mut precise_shaper, None, &source_view);
    assert!(lazy.precise[rule_index], "target rule must complete the precision pass");
    assert_lazy_block_matches_full(&lazy, full, rule_index);

    let mut viewport_shaper =
        shaping::Shaper::new().expect("lazy viewport pass requires a text shaper");
    lazy.ensure_visible(
        0.0,
        SHORT_VIEWPORT_HEIGHT,
        &style,
        VIEWPORT_WIDTH,
        &mut viewport_shaper,
        None,
        &source_view,
    );
    assert_lazy_block_matches_full(&lazy, full, 0);
    lazy.ensure_visible(
        lazy.total_height - SHORT_VIEWPORT_HEIGHT,
        SHORT_VIEWPORT_HEIGHT,
        &style,
        VIEWPORT_WIDTH,
        &mut viewport_shaper,
        None,
        &source_view,
    );
    assert!(lazy.laid_out[0].is_none(), "scrolling away must evict the first block");
    lazy.ensure_visible(
        0.0,
        SHORT_VIEWPORT_HEIGHT,
        &style,
        VIEWPORT_WIDTH,
        &mut viewport_shaper,
        None,
        &source_view,
    );
    assert_lazy_block_matches_full(&lazy, full, 0);

    lazy.ensure_all_blocks(&style, VIEWPORT_WIDTH, Some(&mut viewport_shaper), None, &source_view);
    for block_index in 0..full.blocks.len() {
        assert_lazy_block_matches_full(&lazy, full, block_index);
    }
    assert_close(lazy.total_height, full.total_height, "lazy total document height");

    let (materialized, y_delta) = lazy.materialized_blocks();
    let mut lazy_draw_list = DrawList::new();
    render_doc_with_offset(
        &materialized,
        &style,
        &mut lazy_draw_list,
        0.0,
        VIEWPORT_HEIGHT,
        0.0,
        0.0,
        Some(&mut viewport_shaper),
        &y_delta,
    );
    let lazy_rule = lazy_draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            DrawCmd::FillRect { rect, color, .. }
                if *color == style.rule_color
                    && (rect.h - style.rule_thickness).abs() < GEOMETRY_TOLERANCE
                    && rect.w > VIEWPORT_WIDTH * 0.5 =>
            {
                Some(*rect)
            }
            _ => None,
        })
        .expect("fully materialized lazy layout must render its horizontal rule");
    let full_fixture = render_fixture(&source, &style);
    let full_rule = only_rule(&full_fixture);
    assert_close(lazy_rule.y, full_rule.y, "lazy rendered rule y");
    assert_close(lazy_rule.h, full_rule.h, "lazy rendered rule thickness");
}
