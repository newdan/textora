use super::*;
use crate::builder::MarkdownDoc;
use crate::layout::layout_doc_with_shaper_for_rendering;
use crate::parser::parse_markdown;
use ui::core::DrawCmd;

const POSITION_TOLERANCE: f32 = 0.001;

fn assert_render_matches_layout(source: &str, width: f32, render_with_shaper: bool) {
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parse_markdown(source), &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("spacing regression requires system fonts");
    let laid_out = layout_doc_with_shaper_for_rendering(
        &doc.blocks,
        &style,
        width,
        Some(&mut shaper),
        None,
        &doc_view,
    );
    let LaidOutBlockKind::Text { lines } = &laid_out.doc.blocks[0].kind else {
        panic!("spacing fixture must produce a text block");
    };
    for line in lines {
        let mut draw_list = DrawList::new();
        render_line_with_offset(
            line,
            &style,
            &mut draw_list,
            0.0,
            0.0,
            0.0,
            render_with_shaper.then_some(&mut shaper),
        );
        let expected = line.shaped.as_ref().expect("precise layout retains glyph geometry");
        let mut rendered_clusters = Vec::new();
        let mut rendered_text = String::new();
        let mut expected_x = line.rect.x;
        for command in &draw_list.cmds {
            let DrawCmd::TextLayout { layout, x, .. } = command else { continue };
            assert!(
                (x - expected_x).abs() < POSITION_TOLERANCE,
                "{source}: rendered segment {} starts at {x}, expected {expected_x}",
                layout.text
            );
            let byte_start = rendered_text.len();
            rendered_text.push_str(&layout.text);
            expected_x += layout.shaped.width;
            for cluster in &layout.shaped.clusters {
                let mut cluster = cluster.clone();
                cluster.byte_range.start += byte_start;
                cluster.byte_range.end += byte_start;
                rendered_clusters.push(cluster);
            }
        }
        assert_eq!(rendered_text, line.text, "{source}: every projected character must render");
        assert_eq!(
            rendered_clusters, expected.clusters,
            "{source}: styled rendering must retain exact shaped glyphs, offsets and advances"
        );
    }
}

#[test]
fn styled_render_preserves_natural_spacing_in_prefix_span_and_tail() {
    for source in [
        "工作流agent",
        "工作流**agent**继续",
        "**工作流agent**继续",
        "工作流agent**加粗**工作流agent",
        "工作流*agent*继续",
        "工作流[agent](https://example.com)继续",
        "工作流~~agent~~继续",
        "使用`agent`继续",
        "English **agent** stays intact.",
    ] {
        assert_render_matches_layout(source, 800.0, true);
    }
}

#[test]
fn precomputed_styled_render_does_not_require_a_second_shaper() {
    assert_render_matches_layout("工作流**agent**继续", 800.0, false);
}

#[test]
fn wrapped_styled_render_preserves_final_line_geometry() {
    assert_render_matches_layout("工作流agent**继续agent**继续工作流agent", 100.0, true);
}
