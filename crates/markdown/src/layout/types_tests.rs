use super::super::block::layout_doc_with_shaper;
use super::*;
use crate::builder::MarkdownDoc;
use crate::layout::apply_deltas;
use crate::parser::parse_markdown;
use crate::projection::CursorAffinity;
use crate::projection::SourceByte;
use crate::test_utils::default_style;

fn make_doc(md: &str) -> (&str, MarkdownDoc) {
    let parsed = parse_markdown(md);
    (md, MarkdownDoc::build(&parsed, &default_style()))
}

fn build_editing_layout(
    source: &str,
    cursor_byte: usize,
    source_generation: u32,
) -> LazyLayout<MarkdownDoc> {
    let style = default_style();
    let document =
        MarkdownDoc::build_for_editing(&parse_markdown(source), &default_style(), source);
    let document_view = textora_core::document::StringDocView::new(source);
    let mut layout = LazyLayout::new(document, &style, 400.0, &document_view);
    layout.set_source_generation(source_generation);
    layout.set_edit_source(Some(source.to_owned()));
    layout.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
    layout.build_flat_lines(&document_view);
    layout
}

#[test]
fn horizontal_rule_flat_line_retains_atomic_source_range() {
    let source = "before\n\n---\n\nafter";
    let horizontal_rule_start = source.find("---").expect("fixture contains a rule");
    let layout = build_editing_layout(source, 0, 1);
    let horizontal_rule = layout
        .flat_lines
        .iter()
        .find(|line| line.text.is_empty() && line.source_projection.is_none())
        .expect("fixture lays out a horizontal rule");

    assert_eq!(
        horizontal_rule.atomic_source_range,
        Some(horizontal_rule_start..horizontal_rule_start + 3)
    );
}

#[test]
fn one_enter_after_rule_marker_collapses_source_marker() {
    for source in ["---\n", "before\n\n---\n", "---\r\n"] {
        let layout = build_editing_layout(source, source.len(), 1);
        assert!(
            layout
                .laid_out
                .iter()
                .flatten()
                .any(|block| { matches!(block.kind, LaidOutBlockKind::HorizontalRule) }),
            "one Enter should render the horizontal rule for {source:?}"
        );
    }
}

#[test]
fn active_horizontal_rule_keeps_both_external_paragraph_gaps() {
    let source = "before\n\n---\n\nafter";
    let cursor = source.find("---").expect("fixture contains a rule");
    let layout = build_editing_layout(source, cursor, 1);
    let style = default_style();
    let expected_gap = style.paragraph_spacing.max(style.rule_spacing);
    let blocks: Vec<_> = layout
        .laid_out
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let mut rect = block.as_ref().expect("editing materializes each block").rect;
            rect.y += layout.y_delta[index];
            rect
        })
        .collect();
    assert_eq!(blocks.len(), 3);
    let before_gap = blocks[1].y - (blocks[0].y + blocks[0].h);
    let after_gap = blocks[2].y - (blocks[1].y + blocks[1].h);
    assert!((before_gap - expected_gap).abs() < 0.01, "before gap: {before_gap}");
    assert!((after_gap - expected_gap).abs() < 0.01, "after gap: {after_gap}");
}

#[test]
fn rule_activation_and_neighbor_edits_reconcile_to_fresh_layout() {
    let sources = [
        "before\n\n---\n\nafter",
        "before\n\n---\n\nafter",
        "before\n\n---\n\n# after",
        "before\n\n---\n\n\nafter",
        "before\n\n---\n\nafter",
    ];
    let mut previous = build_editing_layout(sources[0], 0, 1);
    for (offset, source) in sources.iter().enumerate().skip(1) {
        let cursor =
            if offset == 1 { source.find("---").expect("fixture contains a rule") } else { 0 };
        let generation = offset as u32 + 1;
        let expected = build_editing_layout(source, cursor, generation);
        let (reconciled, _) = reconcile_editing_layout(previous, source, cursor, generation);
        assert_flat_layout_equivalent(&reconciled, &expected, source);
        previous = reconciled;
    }
}

fn assert_flat_layout_equivalent(
    actual: &LazyLayout<MarkdownDoc>,
    expected: &LazyLayout<MarkdownDoc>,
    source: &str,
) {
    assert_eq!(actual.total_height, expected.total_height);
    assert_eq!(actual.flat_lines.len(), expected.flat_lines.len());
    for (actual_line, expected_line) in actual.flat_lines.iter().zip(&expected.flat_lines) {
        assert_eq!(actual_line.flat_idx, expected_line.flat_idx);
        assert_eq!(actual_line.rect, expected_line.rect);
        assert_eq!(actual_line.text, expected_line.text);
        assert_eq!(actual_line.font_size, expected_line.font_size);
        assert_eq!(actual_line.source_projection, expected_line.source_projection);
    }

    let actual_projection_index = actual
        .source_projection_index
        .as_ref()
        .expect("the reconciled editing layout publishes a projection index");
    let expected_projection_index = expected
        .source_projection_index
        .as_ref()
        .expect("the full editing layout publishes a projection index");
    assert_eq!(actual_projection_index.visual_lines(), expected_projection_index.visual_lines());
    for source_byte in (0..=source.len()).filter(|byte| source.is_char_boundary(*byte)) {
        for affinity in [CursorAffinity::Upstream, CursorAffinity::Downstream] {
            let actual_position = actual_projection_index
                .visual_position_for_source(SourceByte::new(source_byte), affinity)
                .map(|position| (position.projection_line_idx, position.grapheme_pos));
            let expected_position = expected_projection_index
                .visual_position_for_source(SourceByte::new(source_byte), affinity)
                .map(|position| (position.projection_line_idx, position.grapheme_pos));
            assert_eq!(actual_position, expected_position, "source byte {source_byte}");
        }
    }
}

fn reconcile_editing_layout(
    previous: LazyLayout<MarkdownDoc>,
    source: &str,
    cursor_byte: usize,
    source_generation: u32,
) -> (LazyLayout<MarkdownDoc>, usize) {
    let style = default_style();
    let document =
        MarkdownDoc::build_for_editing(&parse_markdown(source), &default_style(), source);
    let document_view = textora_core::document::StringDocView::new(source);
    let mut layout = LazyLayout::new(document, &style, 400.0, &document_view);
    layout.set_source_generation(source_generation);
    layout.set_edit_source(Some(source.to_owned()));
    layout.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    let reused_block_count = layout.reuse_unchanged_blocks_from(previous);
    layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
    layout.build_flat_lines(&document_view);
    (layout, reused_block_count)
}

#[test]
fn reconciled_zero_width_paragraphs_match_fresh_layout_through_edits() {
    let sources = [
        "# title\n\nhead\n\n\n---\n\n# tail",
        "# title\n\nhead\n\n\n\n---\n\n# tail",
        "# title\n\nhead\n\n新\n\n\n---\n\n# tail",
        "# title\n\nhead\n\n\n---\n\n# tail",
        "# title\n\nhead\n\n---\n\n# tail",
    ];
    let cursor = "# title\n\nhead\n\n".len();
    let mut previous = build_editing_layout(sources[0], cursor, 1);
    for (offset, source) in sources.iter().enumerate().skip(1) {
        let generation = offset as u32 + 1;
        let expected = build_editing_layout(source, cursor, generation);
        let (reconciled, reused_count) =
            reconcile_editing_layout(previous, source, cursor, generation);
        assert!(reused_count > 0, "fixture must exercise block reuse");
        assert_flat_layout_equivalent(&reconciled, &expected, source);
        previous = reconciled;
    }
}

#[test]
fn reconciled_layout_reuses_prefix_and_shifted_suffix_equivalently() {
    let old_source = "# Title\n\nalpha\n\nomega 👩‍💻";
    let new_source = "# Title\n\nalpha changed\n\nomega 👩‍💻";
    let previous = build_editing_layout(
        old_source,
        old_source.find("alpha").expect("the old fixture contains the edited paragraph"),
        1,
    );
    let changed_cursor =
        new_source.find("changed").expect("the new fixture contains the inserted text");
    let expected = build_editing_layout(new_source, changed_cursor, 2);
    let style = default_style();
    let new_document =
        MarkdownDoc::build_for_editing(&parse_markdown(new_source), &default_style(), new_source);
    let document_view = textora_core::document::StringDocView::new(new_source);
    let mut reconciled = LazyLayout::new(new_document, &style, 400.0, &document_view);
    reconciled.set_source_generation(2);
    reconciled.set_edit_source(Some(new_source.to_owned()));
    reconciled.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: changed_cursor,
        preedit_text: None,
        preedit_cursor: None,
    }));

    let reused_block_count = reconciled.reuse_unchanged_blocks_from(previous);

    assert_eq!(reused_block_count, 2);
    assert!(reconciled.laid_out[0].is_some());
    assert!(reconciled.laid_out[1].is_none());
    assert!(reconciled.laid_out[2].is_some());
    assert_eq!(
        reconciled
            .reconciled_flat_lines
            .iter()
            .filter_map(Option::as_ref)
            .map(Vec::len)
            .sum::<usize>(),
        2
    );

    reconciled.ensure_all_blocks(&style, 400.0, None, None, &document_view);
    reconciled.build_flat_lines(&document_view);

    assert_flat_layout_equivalent(&reconciled, &expected, new_source);
}

#[test]
fn reconciled_edit_sequence_matches_full_layout_and_projection_index() {
    let initial_source = "# Title\n\nalpha 👩‍💻\n\n- item\n\n```rust\ncode\n```\n\n尾声";
    let initial_cursor =
        initial_source.find("alpha").expect("the initial fixture contains the edited paragraph");
    let mut reconciled = build_editing_layout(initial_source, initial_cursor, 1);
    let edit_sequence = [
        ("# Title\n\nalpha! 👩‍💻\n\n- item\n\n```rust\ncode\n```\n\n尾声", "alpha!"),
        ("# Title\n\nalpha! 👩‍💻\n\ninserted\n\n- item\n\n```rust\ncode\n```\n\n尾声", "inserted"),
        ("# Title\n\nalpha! 👩‍💻\n\ninserted\n\n```rust\ncode\n```\n\n尾声", "inserted"),
        (
            "# Title\n\nalpha! 👩‍💻\n\ninserted\n\n- first\n- second\n\n```rust\ncode\n```\n\n尾声",
            "second",
        ),
        (
            "# Title\n\nalpha! 👩‍💻\n\ninserted\n\n- first\n- second\n\n```rust\ncode edited\n```\n\n尾声",
            "code edited",
        ),
        (
            "# Title\n\nalpha! 👩‍💻\n\n- first\n- second\n\n```rust\ncode edited\n```\n\n尾声",
            "alpha!",
        ),
    ];

    for (edit_index, (source, cursor_needle)) in edit_sequence.iter().enumerate() {
        let cursor_byte =
            source.find(cursor_needle).expect("every edit fixture contains its cursor needle");
        let source_generation = u32::try_from(edit_index + 2)
            .expect("the bounded edit sequence generation fits in u32");
        let expected = build_editing_layout(source, cursor_byte, source_generation);
        let (next_reconciled, reused_block_count) =
            reconcile_editing_layout(reconciled, source, cursor_byte, source_generation);

        assert!(
            reused_block_count > 0,
            "edit {edit_index} should preserve at least one surrounding block"
        );
        assert_flat_layout_equivalent(&next_reconciled, &expected, source);
        reconciled = next_reconciled;
    }
}

#[test]
fn apply_deltas_empty_height_deltas_noop() {
    let mut yd = vec![0.0f32; 5];
    let orig = yd.clone();
    apply_deltas(&mut yd, &[]);
    assert_eq!(yd, orig);
}

#[test]
fn apply_deltas_single_block_shifts_subsequent() {
    let mut yd = vec![0.0f32; 5];
    // block 2 grew by 10px -> shifts blocks 3, 4
    apply_deltas(&mut yd, &[(2, 10.0)]);
    assert_eq!(yd[0], 0.0); // block 0 unaffected
    assert_eq!(yd[1], 0.0); // block 1 unaffected
    assert_eq!(yd[2], 0.0); // block 2 unaffected (own delta doesn't shift self)
    assert_eq!(yd[3], 10.0);
    assert_eq!(yd[4], 10.0);
}

#[test]
fn apply_deltas_multiple_blocks_accumulate() {
    let mut yd = vec![0.0f32; 5];
    apply_deltas(&mut yd, &[(1, 5.0), (3, 7.0)]);
    assert_eq!(yd[0], 0.0);
    assert_eq!(yd[1], 0.0);
    assert_eq!(yd[2], 5.0);
    assert_eq!(yd[3], 5.0); // block 3 unaffected by own delta
    assert_eq!(yd[4], 12.0); // 5.0 + 7.0
}

#[test]
fn apply_deltas_negative_delta() {
    let mut yd = vec![0.0f32; 5];
    apply_deltas(&mut yd, &[(2, -3.0)]);
    assert_eq!(yd[0], 0.0);
    assert_eq!(yd[1], 0.0);
    assert_eq!(yd[2], 0.0);
    assert_eq!(yd[3], -3.0);
    assert_eq!(yd[4], -3.0);
}

#[test]
fn apply_deltas_first_block_shifts_all_others() {
    let mut yd = vec![0.0f32; 3];
    apply_deltas(&mut yd, &[(0, 8.0)]);
    assert_eq!(yd[0], 0.0); // block 0 unaffected
    assert_eq!(yd[1], 8.0);
    assert_eq!(yd[2], 8.0);
}

#[test]
fn apply_deltas_single_element() {
    let mut yd = vec![0.0f32; 1];
    apply_deltas(&mut yd, &[(0, 5.0)]);
    // Only block 0, no subsequent blocks to shift
    assert_eq!(yd[0], 0.0);
}

#[test]
fn apply_deltas_very_small_delta() {
    let mut yd = vec![0.0f32; 3];
    apply_deltas(&mut yd, &[(0, 0.001)]);
    assert_eq!(yd[0], 0.0);
    assert!((yd[1] - 0.001).abs() < 1e-10);
    assert!((yd[2] - 0.001).abs() < 1e-10);
}

#[test]
fn lazy_layout_estimation_has_all_blocks() {
    let (src, doc) = make_doc("# Title\n\nparagraph\n\n## Section\n\n- item\n\n```\ncode\n```");
    let style = default_style();
    let lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    assert!(!lazy.laid_out.is_empty());
    assert_eq!(lazy.precise.len(), lazy.laid_out.len());
    assert_eq!(lazy.y_delta.len(), lazy.laid_out.len());
    assert!(lazy.precise.iter().all(|p| !*p), "all blocks start as estimated");
    assert!(lazy.y_delta.iter().all(|&d| d == 0.0), "y_delta starts at zero");
    assert!(lazy.total_height > 0.0);
}

#[test]
fn trailing_empty_source_lines_are_included_in_total_height() {
    let source = "paragraph\n\n\n";
    let style = default_style();
    let document = MarkdownDoc::build_for_editing(&parse_markdown(source), &style, source);
    let document_view = textora_core::document::StringDocView::new(source);
    let mut layout = LazyLayout::new(document, &style, 400.0, &document_view);
    layout.set_edit_source(Some(source.to_string()));
    layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
    layout.build_flat_lines(&document_view);

    let trailing_empty_bottom = layout
        .projected_empty_lines
        .iter()
        .map(|line| line.y_top + line.height)
        .max_by(f32::total_cmp)
        .expect("fixture must project trailing empty source lines");

    assert!(
        layout.total_height >= trailing_empty_bottom,
        "total height {} must include trailing empty-line bottom {}",
        layout.total_height,
        trailing_empty_bottom,
    );
}

#[test]
fn heading_blank_run_projects_editable_line_inside_real_gap() {
    // `# T` 后跟 2 个空行：可编辑空段落必须落在标题与后续段落之间，
    // 不得按 paragraph_spacing 假设而与段落文字重叠（heading_spacing_bottom 更小）。
    let source = "# T\n\n\npara";
    let style = default_style();
    let layout = build_editing_layout(source, source.len(), 1);

    let heading_line = layout
        .flat_lines
        .iter()
        .find(|line| line.text.trim_end() == "T")
        .expect("fixture must render the heading line");
    let para_line = layout
        .flat_lines
        .iter()
        .find(|line| line.text == "para")
        .expect("fixture must render the paragraph line");
    let heading_bottom = heading_line.rect.y + heading_line.rect.h;

    assert_eq!(
        layout.projected_empty_lines.len(),
        1,
        "run of two blank lines must project exactly one editable empty line"
    );
    let empty = layout.projected_empty_lines[0];
    assert!(
        empty.y_top >= heading_bottom - 0.01,
        "editable empty line y {} must not overlap the heading (bottom {heading_bottom})",
        empty.y_top
    );
    assert!(
        empty.y_top + empty.height <= para_line.rect.y + 0.01,
        "editable empty line bottom {} must not overlap the paragraph (top {})",
        empty.y_top + empty.height,
        para_line.rect.y
    );
    // 空段落拥有一行高和出向段落间距；入向间距仍沿用标题下间距。
    let gap = para_line.rect.y - heading_bottom;
    let expected_gap = style.heading_spacing_bottom + style.line_height + style.paragraph_spacing;
    assert!(
        (gap - expected_gap).abs() < 0.01,
        "real gap {gap} must contain heading spacing, empty line height and trailing spacing \
         ({expected_gap})"
    );
    assert!(
        (para_line.rect.y - (empty.y_top + empty.height) - style.paragraph_spacing).abs() < 0.01
    );
}

#[test]
fn blank_run_between_paragraphs_reserves_empty_paragraph_height_and_spacing() {
    // para→空段落→para：空段落两侧各有一份 paragraph_spacing。
    let source = "a\n\n\nb";
    let style = default_style();
    let layout = build_editing_layout(source, source.len(), 1);

    let a_line = layout.flat_lines.iter().find(|line| line.text == "a").expect("fixture has a");
    let b_line = layout.flat_lines.iter().find(|line| line.text == "b").expect("fixture has b");
    let gap = b_line.rect.y - (a_line.rect.y + a_line.rect.h);
    let expected = style.paragraph_spacing * 2.0 + style.line_height;
    assert!(
        (gap - expected).abs() < 0.01,
        "inter-block gap {gap} must contain spacing on both sides of the empty paragraph \
         ({expected})"
    );

    let empty =
        layout.projected_empty_lines.first().expect("fixture must project one editable empty line");
    assert!(
        (empty.y_top - (a_line.rect.y + a_line.rect.h + style.paragraph_spacing)).abs() < 0.01,
        "editable empty line must sit right below the paragraph spacing, got y {}",
        empty.y_top
    );
    assert!((b_line.rect.y - (empty.y_top + empty.height) - style.paragraph_spacing).abs() < 0.01);
}

#[test]
fn loose_list_item_reserves_empty_paragraph_height_and_spacing() {
    // `- a` 与项内子段落 `b` 之间有一个可编辑空段落：前后各有一份
    // paragraph_spacing，中间保留完整 line_height。
    let source = "- a\n\n\n  b";
    let style = default_style();
    let layout = build_editing_layout(source, source.len(), 1);

    let item_blocks = layout
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::ListItem { blocks, .. } if blocks.len() == 3 => Some(blocks),
            _ => None,
        })
        .expect("fixture must produce two text paragraphs and an explicit empty paragraph");
    let first_bottom = item_blocks[0].rect.y + item_blocks[0].rect.h;
    let second_top = item_blocks[2].rect.y;
    let expected = style.paragraph_spacing * 2.0 + style.line_height;
    assert!(
        (second_top - first_bottom - expected).abs() < 0.01,
        "child paragraph gap {} must contain spacing on both sides of the empty paragraph \
         ({expected})",
        second_top - first_bottom
    );
}

#[test]
fn editable_empty_paragraph_inside_loose_list_keeps_both_boundaries() {
    // 项内可编辑空段落不得覆盖文字，且上下各保留一份段落间距。
    let source = "- a\n\n\n  b";
    let style = default_style();
    let layout = build_editing_layout(source, source.len(), 1);

    let a_line = layout.flat_lines.iter().find(|line| line.text == "a").expect("fixture has a");
    let b_line = layout.flat_lines.iter().find(|line| line.text == "b").expect("fixture has b");
    let a_bottom = a_line.rect.y + a_line.rect.h;

    let empty = layout
        .projected_empty_lines
        .first()
        .expect("fixture must project one editable empty line inside the list item");
    assert!(
        (empty.y_top - a_bottom - style.paragraph_spacing).abs() < 0.01,
        "editable empty paragraph must keep spacing above: y={}, item bottom={a_bottom}",
        empty.y_top,
    );
    assert!(
        (b_line.rect.y - (empty.y_top + empty.height) - style.paragraph_spacing).abs() < 0.01,
        "editable empty paragraph must keep spacing below: bottom={}, child top={}",
        empty.y_top + empty.height,
        b_line.rect.y
    );
}

#[test]
fn viewport_eviction_discards_ascii_diagram_sidecars_without_mismatching_same_first_row() {
    let filler = "filler line\n".repeat(100);
    let source =
        format!("```\n┌───┐\n│甲  │\n└───┘\n```\n\n{filler}\n```\n┌───┐\n│乙  │\n└───┘\n```");
    let style = default_style();
    let parsed = parse_markdown(&source);
    let document = MarkdownDoc::build(&parsed, &style);
    let source_view = textora_core::document::StringDocView::new(&source);
    let mut layout = LazyLayout::new(document, &style, 400.0, &source_view);
    let mut shaper = shaping::Shaper::new().expect("sidecar eviction test needs a shaper");

    layout.ensure_visible(0.0, 80.0, &style, 400.0, &mut shaper, None, &source_view);
    layout.ensure_visible(
        (layout.total_height - 80.0).max(0.0),
        80.0,
        &style,
        400.0,
        &mut shaper,
        None,
        &source_view,
    );

    let code_lines = layout
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| match &block.kind {
            LaidOutBlockKind::CodeBlock { lines, .. }
                if lines.iter().any(|line| line.text.contains('乙')) =>
            {
                Some(lines)
            }
            _ => None,
        })
        .expect("second diagram must be materialized after scrolling into its viewport");
    let diagram = layout
        .ascii_diagrams()
        .diagram_for(code_lines)
        .expect("second diagram must retain its own sidecar");
    let second_row_text: String =
        diagram.rows[1].cells.iter().map(|cell| cell.text.as_str()).collect();

    assert_eq!(second_row_text, "│乙  │");
    assert_eq!(
        layout.ascii_diagrams().entry_count(),
        1,
        "evicted diagrams must not leave stale registry entries behind"
    );
}

#[test]
fn invalidation_discards_ascii_diagram_sidecars_before_relayout() {
    let source = "```\n┌───┐\n│甲  │\n└───┘\n```\n\nparagraph";
    let style = default_style();
    let parsed = parse_markdown(source);
    let document = MarkdownDoc::build(&parsed, &style);
    let source_view = textora_core::document::StringDocView::new(source);
    let mut layout = LazyLayout::new(document, &style, 400.0, &source_view);
    let mut shaper = shaping::Shaper::new().expect("sidecar invalidation test needs a shaper");
    layout.ensure_visible(0.0, 600.0, &style, 400.0, &mut shaper, None, &source_view);

    assert_eq!(layout.ascii_diagrams().entry_count(), 1);

    layout.invalidate_lines_for_source_bytes([source.find('甲').expect("fixture has label")]);

    assert_eq!(
        layout.ascii_diagrams().entry_count(),
        0,
        "invalidated diagrams must release their sidecar before relayout"
    );
}

#[test]
fn precision_pass_marks_block_precise() {
    let (src, doc) = make_doc("# Title\n\nparagraph text here\n\n## Another heading");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));

    let mut shaper = shaping::Shaper::new().unwrap();
    let _deltas = lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );

    // First block (and possibly more) should be precise now
    assert!(lazy.precise[0], "first block should be precise");
    let any_precise = lazy.precise.iter().any(|p| *p);
    assert!(any_precise, "at least one block should be precise");
}

#[test]
fn precision_pass_respects_scroll_offset() {
    let (src, doc) = make_doc("# Block 1\n\n## Block 2\n\n### Block 3\n\nparagraph");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    assert!(lazy.laid_out.len() >= 3);

    let second_block_y = lazy.laid_out[1].as_ref().unwrap().rect.y;
    let mut shaper = shaping::Shaper::new().unwrap();
    // Scroll so only block 1+ (index 1 and beyond) are in range
    let _deltas = lazy.ensure_precise_range(
        second_block_y + 10.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );

    // Block 0 is above range_start, should NOT be precise
    // (unless viewport is so large that buffer_above reaches it)
    let any_precise = lazy.precise.iter().any(|p| *p);
    assert!(any_precise, "at least one block should be precise");
}

#[test]
fn lazy_layout_e2e_renders_all_text() {
    use crate::render::render_doc_with_offset;
    let md = "# Title\n\nparagraph with **bold**\n\n- list item\n\n```\ncode\n```";
    let style = default_style();
    let (src, doc) = make_doc(md);
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    let mut shaper = shaping::Shaper::new().unwrap();

    // Precision-pass visible area
    lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );

    // Render
    let mut dl = ui::core::paint::DrawList::new();
    let (visible, visible_yd) = lazy.materialized_blocks();
    render_doc_with_offset(
        &visible,
        &style,
        &mut dl,
        0.0,
        600.0,
        0.0,
        0.0,
        Some(&mut shaper),
        &visible_yd,
    );

    let texts: Vec<String> = dl
        .cmds
        .iter()
        .filter_map(|c| {
            if let ui::core::paint::DrawCmd::TextLayout { layout, .. } = c {
                Some(layout.text.clone())
            } else {
                None
            }
        })
        .collect();
    let all_text = texts.concat();
    assert!(all_text.contains("Title"));
    assert!(all_text.contains("paragraph"));
    assert!(all_text.contains("bold"));
    assert!(all_text.contains("list item"));
    assert!(all_text.contains("code"));
}

#[test]
fn lazy_layout_scroll_culling_still_works() {
    use crate::render::render_doc_with_offset;
    let md = "# Top\n\n## Middle\n\n### Bottom";
    let style = default_style();
    let (src, doc) = make_doc(md);
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    let mut shaper = shaping::Shaper::new().unwrap();

    // Only make "Middle" and "Bottom" precise
    let mid_y = lazy.laid_out[1].as_ref().unwrap().rect.y;
    lazy.ensure_precise_range(
        mid_y,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );

    // Scroll past Top
    let scroll_y = lazy.laid_out[0].as_ref().unwrap().rect.y
        + lazy.laid_out[0].as_ref().unwrap().rect.h
        + 10.0;
    let mut dl = ui::core::paint::DrawList::new();
    let (visible, visible_yd) = lazy.materialized_blocks();
    render_doc_with_offset(
        &visible,
        &style,
        &mut dl,
        scroll_y,
        600.0,
        0.0,
        0.0,
        Some(&mut shaper),
        &visible_yd,
    );

    let texts: Vec<String> = dl
        .cmds
        .iter()
        .filter_map(|c| {
            if let ui::core::paint::DrawCmd::TextLayout { layout, .. } = c {
                Some(layout.text.clone())
            } else {
                None
            }
        })
        .collect();
    let all_text = texts.concat();
    assert!(!all_text.contains("Top"), "Top should be culled by scroll");
    assert!(
        all_text.contains("Middle") || all_text.contains("Bottom"),
        "visible blocks should render"
    );
}

#[test]
fn lazy_layout_y_delta_propagates_correctly() {
    let md = "# A\n\n## B\n\n### C";
    let style = default_style();
    let (src, doc) = make_doc(md);
    let lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));

    // Before precision: all y_delta = 0
    assert_eq!(lazy.y_delta[0], 0.0);
    assert_eq!(lazy.y_delta[1], 0.0);
    assert_eq!(lazy.y_delta[2], 0.0);

    // Real Y equals rect.y
    let b0_y = lazy.laid_out[0].as_ref().unwrap().rect.y;
    assert!((lazy.laid_out[0].as_ref().unwrap().rect.y + lazy.y_delta[0] - b0_y).abs() < 0.01);
}

/// 合成的多输出文档块源：一个顶层块展开为多个 LaidOutBlock
/// （根 Container，或意外到达顶层的 TableRow_）。
struct MultiOutputDocSource {
    blocks: Vec<crate::builder::BlockNode>,
}

impl BlockSource for MultiOutputDocSource {
    fn blocks(&self) -> &[crate::builder::BlockNode] {
        &self.blocks
    }

    fn headings(&self) -> &[crate::layout::HeadingEntry] {
        &[]
    }
}

fn multi_output_source(kind: crate::builder::BlockKind, md: &str) -> MultiOutputDocSource {
    let parsed = parse_markdown(md);
    let doc = MarkdownDoc::build(&parsed, &default_style());
    let wrapper = crate::builder::BlockNode {
        kind,
        children: doc.blocks,
        text_lines: vec![],
        projected_lines: vec![],
        text_styles: vec![],
        source_range: crate::builder::BlockSourceSpan::Continuous(0..md.len()),
        block_range: 0..md.len(),
        code_line_source_starts: None,
    };
    MultiOutputDocSource { blocks: vec![wrapper] }
}

fn laid_text(layout: &LazyLayout<MultiOutputDocSource>, slot: usize) -> String {
    let block = layout.laid_out[slot].as_ref().expect("slot must be materialized");
    match &block.kind {
        LaidOutBlockKind::Text { lines } => {
            lines.iter().map(|line| line.text.as_str()).collect::<String>()
        }
        other => {
            panic!("slot {slot} must be a text block, got {:?}", std::mem::discriminant(other))
        }
    }
}

#[test]
fn ensure_all_blocks_distributes_multi_output_doc_blocks() {
    // 根 Container 的两个子段落必须分别落入两个槽位；
    // 只取首个输出会把 slot 1 也填成 "alpha"（静默复制/丢弃）。
    let style = default_style();
    let md = "alpha\n\nomega";
    let source = multi_output_source(crate::builder::BlockKind::Container, md);
    let doc_view = textora_core::document::StringDocView::new(md);
    let mut layout = LazyLayout::new(source, &style, 400.0, &doc_view);
    layout.ensure_all_blocks(&style, 400.0, None, None, &doc_view);

    assert_eq!(layout.laid_out.len(), 2);
    assert_eq!(laid_text(&layout, 0), "alpha");
    assert_eq!(laid_text(&layout, 1), "omega");
}

#[test]
fn precise_block_at_distributes_multi_output_doc_blocks() {
    let style = default_style();
    let md = "alpha\n\nomega";
    let source = multi_output_source(crate::builder::BlockKind::Container, md);
    let doc_view = textora_core::document::StringDocView::new(md);
    let mut layout = LazyLayout::new(source, &style, 400.0, &doc_view);
    let mut shaper = shaping::Shaper::new().expect("multi-output test needs a shaper");

    layout.precise_block_at(0, &style, &mut shaper, None, &doc_view);

    assert_eq!(laid_text(&layout, 0), "alpha");
    assert_eq!(laid_text(&layout, 1), "omega");
    assert!(layout.precise.iter().all(|&p| p), "both slots of the group become precise");
}

#[test]
fn precise_multi_output_group_propagates_each_height_delta_once() {
    const VIEWPORT_WIDTH: f32 = 120.0;

    let style = default_style();
    let md = format!("{}\n\nomega", "wide ".repeat(40));
    let source = multi_output_source(crate::builder::BlockKind::Container, &md);
    let document_view = textora_core::document::StringDocView::new(&md);
    let mut layout = LazyLayout::new(source, &style, VIEWPORT_WIDTH, &document_view);
    let estimated_first_height = layout.estimated_heights[0];
    let mut shaper = shaping::Shaper::new().expect("multi-output test needs a shaper");

    layout.precise_block_at(0, &style, &mut shaper, None, &document_view);

    assert!(
        (layout.estimated_heights[0] - estimated_first_height).abs() > 0.5,
        "fixture must trigger a material first-slot height delta"
    );

    let reference_source = multi_output_source(crate::builder::BlockKind::Container, &md);
    let mut reference_shaper =
        shaping::Shaper::new().expect("multi-output reference layout needs a shaper");
    let reference = layout_doc_with_shaper(
        reference_source.blocks(),
        &style,
        VIEWPORT_WIDTH,
        Some(&mut reference_shaper),
        None,
        &document_view,
    );
    let displayed_second_y =
        layout.laid_out[1].as_ref().expect("second output must be materialized").rect.y
            + layout.y_delta[1];

    assert!(
        (displayed_second_y - reference.blocks[1].rect.y).abs() < 0.01,
        "second output y must match one-shot layout: lazy={displayed_second_y}, reference={}",
        reference.blocks[1].rect.y
    );
    assert!(
        (layout.total_height - reference.total_height).abs() < 0.01,
        "total height must match one-shot layout: lazy={}, reference={}",
        layout.total_height,
        reference.total_height
    );

    let stable_second_y = displayed_second_y;
    let stable_total_height = layout.total_height;
    layout.precise_block_at(0, &style, &mut shaper, None, &document_view);
    let repeated_second_y =
        layout.laid_out[1].as_ref().expect("second output must remain materialized").rect.y
            + layout.y_delta[1];
    assert!((repeated_second_y - stable_second_y).abs() < 0.01);
    assert!((layout.total_height - stable_total_height).abs() < 0.01);
}

#[test]
fn top_level_table_row_expands_without_layout_mismatch() {
    // flatten_blocks 必须镜像 layout_block 对顶层 TableRow_ 的子块展开，
    // 否则 LazyLayout::new 的块数不变量被打破。
    let style = default_style();
    let md = "alpha\n\nomega";
    let source = multi_output_source(crate::builder::BlockKind::TableRow_, md);
    let doc_view = textora_core::document::StringDocView::new(md);
    let mut layout = LazyLayout::new(source, &style, 400.0, &doc_view);
    layout.ensure_all_blocks(&style, 400.0, None, None, &doc_view);

    assert_eq!(layout.laid_out.len(), 2);
    assert_eq!(laid_text(&layout, 0), "alpha");
    assert_eq!(laid_text(&layout, 1), "omega");
}

#[test]
fn incremental_relayout_matches_one_shot_block_positions() {
    // 逐块内容重排必须保留估计阶段解析的边界，与全文布局落点一致。
    // 覆盖紧凑列表、列表组边界、连续标题和分割线邻接。
    let fixtures = [
        "para\n- a\n- b",
        "- a\n- b\n\npara",
        "para\n\n- a\n\n# H",
        "# H\n\npara\n- x",
        "- a\n\n# H\n\npara",
        "para\n\n---\n\n# H\n\ntail",
        "# A\n# B\n\n> q\n\n```\ncode\n```\n\nend",
    ];
    for md in fixtures {
        let style = default_style();
        let (src, doc) = make_doc(md);
        let doc_view = textora_core::document::StringDocView::new(src);
        let oneshot = crate::layout::block::layout_doc(doc.blocks(), &style, 400.0, &doc_view);

        let (_, lazy_doc) = make_doc(md);
        let mut lazy = LazyLayout::new(lazy_doc, &style, 400.0, &doc_view);
        lazy.ensure_all_blocks(&style, 400.0, None, None, &doc_view);

        assert_eq!(lazy.laid_out.len(), oneshot.blocks.len(), "block count mismatch for {md:?}");
        for (idx, oneshot_block) in oneshot.blocks.iter().enumerate() {
            let laid = lazy.laid_out[idx].as_ref().expect("block must be materialized");
            let final_y = laid.rect.y + lazy.y_delta.get(idx).copied().unwrap_or(0.0);
            assert!(
                (final_y - oneshot_block.rect.y).abs() < 0.01,
                "block {idx} y mismatch for {md:?}: incremental {final_y} vs one-shot {}",
                oneshot_block.rect.y
            );
        }
        assert!(
            (lazy.total_height - oneshot.total_height).abs() < 0.01,
            "total height mismatch for {md:?}: incremental {} vs one-shot {}",
            lazy.total_height,
            oneshot.total_height
        );
    }
}

#[test]
fn precise_block_at_returns_height_delta() {
    let (src, doc) = make_doc("# Title\n\nparagraph text");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    let mut shaper = shaping::Shaper::new().unwrap();

    assert!(!lazy.precise[0]);
    let _delta = lazy.precise_block_at(
        0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    assert!(lazy.precise[0], "block should be marked precise");
    // delta may be 0.0 if estimation was exact, but precise_block_at should not panic
    // total_height should have been updated
    assert!(lazy.total_height > 0.0);
    // calling again should still work (idempotent)
    let _delta2 = lazy.precise_block_at(
        0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    assert!(lazy.precise[0]);
}

#[test]
fn precise_block_at_out_of_bounds_returns_zero() {
    let (src, doc) = make_doc("hello");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    let mut shaper = shaping::Shaper::new().unwrap();

    let delta = lazy.precise_block_at(
        999,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    assert_eq!(delta, 0.0, "out-of-bounds idx should return 0.0");
}

#[test]
fn lazy_layout_empty_document() {
    let (src, doc) = make_doc("");
    let style = default_style();
    let lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    assert!(lazy.laid_out.is_empty());
    assert!(lazy.precise.is_empty());
    assert!(lazy.y_delta.is_empty());
    assert_eq!(lazy.total_height, 0.0);
}

#[test]
fn ensure_precise_range_twice_no_double_count() {
    let (src, doc) = make_doc("# A\n\n## B\n\n### C");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    let mut shaper = shaping::Shaper::new().unwrap();

    let _deltas1 = lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    let height_after_first = lazy.total_height;
    let y_delta_after_first = lazy.y_delta.clone();

    // Second call should be a no-op (all blocks already precise)
    let deltas2 = lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    assert!(deltas2.is_empty(), "second call should produce no deltas");
    assert_eq!(lazy.total_height, height_after_first, "total_height should not change");
    assert_eq!(lazy.y_delta, y_delta_after_first, "y_delta should not change");
}

#[test]
fn first_visible_block_idx_with_y_delta() {
    use crate::render::first_visible_block_idx;
    let md = "# A\n\n## B\n\n### C";
    let style = default_style();
    let (src, doc) = make_doc(md);
    let laid_out = crate::layout::layout_doc(
        &doc.blocks,
        &style,
        400.0,
        &textora_core::document::StringDocView::new(src),
    );
    assert!(laid_out.blocks.len() >= 3);

    // Without y_delta, should find block 0 at scroll_y=0
    assert_eq!(first_visible_block_idx(&laid_out.blocks, &[], 0.0), 0);

    // y_delta[1] = 50 means block 1's real position is shifted down by 50px.
    // This means block 1 extends further down, so scrolling past block 0's
    // original bottom should still find block 1 via backward walk.
    let mut y_delta = vec![0.0f32; laid_out.blocks.len()];
    y_delta[1] = 50.0; // block 1 and later shifted down by 50px

    // scroll_y just past block 0's bottom (without y_delta, block 1 starts here)
    let scroll_y = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 1.0;
    let idx_no_delta = first_visible_block_idx(&laid_out.blocks, &[], scroll_y);
    let idx_with_delta = first_visible_block_idx(&laid_out.blocks, &y_delta, scroll_y);

    // Both should find a valid block
    assert!(idx_no_delta < laid_out.blocks.len());
    assert!(idx_with_delta < laid_out.blocks.len());

    // With y_delta, block 1's real_bottom = rect.y + rect.h + 50,
    // which is higher than without y_delta, so the backward walk
    // may extend visibility further
    assert!(
        idx_with_delta <= idx_no_delta + 1,
        "y_delta should not cause block index to jump too far"
    );
}

#[test]
fn heading_followed_by_code_block_spacing_after_precision() {
    let mut shaper = shaping::Shaper::new().unwrap();
    let style = default_style();
    let md = "### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count\n                          = display_map.tree.total_rows()\n```";
    let parsed = parse_markdown(md);
    let doc = MarkdownDoc::build(&parsed, &style);
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 600.0, &textora_core::document::StringDocView::new(md));

    let deltas = lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(md),
    );

    assert!(lazy.laid_out.len() >= 2, "expected at least 2 blocks, got {}", lazy.laid_out.len());

    let heading = lazy.laid_out[0].as_ref().unwrap();
    let code_block = lazy.laid_out[1].as_ref().unwrap();

    let heading_real_bottom =
        heading.rect.y + lazy.y_delta.first().copied().unwrap_or(0.0) + heading.rect.h;
    let code_real_top = code_block.rect.y + lazy.y_delta.get(1).copied().unwrap_or(0.0);

    let gap = code_real_top - heading_real_bottom;
    assert!(
        gap >= style.heading_spacing_bottom - 1.0,
        "gap {} between heading bottom and code block top after precision should be >= heading_spacing_bottom {}. heading.rect=({},{}), code.rect=({},{}), y_delta={:?}, deltas={:?}",
        gap,
        style.heading_spacing_bottom,
        heading.rect.y,
        heading.rect.h,
        code_block.rect.y,
        code_block.rect.h,
        &lazy.y_delta,
        deltas
    );
}

#[test]
fn large_code_block_then_heading_then_code_block_spacing() {
    // Matches the real document structure: big diagram -> heading -> code block
    let mut shaper = shaping::Shaper::new().unwrap();
    let style = default_style();
    // Simulate a large code block (like the 84-line diagram), then heading, then code block
    let large_block = "```\n".to_string() + &"line\n".repeat(84) + "```";
    let md =
        format!("{}\n\n### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count\n```", large_block);
    let parsed = parse_markdown(&md);
    let doc = MarkdownDoc::build(&parsed, &style);
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 600.0, &textora_core::document::StringDocView::new(&md));

    let _deltas = lazy.ensure_precise_range(
        0.0,
        10000.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(&md),
    );

    assert!(
        lazy.laid_out.len() >= 3,
        "expected at least 3 blocks (large code + heading + code), got {}",
        lazy.laid_out.len()
    );

    let heading = lazy.laid_out[1].as_ref().unwrap();
    let code_block = lazy.laid_out[2].as_ref().unwrap();

    let heading_real_bottom =
        heading.rect.y + lazy.y_delta.get(1).copied().unwrap_or(0.0) + heading.rect.h;
    let code_real_top = code_block.rect.y + lazy.y_delta.get(2).copied().unwrap_or(0.0);

    let gap = code_real_top - heading_real_bottom;
    assert!(
        gap >= style.heading_spacing_bottom - 1.0,
        "gap {} between heading bottom and code block top should be >= heading_spacing_bottom {}. heading.rect=({},{}), code.rect.y={}, y_delta[1]={}, y_delta[2]={}",
        gap,
        style.heading_spacing_bottom,
        heading.rect.y,
        heading.rect.h,
        code_block.rect.y,
        lazy.y_delta.get(1).copied().unwrap_or(0.0),
        lazy.y_delta.get(2).copied().unwrap_or(0.0)
    );
}

#[test]
fn precision_block_rect_y_equals_estimation_rect_y() {
    // After precision pass, block.rect.y must stay as the ESTIMATION position,
    // not include y_delta. Otherwise rendering's rect.y + y_delta[i] formula
    // double-counts and creates visual overlaps.
    let mut shaper = shaping::Shaper::new().unwrap();
    let style = default_style();
    // Two paragraphs: first paragraph's height may differ between estimation
    // and precision, which produces a non-zero delta propagated via y_delta.
    let md = "A long paragraph that might wrap differently with HarfBuzz shaping than with the simple byte-based estimation used during the initial layout pass.\n\nSecond paragraph here.";
    let parsed = parse_markdown(md);
    let doc = MarkdownDoc::build(&parsed, &style);
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(md));

    // Snapshot estimation rect.y values
    let est_y0 = lazy.laid_out[0].as_ref().unwrap().rect.y;
    let est_y1 = lazy.laid_out[1].as_ref().unwrap().rect.y;

    // Precision-lay both blocks
    let _deltas = lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(md),
    );

    // rect.y must still equal the estimation position (not include y_delta).
    // If rect.y changed to include y_delta, the rendering formula
    // screen_y = rect.y + y_delta[i] would double-count.
    assert!(
        (lazy.laid_out[0].as_ref().unwrap().rect.y - est_y0).abs() < 0.5,
        "block[0].rect.y changed from {} to {} (delta={}), should stay at estimation position",
        est_y0,
        lazy.laid_out[0].as_ref().unwrap().rect.y,
        lazy.laid_out[0].as_ref().unwrap().rect.y - est_y0,
    );
    assert!(
        (lazy.laid_out[1].as_ref().unwrap().rect.y - est_y1).abs() < 0.5,
        "block[1].rect.y changed from {} to {} (delta={}), should stay at estimation position. y_delta[1]={}",
        est_y1,
        lazy.laid_out[1].as_ref().unwrap().rect.y,
        lazy.laid_out[1].as_ref().unwrap().rect.y - est_y1,
        lazy.y_delta.get(1).copied().unwrap_or(0.0),
    );
}

#[test]
fn no_overlap_heading_code_block_lines() {
    // Check that heading lines don't overlap with code block lines
    let mut shaper = shaping::Shaper::new().unwrap();
    let style = default_style();
    let md = "### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count\n                          = display_map.tree.total_rows()\n```";
    let parsed = parse_markdown(md);
    let doc = MarkdownDoc::build(&parsed, &style);
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 600.0, &textora_core::document::StringDocView::new(md));

    let _deltas = lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(md),
    );

    assert!(lazy.laid_out.len() >= 2, "expected at least 2 blocks, got {}", lazy.laid_out.len());

    let heading = lazy.laid_out[0].as_ref().unwrap();
    let code_block = lazy.laid_out[1].as_ref().unwrap();

    // Heading is Text block; get last line's bottom
    if let LaidOutBlockKind::Text { lines } = &heading.kind
        && let Some(last_heading_line) = lines.last()
    {
        let heading_text_bottom = last_heading_line.rect.y + last_heading_line.rect.h;
        let heading_real_bottom =
            heading_text_bottom + lazy.y_delta.first().copied().unwrap_or(0.0);

        // Code block first line
        if let LaidOutBlockKind::CodeBlock { lines: code_lines, .. } = &code_block.kind
            && let Some(first_code_line) = code_lines.first()
        {
            let code_text_top =
                first_code_line.rect.y + lazy.y_delta.get(1).copied().unwrap_or(0.0);

            let gap = code_text_top - heading_real_bottom;
            // Gap should be heading_spacing_bottom + code_block_padding
            let expected_min = style.heading_spacing_bottom + style.code_block_padding - 2.0;
            assert!(
                gap >= expected_min,
                "heading last line bottom to code first line top gap {} should be >= {} (heading_spacing_bottom {} + code_padding {}). heading_line.rect=({},{}), code_line.rect.y={}, y_delta={:?}",
                gap,
                expected_min,
                style.heading_spacing_bottom,
                style.code_block_padding,
                last_heading_line.rect.y,
                last_heading_line.rect.h,
                first_code_line.rect.y,
                &lazy.y_delta
            );
        }
    }
}

#[test]
fn flat_lines_update_after_y_delta() {
    let (src, doc) = make_doc("Line 1\nLine 2");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    let initial_y = lazy.flat_lines[0].rect.y;

    // Simulate y_delta change
    if !lazy.y_delta.is_empty() {
        lazy.y_delta[0] = 10.0;
        lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

        // Y position should have changed
        assert_ne!(lazy.flat_lines[0].rect.y, initial_y);
    }
}

// ===== build_flat_lines tests =====

#[test]
fn build_flat_lines_text_block() {
    let (src, doc) = make_doc("Hello world\nSecond line");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    assert!(!lazy.flat_lines.is_empty());
    // Text may be wrapped, so just check content contains expected text
    let all_text: String =
        lazy.flat_lines.iter().map(|fl| fl.text.as_str()).collect::<Vec<_>>().join(" ");
    assert!(all_text.contains("Hello world"));
    assert!(all_text.contains("Second line"));
    // Check flat_idx is sequential
    for (i, fl) in lazy.flat_lines.iter().enumerate() {
        assert_eq!(fl.flat_idx, i);
    }
}

#[test]
fn build_flat_lines_blockquote() {
    let (src, doc) = make_doc("> quoted text\n> second line");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    // BlockQuote should be flattened into the lines
    assert!(!lazy.flat_lines.is_empty());
    assert!(lazy.flat_lines.iter().any(|fl| fl.text.contains("quoted text")));
}

#[test]
fn build_flat_lines_table() {
    let (src, doc) = make_doc("| A | B |\n|---|---|\n| 1 | 2 |");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    // Table cells should be flattened
    assert!(!lazy.flat_lines.is_empty());
    // Check that table content is present
    let all_text: String =
        lazy.flat_lines.iter().map(|fl| fl.text.as_str()).collect::<Vec<_>>().join(" ");
    assert!(all_text.contains("A") || all_text.contains("B"));
}

#[test]
fn build_flat_lines_horizontal_rule() {
    let (src, doc) = make_doc("---");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    assert_eq!(lazy.flat_lines.len(), 1);
    assert_eq!(lazy.flat_lines[0].text, "");
    assert_eq!(lazy.flat_lines[0].font_size, 14.0); // HR has non-zero font_size so cursor is visible
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: 0,
        preedit_text: None,
        preedit_cursor: None,
    }));
    lazy.validate_editable_projections()
        .expect("a non-editable horizontal rule must not require a source projection");
}

#[test]
fn build_flat_lines_sorted_by_rect_y() {
    let (src, doc) = make_doc("First\n\nSecond\n\nThird");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    // Verify invariant: flat_lines sorted by rect.y ascending
    for i in 1..lazy.flat_lines.len() {
        assert!(
            lazy.flat_lines[i].rect.y >= lazy.flat_lines[i - 1].rect.y,
            "flat_lines not sorted: line {} y={} < line {} y={}",
            i,
            lazy.flat_lines[i].rect.y,
            i - 1,
            lazy.flat_lines[i - 1].rect.y
        );
    }
}

#[test]
fn build_flat_lines_empty_doc() {
    let (src, doc) = make_doc("");
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    // Empty doc might still have one empty line
    // Just verify it doesn't panic
}

/// Regression: flat_line y must match rendering y (within content_height).
/// Previously, push_flat_line computed y = abs_block_y + line.rect.y,
/// but line.rect.y was already absolute — causing double-counting.
#[test]
fn flat_lines_y_within_content_height() {
    let (src, doc) = make_doc(
        "# Title\n\nFirst paragraph.\n\nSecond paragraph.\n\n> Blockquote text\n\n- List item",
    );
    let style = default_style();
    let lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    let total_h = lazy.total_height;

    for (i, fl) in lazy.flat_lines.iter().enumerate() {
        assert!(
            fl.rect.y < total_h + style.line_height * 2.0, // small tolerance for spacing
            "flat_line {} '{}' y={} exceeds content_height={} — likely double-counted",
            i,
            fl.text,
            fl.rect.y,
            total_h,
        );
    }
}

/// Regression: flat_line y for the first line of a text block must
/// approximately equal block.rect.y (not 2× block.rect.y).
#[test]
fn flat_line_y_matches_block_y() {
    let (src, doc) = make_doc("Hello world");
    let style = default_style();
    let lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));

    assert!(!lazy.flat_lines.is_empty());
    assert!(!lazy.laid_out.is_empty());
    assert!(lazy.laid_out[0].is_some());

    let block = lazy.laid_out[0].as_ref().unwrap();
    let first_flat = &lazy.flat_lines[0];
    // first flat_line y should be close to block.rect.y, not 2× block.rect.y
    let expected_y = block.rect.y;
    assert!(
        (first_flat.rect.y - expected_y).abs() < 1.0,
        "flat_line y={} but block y={} — expected approximately equal",
        first_flat.rect.y,
        expected_y,
    );
}

/// Novel mode: consecutive paragraphs must have gap = paragraph_spacing (≈0.5× line_height),
/// NOT ~1.5–2× line_height (regression: each paragraph carried a trailing empty line from
/// the byte-range newline).
#[test]
fn novel_paragraph_spacing_not_doubled() {
    let src = "第一段内容文字。\n\n第二段内容文字。\n";
    let doc_view = textora_core::document::StringDocView::new(src);
    let novel = crate::builder::NovelStructure::scan(&doc_view);
    let style = default_style();
    let expected_gap = style.paragraph_spacing;
    let line_h = style.line_height;

    // Verify we have two paragraph blocks.
    assert_eq!(novel.blocks().len(), 2, "should have 2 paragraph blocks");
    for (i, b) in novel.blocks().iter().enumerate() {
        assert!(
            matches!(b.kind, crate::builder::BlockKind::Paragraph),
            "block {i} should be Paragraph"
        );
        // Each paragraph should produce exactly 1 line (no trailing empty).
        let lines = b.lines(&doc_view);
        assert_eq!(
            lines.len(),
            1,
            "paragraph {i} should have 1 line, got {}: {:?}",
            lines.len(),
            lines
        );
    }

    // Full layout as baseline.
    let full = LazyLayout::from_doc(novel.clone(), &style, 400.0, &doc_view);
    assert!(full.laid_out.len() >= 2);
    assert!(full.laid_out[0].is_some());
    assert!(full.laid_out[1].is_some());
    let b0_full = full.laid_out[0].as_ref().unwrap();
    let b1_full = full.laid_out[1].as_ref().unwrap();
    let gap_full = b1_full.rect.y - (b0_full.rect.y + b0_full.rect.h);
    assert!(
        (gap_full - expected_gap).abs() < 1.0,
        "full layout gap {:.1} should be ~paragraph_spacing {:.1}",
        gap_full,
        expected_gap
    );

    // Viewport-driven path: ensure_visible + build_flat_lines.
    let mut vp = LazyLayout::new(novel.clone(), &style, 400.0, &doc_view);
    let total_h = vp.total_height;
    vp.ensure_visible(
        0.0,
        total_h + 10.0,
        &style,
        400.0,
        &mut shaping::Shaper::new().expect("need shaper"),
        None,
        &doc_view,
    );
    vp.build_flat_lines(&doc_view);

    assert!(vp.laid_out.len() >= 2);
    assert!(vp.laid_out[0].is_some(), "block 0 should be materialized");
    assert!(vp.laid_out[1].is_some(), "block 1 should be materialized");
    let b0_vp = vp.laid_out[0].as_ref().unwrap();
    let b1_vp = vp.laid_out[1].as_ref().unwrap();
    let real_y0 = b0_vp.rect.y + vp.y_delta.first().copied().unwrap_or(0.0);
    let real_y1 = b1_vp.rect.y + vp.y_delta.get(1).copied().unwrap_or(0.0);
    let gap_vp = real_y1 - (real_y0 + b0_vp.rect.h);
    assert!(
        (gap_vp - expected_gap).abs() < 1.0,
        "viewport-driven gap {:.1} should be ~paragraph_spacing {:.1} (line_height={:.1})",
        gap_vp,
        expected_gap,
        line_h
    );

    // Sanity: gap must not be ~2× line_height (the old bug).
    assert!(
        gap_vp < line_h * 1.2,
        "gap {:.1} should NOT be ~2 line heights ({:.1}) — paragraph_spacing={:.1}",
        gap_vp,
        line_h * 2.0,
        expected_gap
    );
}

// ===== Active block marker tests =====

/// Build a LazyLayout with edit context (cursor at given byte).
fn layout_with_cursor(md: &str, cursor_byte: usize) -> LazyLayout<crate::builder::MarkdownDoc> {
    let src = md;
    let parsed = crate::parser::parse_markdown(src);
    let doc = crate::builder::MarkdownDoc::build(&parsed, &default_style());
    let style = default_style();
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.set_edit_source(Some(src.to_string()));
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    // Rebuild with edit context for span expansion + active marker
    let mut shaper = shaping::Shaper::new().expect("need shaper");
    lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));
    lazy
}

fn layout_with_cursor_and_width(
    source: &str,
    cursor_byte: usize,
    width: f32,
) -> LazyLayout<crate::builder::MarkdownDoc> {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = textora_core::document::StringDocView::new(source);
    let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
    lazy.set_edit_source(Some(source.to_string()));
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    let mut shaper = shaping::Shaper::new().expect("projection test needs a shaper");
    lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
    lazy.build_flat_lines(&doc_view);
    lazy
}

#[test]
fn every_editable_flat_line_has_projection_after_full_layout() {
    let corpus = [
        "plain paragraph",
        "# wrapped heading wrapped heading wrapped heading",
        "> first\n> second",
        "- first\n  continuation",
        "| a | b |\n| --- | --- |\n| c | d |",
        "```rust\nlet value = 1;\n```",
    ];

    for source in corpus {
        let lazy = layout_with_cursor_and_width(source, 0, 160.0);
        lazy.validate_editable_projections()
            .unwrap_or_else(|error| panic!("missing projection for {source:?}: {error:?}"));
    }
}

#[test]
fn estimated_soft_wrap_preserves_editable_projections() {
    let source = "a paragraph that is deliberately long enough to require an estimated soft wrap";
    let style = default_style();
    let parsed = crate::parser::parse_markdown(source);
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = textora_core::document::StringDocView::new(source);
    let estimated = layout_doc_with_shaper(doc.blocks(), &style, 40.0, None, None, &doc_view);
    let mut lazy = LazyLayout::new(doc, &style, 40.0, &doc_view);
    lazy.laid_out = estimated.blocks.into_iter().map(Some).collect();
    lazy.viewport_range = 0..lazy.laid_out.len();
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: 0,
        preedit_text: None,
        preedit_cursor: None,
    }));
    lazy.build_flat_lines(&doc_view);

    assert!(
        lazy.flat_lines.iter().any(|line| line.text.is_empty() && line.source_projection.is_some()),
        "estimated soft wrapping must preserve an editable projection for each continuation"
    );
    lazy.validate_editable_projections()
        .expect("editable estimated continuations must have source projections");
}

#[test]
fn local_viewport_rebuild_retains_stable_projection_identity_and_geometry() {
    let source = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
    let mut lazy = layout_with_cursor_and_width(source, 0, 240.0);
    let before = lazy
        .source_projection_index
        .as_ref()
        .expect("full fixture must have an index")
        .visual_lines()
        .iter()
        .map(|line| (line.owner, line.source_extent.clone()))
        .collect::<Vec<_>>();
    lazy.evict_outside(&(1..2));
    lazy.viewport_range = 1..2;
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(source));
    assert_eq!(
        lazy.source_projection_index
            .as_ref()
            .expect("index must survive local viewport rebuild")
            .visual_lines()
            .iter()
            .map(|line| (line.owner, line.source_extent.clone()))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn offscreen_projection_remains_addressable_without_a_rendered_line() {
    use crate::projection::ProjectionLineIdx;

    let source = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
    let mut lazy = layout_with_cursor_and_width(source, 0, 240.0);
    lazy.evict_outside(&(1..2));
    lazy.viewport_range = 1..2;
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(source));

    let index = lazy.source_projection_index.as_ref().expect("viewport layout has an index");
    let offscreen = index
        .visual_position_for_source(SourceByte::new(0), CursorAffinity::Downstream)
        .expect("the first paragraph remains in the projection index");
    assert_eq!(offscreen.projection_line_idx, ProjectionLineIdx::new(0));
    assert_eq!(lazy.flat_line_for_projection(offscreen.projection_line_idx), None);
    assert_eq!(
        index.source_anchor_at(lazy.source_generation, offscreen),
        Ok(SourceAnchor::downstream(SourceByte::new(0))),
    );

    let rendered = lazy.flat_lines.first().expect("the second paragraph is rendered");
    let rendered_idx = crate::selection::FlatLineIdx::new(rendered.flat_idx);
    let visible = lazy
        .projection_line_for_flat_line(rendered_idx)
        .expect("the visible paragraph has an editable projection");
    assert_ne!(visible, offscreen.projection_line_idx);
    assert_eq!(lazy.flat_line_for_projection(visible), Some(rendered_idx));
}

#[test]
fn atomic_flat_line_has_no_editable_projection() {
    let lazy = layout_with_cursor_and_width("first paragraph\n\n---\n\nthird paragraph", 0, 240.0);
    let rule = lazy
        .flat_lines
        .iter()
        .find(|line| line.atomic_source_range.is_some())
        .expect("the horizontal rule is rendered as an atomic line");
    assert_eq!(
        lazy.projection_line_for_flat_line(crate::selection::FlatLineIdx::new(rule.flat_idx)),
        None,
    );
}

#[test]
fn wrapped_layout_publishes_upstream_and_downstream_positions_at_shared_boundary() {
    let source = "a plain paragraph that is deliberately long enough to wrap several times";
    let lazy = layout_with_cursor_and_width(source, 0, 80.0);
    let index = lazy
        .source_projection_index
        .as_ref()
        .expect("a real wrapped layout must publish its projection index");
    let visual_lines = index.visual_lines();
    let (line_idx, previous, next, shared_byte) = visual_lines
        .windows(2)
        .enumerate()
        .find_map(|(line_idx, lines)| {
            let previous = &lines[0];
            let next = &lines[1];
            let shared_byte = previous.boundaries.last()?.byte;
            (shared_byte == next.boundaries.first()?.byte).then_some((
                line_idx,
                previous,
                next,
                shared_byte,
            ))
        })
        .expect("fixture must contain an automatic wrap boundary");

    assert_eq!(
        previous.boundaries.last(),
        Some(&SourceAnchor::downstream(shared_byte)),
        "layout must preserve the original source anchor on the preceding visual line",
    );
    assert_eq!(
        next.boundaries.first(),
        Some(&SourceAnchor::downstream(shared_byte)),
        "layout must preserve the original source anchor on the following visual line",
    );
    let upstream = index
        .visual_position_for_source(shared_byte, CursorAffinity::Upstream)
        .expect("the preceding visual line must be addressable upstream");
    let downstream = index
        .visual_position_for_source(shared_byte, CursorAffinity::Downstream)
        .expect("the following visual line must be addressable downstream");
    assert_eq!(upstream.projection_line_idx.get(), line_idx);
    assert_eq!(upstream.grapheme_pos.get(), previous.boundaries.len() - 1);
    assert_eq!(downstream.projection_line_idx.get(), line_idx + 1);
    assert_eq!(downstream.grapheme_pos.get(), 0);
}

#[test]
#[should_panic(expected = "source projection layout revision must not overflow")]
fn source_projection_revision_does_not_wrap_on_overflow() {
    let mut lazy = layout_with_cursor_and_width("plain paragraph", 0, 240.0);
    lazy.layout_revision = u64::MAX;

    let _ = lazy.rebuild_source_projection_index();
}

#[test]
fn active_wrapped_heading_gives_every_segment_explicit_projection() {
    let source = "# a heading long enough to wrap across three visual rows";
    let lazy = layout_with_cursor_and_width(source, 4, 120.0);
    let lines = &lazy.flat_lines;
    assert!(lines.len() >= 3);
    assert!(lines.iter().all(|line| line.source_projection.is_some()));
}

#[test]
fn consecutive_blockquote_projection_jumps_over_second_marker() {
    let source = "> first physical line\n> second physical line";
    let second = source.find("second").expect("fixture must contain second");
    let lazy = layout_with_cursor_and_width(source, second, 180.0);
    let second_line = lazy
        .flat_lines
        .iter()
        .find(|line| line.text.contains("second"))
        .expect("second text must be visible");
    let projection = second_line.source_projection.as_ref().expect("projection required");
    assert!(projection.boundaries.iter().any(|anchor| anchor.byte.get() == second));
    assert!(!projection.boundaries.iter().any(|anchor| anchor.byte.get() == 0));
}

#[test]
fn nested_blockquote_syntax_gap_has_canonical_projection() {
    let source = "> outer\n> > **inner** — continuation";
    let lazy = layout_with_cursor_and_width(source, 8, 140.0);
    let index = lazy
        .source_projection_index
        .as_ref()
        .expect("nested blockquote layout must publish an index");

    for source_byte in 7..14 {
        assert!(
            index
                .visual_position_for_source(
                    SourceByte::new(source_byte),
                    CursorAffinity::Downstream
                )
                .is_some(),
            "nested blockquote syntax byte {source_byte} must resolve canonically"
        );
    }
}

#[test]
fn heading_active_marker_appears_in_flat_line_text() {
    let lazy = layout_with_cursor("# Title", 1); // cursor in the prefix
    assert!(!lazy.flat_lines.is_empty(), "should have flat lines");
    let first_text = &lazy.flat_lines[0].text;
    assert!(
        first_text.starts_with("# "),
        "heading flat line should start with '# ', got: {:?}",
        first_text
    );
    assert!(
        first_text.contains("Title"),
        "heading flat line should contain 'Title', got: {:?}",
        first_text
    );
}

#[test]
fn heading_no_marker_when_cursor_outside_block() {
    // Cursor outside heading block range should NOT show marker
    let parsed = crate::parser::parse_markdown("# Title\n\nparagraph");
    let doc = crate::builder::MarkdownDoc::build(&parsed, &default_style());
    let style = default_style();
    let src = "# Title\n\nparagraph";
    let mut lazy =
        LazyLayout::from_doc(doc, &style, 400.0, &textora_core::document::StringDocView::new(src));
    lazy.set_edit_source(Some(src.to_string()));
    // Cursor in paragraph (byte 10), NOT in heading
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: 10,
        preedit_text: None,
        preedit_cursor: None,
    }));
    let mut shaper = shaping::Shaper::new().expect("need shaper");
    lazy.ensure_precise_range(
        0.0,
        600.0,
        &style,
        &mut shaper,
        None,
        &textora_core::document::StringDocView::new(src),
    );
    lazy.build_flat_lines(&textora_core::document::StringDocView::new(src));

    let first_text = &lazy.flat_lines[0].text;
    assert!(
        !first_text.starts_with("# "),
        "heading flat line should NOT start with '# ' when cursor outside, got: {:?}",
        first_text
    );
    assert!(first_text.contains("Title"), "heading flat line should contain 'Title'");
}

#[test]
fn heading_h2_active_marker_text() {
    let lazy = layout_with_cursor("## Section", 1); // cursor in the prefix
    let first_text = &lazy.flat_lines[0].text;
    assert!(
        first_text.starts_with("## "),
        "H2 flat line should start with '## ', got: {:?}",
        first_text
    );
}

#[test]
fn list_item_active_marker_appears_in_flat_line_text() {
    let lazy = layout_with_cursor("- item", 3); // cursor in "item"
    assert!(!lazy.flat_lines.is_empty());
    let first_text = &lazy.flat_lines[0].text;
    assert!(
        first_text.starts_with("- "),
        "list flat line should start with '- ', got: {:?}",
        first_text
    );
    assert!(
        first_text.contains("item"),
        "list flat line should contain 'item', got: {:?}",
        first_text
    );
}

#[test]
fn blockquote_active_marker_appears_in_flat_line_text() {
    let lazy = layout_with_cursor("> quoted", 5); // cursor in "quoted"
    assert!(!lazy.flat_lines.is_empty());
    let first_text = &lazy.flat_lines[0].text;
    assert!(
        first_text.starts_with("> "),
        "blockquote flat line should start with '> ', got: {:?}",
        first_text
    );
    assert!(
        first_text.contains("quoted"),
        "blockquote flat line should contain 'quoted', got: {:?}",
        first_text
    );
}

#[test]
fn task_list_active_marker_appears_in_flat_line_text() {
    let lazy = layout_with_cursor("- [ ] todo", 7); // cursor in "todo"
    assert!(!lazy.flat_lines.is_empty());
    let first_text = &lazy.flat_lines[0].text;
    assert!(
        first_text.starts_with("- [ ] "),
        "task list flat line should start with '- [ ] ', got: {:?}",
        first_text
    );
    assert!(
        first_text.contains("todo"),
        "task list flat line should contain 'todo', got: {:?}",
        first_text
    );
}

#[test]
fn heading_active_marker_projection_correct() {
    // "# Title" = 7 bytes: [0]='#', [1]=' ', [2]='T', [3]='i', [4]='t', [5]='l', [6]='e'
    let lazy = layout_with_cursor("# Title", 1);
    let fl = &lazy.flat_lines[0];
    let map = fl
        .source_projection
        .as_ref()
        .expect("should have projection")
        .boundaries
        .iter()
        .map(|anchor| anchor.byte.get())
        .collect::<Vec<_>>();
    // Visual chars: '#' (0), ' ' (1), 'T' (2), 'i' (3), 't' (4), 'l' (5), 'e' (6), sentinel (7)
    assert!(
        map.len() >= 8,
        "projection should have 8+ boundaries (7 chars + sentinel), got {}",
        map.len()
    );
    assert_eq!(map[0], 0, "visual char 0 '#' → source byte 0");
    assert_eq!(map[1], 1, "visual char 1 ' ' → source byte 1");
    assert_eq!(map[2], 2, "visual char 2 'T' → source byte 2");
    assert_eq!(map[3], 3, "visual char 3 'i' → source byte 3");
    assert_eq!(map[6], 6, "visual char 6 'e' → source byte 6");
    // Sentinel should point past the last byte (7 = length of "# Title")
    assert_eq!(map[7], 7, "sentinel should be 7 (len of '# Title')");
}

#[test]
fn list_item_active_marker_projection_correct() {
    // "- item" = 6 bytes: [0]='-', [1]=' ', [2]='i', [3]='t', [4]='e', [5]='m'
    let lazy = layout_with_cursor("- item", 3);
    let fl = &lazy.flat_lines[0];
    let map = fl
        .source_projection
        .as_ref()
        .expect("should have projection")
        .boundaries
        .iter()
        .map(|anchor| anchor.byte.get())
        .collect::<Vec<_>>();
    assert!(
        map.len() >= 7,
        "projection should have 7+ boundaries (6 chars + sentinel), got {}",
        map.len()
    );
    assert_eq!(map[0], 0, "visual char 0 '-' → source byte 0");
    assert_eq!(map[1], 1, "visual char 1 ' ' → source byte 1");
    assert_eq!(map[2], 2, "visual char 2 'i' → source byte 2");
    assert_eq!(map[6], 6, "sentinel should be 6 (len of '- item')");
}

#[test]
fn heading_with_inline_bold_keeps_hidden_prefix_projection() {
    let lazy = layout_with_cursor("# **bold**", 5);
    let line = &lazy.flat_lines[0];
    assert_eq!(line.text, "**bold**");
    let boundaries = line
        .source_projection
        .as_ref()
        .expect("heading projection")
        .boundaries
        .iter()
        .map(|anchor| anchor.byte.get())
        .collect::<Vec<_>>();
    assert_eq!(boundaries, (2..=10).collect::<Vec<_>>());
}

#[test]
fn ordered_list_active_marker() {
    // "3. item" = 7 bytes: [0]='3', [1]='.', [2]=' ', [3]='i', [4]='t', [5]='e', [6]='m'
    let lazy = layout_with_cursor("3. item", 4);
    let first_text = &lazy.flat_lines[0].text;
    assert!(
        first_text.starts_with("3. "),
        "ordered list should start with '3. ', got: {:?}",
        first_text
    );
}

#[test]
fn activating_ordered_list_materializes_only_the_current_source_number() {
    let source = "1. first\n7. second\n4. third";
    let cursor = source.find("second").expect("fixture must contain the active item");
    let layout = layout_with_cursor(source, cursor);
    let rendered_lines =
        layout.flat_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

    assert_eq!(rendered_lines, ["first", "7. second", "third"]);
}

#[test]
fn moving_cursor_into_ordered_list_invalidates_only_the_current_item() {
    let source = "1. first\n7. second\n4. third";
    let cursor = source.find("second").expect("fixture must contain the active item");
    let style = default_style();
    let parsed = crate::parser::parse_markdown(source);
    let document = crate::builder::MarkdownDoc::build(&parsed, &style);
    let document_view = textora_core::document::StringDocView::new(source);
    let mut layout = LazyLayout::from_doc(document, &style, 400.0, &document_view);
    layout.set_edit_source(Some(source.to_owned()));
    layout.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: cursor,
        preedit_text: None,
        preedit_cursor: None,
    }));

    layout.invalidate_lines_for_source_bytes([cursor]);
    layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
    layout.build_flat_lines(&document_view);

    let rendered_lines =
        layout.flat_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
    assert_eq!(rendered_lines, ["first", "7. second", "third"]);
}

#[cfg(feature = "rich-markdown")]
#[test]
fn changing_selection_invalidates_embedded_block_on_enter_and_exit() {
    let source = "before\n\n```mermaid\ngraph TD\nA-->B\n```";
    let mut layout = layout_with_cursor(source, 0);
    let diagram_start = source.find("graph TD").expect("diagram source");
    assert!(layout.laid_out.iter().any(Option::is_some));
    layout.set_selection_range(Some(diagram_start..diagram_start + 5));
    assert!(layout.laid_out.iter().any(Option::is_none));
    let style = default_style();
    let document_view = textora_core::document::StringDocView::new(source);
    layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
    assert!(layout.laid_out.iter().all(Option::is_some));
    layout.set_selection_range(None);
    assert!(layout.laid_out.iter().any(Option::is_none));
}

// ===== y-stability: marker prepend must not shift subsequent blocks =====

/// Build two LazyLayouts of the same markdown, one with cursor outside the
/// first list item (no marker) and one with cursor inside it (marker active).
/// Returns `(lazy_outside, lazy_inside)`.
fn layout_list_pair_outside_and_inside(
    md: &str,
) -> (LazyLayout<crate::builder::MarkdownDoc>, LazyLayout<crate::builder::MarkdownDoc>) {
    // Cursor in the SECOND item -> first item gets NO marker
    let outside = layout_with_cursor(md, 10);
    // Cursor in the FIRST item -> first item gets marker
    let inside = layout_with_cursor(md, 3);
    (outside, inside)
}

#[test]
fn list_item_y_stable_when_cursor_moves_into_marker() {
    // "- first\n- second\n" = 17 bytes
    // first item block_range covers bytes 0..8, second item 8..17
    let md = "- first\n- second\n";
    let (outside, inside) = layout_list_pair_outside_and_inside(md);

    assert!(
        outside.laid_out.len() >= 2,
        "expected at least 2 laid-out blocks, got {}",
        outside.laid_out.len()
    );
    assert!(
        inside.laid_out.len() >= 2,
        "expected at least 2 laid-out blocks, got {}",
        inside.laid_out.len()
    );

    // Verify first item gets marker when cursor inside, not when outside
    let first_outside_text = &outside.flat_lines[0].text;
    let first_inside_text = &inside.flat_lines[0].text;
    assert!(
        !first_outside_text.starts_with("- "),
        "first item should NOT have marker when cursor is outside, got: {:?}",
        first_outside_text
    );
    assert!(
        first_inside_text.starts_with("- "),
        "first item should have marker when cursor is inside, got: {:?}",
        first_inside_text
    );

    // The critical assertion: second block's real y must be unchanged.
    let second_outside_y = outside.laid_out[1].as_ref().unwrap().rect.y
        + outside.y_delta.get(1).copied().unwrap_or(0.0);
    let second_inside_y =
        inside.laid_out[1].as_ref().unwrap().rect.y + inside.y_delta.get(1).copied().unwrap_or(0.0);

    assert!(
        (second_outside_y - second_inside_y).abs() < 0.5,
        "second list item y shifted: outside={:.1} vs inside={:.1} (delta={:.1})",
        second_outside_y,
        second_inside_y,
        second_outside_y - second_inside_y,
    );
}

#[test]
fn list_item_height_stable_when_cursor_moves_into_marker() {
    // Same as above but also check that first block's height is unchanged.
    let md = "- first\n- second\n";
    let (outside, inside) = layout_list_pair_outside_and_inside(md);

    let first_outside_h = outside.laid_out[0].as_ref().unwrap().rect.h;
    let first_inside_h = inside.laid_out[0].as_ref().unwrap().rect.h;

    assert!(
        (first_outside_h - first_inside_h).abs() < 0.5,
        "first list item height changed: outside={:.1} vs inside={:.1}",
        first_outside_h,
        first_inside_h,
    );
}

#[test]
fn total_height_stable_when_list_marker_appears() {
    // Total document height must not change when only marker visibility changes.
    let md = "- first\n- second\n";
    let (outside, inside) = layout_list_pair_outside_and_inside(md);

    assert!(
        (outside.total_height - inside.total_height).abs() < 0.5,
        "total_height changed: outside={:.1} vs inside={:.1} (delta={:.1})",
        outside.total_height,
        inside.total_height,
        outside.total_height - inside.total_height,
    );
}

#[test]
fn heading_y_stable_when_cursor_moves_into_marker() {
    // Same property for headings: marker prepend must not shift subsequent blocks.
    let md = "# Title\n\nparagraph";
    // Cursor at byte 10 (in "paragraph") -> heading has no marker
    let outside = layout_with_cursor(md, 10);
    // Cursor at byte 1 (in the prefix) -> heading has marker
    let inside = layout_with_cursor(md, 1);

    // Heading text check
    assert!(
        !outside.flat_lines[0].text.starts_with("# "),
        "heading should NOT have marker when cursor outside"
    );
    assert!(
        inside.flat_lines[0].text.starts_with("# "),
        "heading should have marker when cursor inside"
    );

    // Second block y stability
    if outside.laid_out.len() >= 2 && inside.laid_out.len() >= 2 {
        let second_outside_y = outside.laid_out[1].as_ref().unwrap().rect.y
            + outside.y_delta.get(1).copied().unwrap_or(0.0);
        let second_inside_y = inside.laid_out[1].as_ref().unwrap().rect.y
            + inside.y_delta.get(1).copied().unwrap_or(0.0);

        assert!(
            (second_outside_y - second_inside_y).abs() < 0.5,
            "second block y shifted when heading marker appeared: outside={:.1} vs inside={:.1}",
            second_outside_y,
            second_inside_y,
        );
    }
}

#[test]
fn blockquote_y_stable_when_cursor_moves_into_marker() {
    // Use a doc with blockquote + paragraph so we can place cursor
    // outside the blockquote to test marker on/off while checking
    // subsequent block y stability.
    let md = "> quoted\n\nparagraph";
    // "> quoted\n\nparagraph"
    // [0]='>', [1]=' ', [2..8]="quoted", [8]='\n', [9]='\n',
    // [10]='p', [11..18]="aragraph"
    // blockquote block_range: 0..9, paragraph block_range: 9..19

    // Cursor at byte 12 (in "paragraph") -> blockquote has no marker
    let outside = layout_with_cursor(md, 12);
    // Cursor at byte 3 (in "quoted") -> blockquote has marker
    let inside = layout_with_cursor(md, 3);

    // Blockquote text check
    assert!(
        !outside.flat_lines[0].text.starts_with("> "),
        "blockquote should NOT have marker when cursor outside, got: {:?}",
        outside.flat_lines[0].text
    );
    assert!(
        inside.flat_lines[0].text.starts_with("> "),
        "blockquote should have marker when cursor inside, got: {:?}",
        inside.flat_lines[0].text
    );

    // Height stability
    let first_outside_h = outside.laid_out[0].as_ref().unwrap().rect.h;
    let first_inside_h = inside.laid_out[0].as_ref().unwrap().rect.h;
    assert!(
        (first_outside_h - first_inside_h).abs() < 0.5,
        "blockquote height changed: outside={:.1} vs inside={:.1}",
        first_outside_h,
        first_inside_h,
    );

    // Second block y stability
    if outside.laid_out.len() >= 2 && inside.laid_out.len() >= 2 {
        let second_outside_y = outside.laid_out[1].as_ref().unwrap().rect.y
            + outside.y_delta.get(1).copied().unwrap_or(0.0);
        let second_inside_y = inside.laid_out[1].as_ref().unwrap().rect.y
            + inside.y_delta.get(1).copied().unwrap_or(0.0);
        assert!(
            (second_outside_y - second_inside_y).abs() < 0.5,
            "paragraph y shifted when blockquote marker appeared: outside={:.1} vs inside={:.1}",
            second_outside_y,
            second_inside_y,
        );
    }
}
