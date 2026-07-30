use textora_markdown::builder::MarkdownDoc;
use textora_markdown::layout::layout_doc_for_rendering;
use textora_markdown::parser::parse_markdown;
use textora_markdown::render::render_layout;
use textora_markdown::style::MarkdownStyle;
use ui::core::paint::DrawCmd;

#[test]
fn public_layout_and_render_combination_preserves_ascii_diagram_grid_geometry() {
    let source = "```\n┌────┐\n│中文│\n└────┘\n```";
    let style = MarkdownStyle::from_theme(&ui::theme::test_theme(), 15.0, 24.0);
    let parsed = parse_markdown(source);
    let document = MarkdownDoc::build(&parsed, &style);
    let source_view = core::document::StringDocView::new(source);
    let layout = layout_doc_for_rendering(&document.blocks, &style, 400.0, &source_view);
    let mut shaper = shaping::Shaper::new().expect("grid rendering test requires a shaper");
    let mut draw_list = ui::core::paint::DrawList::new();

    render_layout(&layout, &style, &mut draw_list, 0.0, 600.0, Some(&mut shaper));

    assert!(
        draw_list.cmds.iter().any(|command| {
            matches!(command, DrawCmd::FillRect { rect, .. } if rect.w <= 2.0 && rect.h > 8.0)
        }),
        "the public layout-to-render path must retain fixed-grid box geometry"
    );
}
