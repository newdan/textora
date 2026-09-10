use super::*;

const VIEWPORT_WIDTH: f32 = 800.0;
const VIEWPORT_HEIGHT: f32 = 600.0;
const NEAR_BOUNDARY_RATIO: f32 = 0.25;
const FAR_BOUNDARY_RATIO: f32 = 0.75;

fn render_rule_fixture(view: &mut MarkdownEditorView, source: &str, dpi_scale: f32) {
    let document = core::document::StringDocView::new(source);
    let theme = ui::theme::test_theme();
    let bounds = ui::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
    let mut shaper = shaping::Shaper::new().expect("rule interaction tests require a shaper");
    ViewPlugin::render(view, &document, bounds, &theme, &mut shaper, dpi_scale);
}

fn rule_fixture(source: &str, cursor: usize, dpi_scale: f32) -> MarkdownEditorView {
    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    view.engine.handle_set_cursor_byte(cursor);
    render_rule_fixture(&mut view, source, dpi_scale);
    view
}

#[test]
fn document_edge_rule_margins_remain_clickable_at_each_dpi() {
    for dpi_scale in [1.0, 1.5, 2.0] {
        for source in ["---\n\n正文", "正文\n\n---"] {
            let marker_start = source.find("---").expect("fixture contains a rule");
            let cursor = if marker_start == 0 { source.len() } else { 0 };
            let view = rule_fixture(source, cursor, dpi_scale);
            let rule = view
                .engine()
                .flat_lines()
                .iter()
                .find(|line| line.atomic_source_range.is_some())
                .expect("fixture contains an inactive rule");
            let rule_spacing = ui::theme::test_theme().markdown.spacing.rule_spacing * dpi_scale;
            let (click_y, expected_byte) = if marker_start == 0 {
                (rule.rect.y - rule_spacing * FAR_BOUNDARY_RATIO, marker_start)
            } else {
                (rule.rect.y + rule.rect.h + rule_spacing * FAR_BOUNDARY_RATIO, source.len())
            };
            assert_eq!(
                view.engine().hit_test_byte(rule.rect.x, click_y, 0.0, 0.0),
                Some(expected_byte),
                "document-edge rule margin must remain clickable: {source:?}, DPI={dpi_scale}"
            );
        }
    }
}

#[test]
fn rule_neighbors_share_gap_hit_regions_without_overlapping_text() {
    let source = "前段\n\n---\n\n后段";
    let marker_start = source.find("---").expect("fixture contains a rule");
    let marker_end = marker_start + "---".len();
    let after_start = source.find("后段").expect("fixture contains a following paragraph");
    for dpi_scale in [1.0, 1.5, 2.0] {
        let view = rule_fixture(source, 0, dpi_scale);
        let lines = view.engine().flat_lines();
        assert_eq!(lines.len(), 3);
        let before_bottom = lines[0].rect.y + lines[0].rect.h;
        let rule_top = lines[1].rect.y;
        let rule_bottom = rule_top + lines[1].rect.h;
        let after_top = lines[2].rect.y;
        let above_gap = rule_top - before_bottom;
        let below_gap = after_top - rule_bottom;
        for (click_y, expected_byte) in [
            (before_bottom + above_gap * NEAR_BOUNDARY_RATIO, "前段".len()),
            (before_bottom + above_gap * FAR_BOUNDARY_RATIO, marker_start),
            (rule_bottom + below_gap * NEAR_BOUNDARY_RATIO, marker_end),
            (rule_bottom + below_gap * FAR_BOUNDARY_RATIO, after_start),
        ] {
            assert_eq!(
                view.engine().hit_test_byte(lines[1].rect.x, click_y, 0.0, 0.0),
                Some(expected_byte),
                "gap ownership must match adjacent source boundaries at DPI={dpi_scale}"
            );
        }
    }
}

#[test]
fn editable_empty_paragraphs_beside_rule_keep_their_hit_targets() {
    for newline in ["\n", "\r\n"] {
        let source = "前段\n\n\n---\n\n\n后段".replace('\n', newline);
        let view = rule_fixture(&source, 0, 2.0);
        let empty_lines: Vec<_> = view
            .engine()
            .flat_lines()
            .iter()
            .filter(|line| line.text.is_empty() && line.atomic_source_range.is_none())
            .collect();
        assert_eq!(empty_lines.len(), 2);
        for line in empty_lines {
            let anchor = line
                .source_projection
                .as_ref()
                .and_then(|projection| projection.boundaries.first())
                .expect("editable empty paragraph has a source anchor");
            assert_eq!(
                view.engine()
                    .hit_test_byte(line.rect.x, line.rect.y + line.rect.h * 0.5, 0.0, 0.0,),
                Some(anchor.byte)
            );
        }
    }
}

#[test]
fn rule_activation_roundtrip_restores_geometry_and_total_height() {
    let source = "前段\n\n---\n\n后段";
    let marker_start = source.find("---").expect("fixture contains a rule");
    for dpi_scale in [1.0, 1.5, 2.0] {
        let mut view = rule_fixture(source, 0, dpi_scale);
        let before_rects: Vec<_> =
            view.engine().flat_lines().iter().map(|line| line.rect).collect();
        let before_height = view.engine.lazy.as_ref().expect("fixture has layout").total_height;
        view.engine.handle_set_cursor_byte(marker_start);
        render_rule_fixture(&mut view, source, dpi_scale);
        assert!(view.engine().flat_lines().iter().any(|line| line.text == "---"));
        view.engine.handle_set_cursor_byte(0);
        render_rule_fixture(&mut view, source, dpi_scale);
        let after_rects: Vec<_> = view.engine().flat_lines().iter().map(|line| line.rect).collect();
        let after_height = view.engine.lazy.as_ref().expect("fixture has layout").total_height;
        assert_eq!(before_rects, after_rects);
        assert_eq!(before_height, after_height);
    }
}
