extern crate core as doc_core;

use std::borrow::Cow;

use doc_core::document::{DocView, DocViewMut, StringDocView};
use textora_markdown::style::MarkdownStyle;
use textora_markdown::view::{MarkdownEditorView, MarkdownRenderSettings, MarkdownView};
use ui::core::paint::{DrawCmd, DrawList};
use ui::plugin::{PluginMessage, ViewPlugin};

const VIEWPORT_WIDTH: f32 = 480.0;
const VIEWPORT_HEIGHT: f32 = 700.0;

fn style() -> MarkdownStyle {
    MarkdownStyle::from_theme(&ui::theme::test_theme(), 15.0, 24.0)
}

fn preview(source: &str, width: f32) -> DrawList {
    textora_markdown::render_markdown(source, &style(), width, VIEWPORT_HEIGHT, 0.0)
}

fn images(draw_list: &DrawList) -> Vec<(&ui::core::paint::RasterImage, ui::Rect)> {
    draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::Image { image, rect } => Some((image.as_ref(), *rect)),
            _ => None,
        })
        .collect()
}

fn visible_text(draw_list: &DrawList) -> String {
    draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn assert_visible_image(image: &ui::core::paint::RasterImage) {
    assert!(image.width() > 0 && image.height() > 0);
    assert_eq!(image.pixels().len(), image.width() as usize * image.height() as usize * 4);
    assert!(image.pixels().chunks_exact(4).any(|pixel| pixel[3] > 0));
}

#[test]
fn inline_formula_has_real_image_between_adjacent_words() {
    let draw_list = preview("before $x^2$ after", VIEWPORT_WIDTH);
    let image_commands = images(&draw_list);
    assert_eq!(image_commands.len(), 1);
    let (image, rect) = image_commands[0];
    assert_visible_image(image);
    assert_eq!(rect.w, image.width() as f32);
    assert_eq!(rect.h, image.height() as f32);

    let before_x = draw_list.cmds.iter().find_map(|command| match command {
        DrawCmd::TextLayout { layout, x, .. } if layout.text.contains("before") => Some(*x),
        _ => None,
    });
    let after_x = draw_list.cmds.iter().find_map(|command| match command {
        DrawCmd::TextLayout { layout, x, .. } if layout.text.contains("after") => Some(*x),
        _ => None,
    });
    assert!(before_x.expect("leading word remains visible") < rect.x);
    assert!(after_x.expect("trailing word remains visible") >= rect.x + rect.w);
}

#[test]
fn display_formula_and_mermaid_are_visible_image_blocks() {
    let source = "$$\\frac{1}{2}$$\n\n```mermaid\nflowchart LR\nA[开始] --> B[结束]\n```";
    let draw_list = preview(source, VIEWPORT_WIDTH);
    let image_commands = images(&draw_list);
    assert_eq!(image_commands.len(), 2);
    for (image, rect) in &image_commands {
        assert_visible_image(image);
        assert!(rect.w > 0.0 && rect.h > 0.0 && rect.w <= VIEWPORT_WIDTH);
        assert!((rect.w / image.width() as f32 - rect.h / image.height() as f32).abs() < 0.01);
    }
    assert!(image_commands[0].1.y + image_commands[0].1.h <= image_commands[1].1.y);
}

#[test]
fn math_symbols_in_literal_code_stay_as_text() {
    let source = "`$x^2$`\n\n```rust\n$$\\frac{1}{2}$$\nflowchart LR\nA --> B\n```";
    let draw_list = preview(source, VIEWPORT_WIDTH);
    assert!(images(&draw_list).is_empty());
    let text = visible_text(&draw_list);
    assert!(text.contains("$x^2$"));
    assert!(text.contains("$$\\frac{1}{2}$$"));
}

#[test]
fn invalid_math_and_mermaid_keep_readable_source() {
    let source = "$\\frac{1}{$\n\n```mermaid\nnot a diagram\n```";
    let draw_list = preview(source, VIEWPORT_WIDTH);
    assert!(images(&draw_list).is_empty());
    let text = visible_text(&draw_list);
    assert!(text.contains(r"$\frac{1}{$"), "math source disappeared: {text:?}");
    assert!(text.contains("```mermaid"), "diagram fence disappeared: {text:?}");
    assert!(text.matches("```").count() >= 2, "closing fence disappeared: {text:?}");
    assert!(text.contains("not a diagram"), "diagram source disappeared: {text:?}");
}

#[test]
fn formulas_in_list_and_table_cells_produce_images() {
    let source = "- $x^2$\n\n| Value |\n| --- |\n| $y^2$ |";
    let draw_list = preview(source, VIEWPORT_WIDTH);
    assert_eq!(images(&draw_list).len(), 2);
}

#[test]
fn narrow_chinese_text_wraps_without_overlapping_inline_formula() {
    let draw_list = preview("甲甲甲 $\\frac{1}{2}$ 乙乙乙", 100.0);
    let image_commands = images(&draw_list);
    assert_eq!(image_commands.len(), 1);
    let (_, image_rect) = image_commands[0];
    let text_lines: Vec<_> = draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::TextLayout { layout, x, y_baseline, .. } if !layout.text.trim().is_empty() => {
                Some((*x, *y_baseline, layout.shaped.width))
            }
            _ => None,
        })
        .collect();
    assert!(text_lines.iter().any(|(_, baseline, _)| *baseline > image_rect.y + image_rect.h));
    for (x, baseline, width) in text_lines {
        if baseline >= image_rect.y && baseline <= image_rect.y + image_rect.h {
            assert!(x + width <= image_rect.x || x >= image_rect.x + image_rect.w);
        }
    }
}

fn settings() -> MarkdownRenderSettings<'static> {
    MarkdownRenderSettings {
        font_family: "Arial",
        font_size: 15.0,
        line_height: 24.0,
        toc_max_depth: 3,
        markdown_first_line_indent: false,
        text_spacing_mode: ui::typography::TextSpacingMode::Natural,
    }
}

fn render_view(view: &mut MarkdownView, shaper: &mut shaping::Shaper) -> DrawList {
    view.render(
        &ui::theme::test_theme(),
        VIEWPORT_WIDTH,
        VIEWPORT_HEIGHT,
        0.0,
        0.0,
        settings(),
        Some(shaper),
    )
    .0
}

#[test]
fn editing_earlier_paragraph_preserves_later_formula_and_mermaid_images() {
    let suffix = "\n\n$x^2$\n\n```mermaid\nflowchart LR\nA --> B\n```";
    let mut view = MarkdownView::new();
    let mut shaper = shaping::Shaper::new().expect("view rendering requires a shaper");
    for (generation, prefix) in [(1, "intro"), (2, "intro changed"), (3, "intro\nextra line")] {
        view.set_source(format!("{prefix}{suffix}"), generation);
        let draw_list = render_view(&mut view, &mut shaper);
        assert_eq!(
            images(&draw_list).len(),
            2,
            "lost embedded image after generation {generation}"
        );
    }
}

struct EditableDocument(String);

impl DocView for EditableDocument {
    fn line_count(&self) -> usize {
        StringDocView::new(&self.0).line_count()
    }

    fn doc_line_text(&self, line: usize) -> Cow<'_, str> {
        Cow::Owned(StringDocView::new(&self.0).doc_line_text(line).into_owned())
    }

    fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> Cow<'_, str> {
        Cow::Borrowed(&self.0[range])
    }

    fn line_byte_offset(&self, line: usize) -> usize {
        StringDocView::new(&self.0).line_byte_offset(line)
    }

    fn line_byte_length(&self, line: usize) -> usize {
        StringDocView::new(&self.0).line_byte_length(line)
    }

    fn scroll_y(&self) -> f32 {
        0.0
    }

    fn viewport_height(&self) -> f32 {
        VIEWPORT_HEIGHT
    }
}

impl DocViewMut for EditableDocument {
    fn set_scroll_y(&mut self, _: f32) {}

    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.0.replace_range(range, text);
    }
}

fn render_editor(
    view: &mut MarkdownEditorView,
    document: &EditableDocument,
    shaper: &mut shaping::Shaper,
) -> DrawList {
    view.render(
        document,
        ui::Rect::new(0.0, 0.0, VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
        &ui::theme::test_theme(),
        shaper,
        1.0,
    )
}

#[test]
fn cursor_entering_formula_expands_source_and_leaving_restores_image() {
    let source = "before $x^2$ after";
    let mut document = EditableDocument(source.into());
    let mut view = MarkdownEditorView::new();
    let mut shaper = shaping::Shaper::new().expect("editor rendering requires a shaper");
    view.set_source(source.into(), 1);
    assert_eq!(images(&render_editor(&mut view, &document, &mut shaper)).len(), 1);

    let math_byte = source.find('x').expect("fixture contains math");
    view.handle_message(PluginMessage::SetCursorByte(math_byte), &mut document);
    let expanded = render_editor(&mut view, &document, &mut shaper);
    assert!(images(&expanded).is_empty());
    assert!(visible_text(&expanded).contains("x^2"));

    view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
    assert_eq!(images(&render_editor(&mut view, &document, &mut shaper)).len(), 1);
}

#[test]
fn cursor_entering_mermaid_expands_source_and_leaving_restores_image() {
    let source = "before\n\n```mermaid\nflowchart LR\nA --> B\n```\n\nafter";
    let mut document = EditableDocument(source.into());
    let mut view = MarkdownEditorView::new();
    let mut shaper = shaping::Shaper::new().expect("editor rendering requires a shaper");
    view.set_source(source.into(), 1);
    assert_eq!(images(&render_editor(&mut view, &document, &mut shaper)).len(), 1);

    let diagram_byte = source.find("flowchart").expect("fixture contains Mermaid source");
    view.handle_message(PluginMessage::SetCursorByte(diagram_byte), &mut document);
    let expanded = render_editor(&mut view, &document, &mut shaper);
    assert!(images(&expanded).is_empty());
    let text = visible_text(&expanded);
    assert!(text.contains("flowchart LR"));
    assert!(text.contains("```mermaid"));
    assert!(text.matches("```").count() >= 2);

    view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
    assert_eq!(images(&render_editor(&mut view, &document, &mut shaper)).len(), 1);
}

#[test]
fn selection_crossing_embedded_elements_expands_and_clear_restores_images() {
    let cases = [
        ("before $x^2$ after", "$x^2$"),
        ("before\n\n$$\\frac{1}{2}$$\n\nafter", "$$\\frac{1}{2}$$"),
        ("before\n\n```mermaid\nflowchart LR\nA --> B\n```\n\nafter", "```mermaid"),
    ];

    for (source, expanded_source) in cases {
        let mut document = EditableDocument(source.into());
        let mut view = MarkdownEditorView::new();
        let mut shaper = shaping::Shaper::new().expect("editor rendering requires a shaper");
        view.set_source(source.into(), 1);
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
        assert_eq!(images(&render_editor(&mut view, &document, &mut shaper)).len(), 1);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut document);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut document);
        assert_eq!(view.engine().selection_source_range(), Some((0, source.len())));
        let selected = render_editor(&mut view, &document, &mut shaper);
        assert!(images(&selected).is_empty(), "selected source still shows an image: {source:?}");
        assert!(
            visible_text(&selected).contains(expanded_source),
            "selected source is unreadable: {source:?}"
        );

        view.handle_message(PluginMessage::ClearSelection, &mut document);
        assert_eq!(
            images(&render_editor(&mut view, &document, &mut shaper)).len(),
            1,
            "clearing selection did not restore image: {source:?}"
        );
    }
}
