//! Markdown rendering for edit+.
#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::empty_line_after_doc_comments)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::len_zero)]

pub mod augmenter;
pub mod builder;
pub mod commands;
pub mod edit;
pub mod edit_context;
pub mod grapheme_map;
pub mod layout;
pub mod parser;
pub(crate) mod projection;
pub mod render;
pub mod search;
pub mod selection;
pub mod style;
pub mod view;

pub mod mindmap_view;
pub mod mmf;

use ui::core::paint::DrawList;

/// Clamp a byte index to a valid char boundary in `s` (snap left).
/// In debug builds, fires an assertion so invalid offsets are caught early;
/// in release builds, silently falls back to the nearest safe boundary.
#[inline]
pub(crate) fn safe_byte_idx(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        s.len()
    } else {
        debug_assert!(
            s.is_char_boundary(idx),
            "byte index {idx} is not a char boundary in string of len {}",
            s.len()
        );
        s.floor_char_boundary(idx)
    }
}

/// Render markdown source text into a DrawList.
///
/// This is the main public API — calls the full pipeline:
/// parse → build → layout → render.
pub fn render_markdown(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
) -> DrawList {
    let mut shaper = shaping::Shaper::new().ok();
    render_markdown_with_shaper(src, style, viewport_w, viewport_h, scroll_y, shaper.as_mut())
}

/// Render with optional shaper for precise text measurement.
pub fn render_markdown_with_shaper(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    shaper: Option<&mut shaping::Shaper>,
) -> DrawList {
    render_markdown_with_offset(src, style, viewport_w, viewport_h, scroll_y, shaper, 0.0, 0.0)
}

/// Render with pixel offset for positioning inside editor content area.
pub fn render_markdown_with_offset(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    shaper: Option<&mut shaping::Shaper>,
    offset_x: f32,
    offset_y: f32,
) -> DrawList {
    render_markdown_with_highlighter(
        src, style, viewport_w, viewport_h, scroll_y, shaper, offset_x, offset_y, None,
    )
}

/// Render with optional syntax highlighting for code blocks.
pub fn render_markdown_with_highlighter(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
    offset_x: f32,
    offset_y: f32,
    highlighter: Option<&dyn builder::CodeHighlighter>,
) -> DrawList {
    let parsed = parser::parse_markdown(src);
    let doc = builder::MarkdownDoc::build(&parsed, style);
    let string_doc = core::document::StringDocView::new(src);
    let layout = layout::layout_doc_with_shaper_for_rendering(
        &doc.blocks,
        style,
        viewport_w,
        shaper.as_deref_mut(),
        highlighter,
        &string_doc,
    );
    let mut dl = DrawList::new();
    render::render_layout_with_offset(
        &layout,
        style,
        &mut dl,
        scroll_y,
        viewport_h,
        offset_x,
        offset_y,
        shaper,
        &[],
    );
    dl
}

/// Get the total content height for a markdown document (for scrollbar calculations).
pub fn markdown_content_height(src: &str, style: &style::MarkdownStyle, viewport_w: f32) -> f32 {
    let parsed = parser::parse_markdown(src);
    let doc = builder::MarkdownDoc::build(&parsed, style);
    let string_doc = core::document::StringDocView::new(src);
    let laid_out = layout::layout_doc(&doc.blocks, style, viewport_w, &string_doc);
    laid_out.total_height
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::style::MarkdownStyle;

    /// Default dark-theme style for testing (matches ui::theme::test_theme() + 15px/24px).
    pub fn default_style() -> MarkdownStyle {
        let theme = ui::theme::test_theme();
        MarkdownStyle::from_theme(&theme, 15.0, 24.0)
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use test_utils::default_style;

    #[test]
    fn e2e_paragraph_text_reaches_drawlist() {
        let dl = render_markdown("hello world", &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.contains(&"hello world"),
            "paragraph text missing from DrawList, got: {:?}",
            texts
        );
    }

    #[test]
    fn e2e_heading_text_reaches_drawlist() {
        let dl = render_markdown("# Big Title", &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.contains(&"Big Title"),
            "heading text missing from DrawList, got: {:?}",
            texts
        );
    }

    #[test]
    fn e2e_code_block_text_reaches_drawlist() {
        let dl = render_markdown("```\nlet x = 1;\n```", &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("let x = 1;")),
            "code block text missing, got: {:?}",
            texts
        );
    }

    #[test]
    fn e2e_list_items_reach_drawlist() {
        let dl = render_markdown("- alpha\n- beta", &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("alpha")),
            "list item 'alpha' missing, got: {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.contains("beta")),
            "list item 'beta' missing, got: {:?}",
            texts
        );
    }

    #[test]
    fn e2e_empty_doc_no_text_cmds() {
        let dl = render_markdown("", &default_style(), 400.0, 600.0, 0.0);
        let text_count =
            dl.cmds.iter().filter(|c| matches!(c, ui::core::DrawCmd::TextLayout { .. })).count();
        assert_eq!(text_count, 0, "empty doc should have no text commands");
    }

    #[test]
    fn e2e_scroll_offset_applied() {
        // Use multiple paragraphs to ensure content is taller than scroll offset
        let md = "line1\n\nline2\n\nline3\n\nline4\n\nline5\n\nline6\n\nline7\n\nline8";
        let dl0 = render_markdown(md, &default_style(), 400.0, 600.0, 0.0);
        let dl10 = render_markdown(md, &default_style(), 400.0, 600.0, 10.0);
        // First text command should have different y positions
        let y0 = dl0.cmds.iter().find_map(|c| {
            if let ui::core::DrawCmd::TextLayout { y_baseline, .. } = c {
                Some(*y_baseline)
            } else {
                None
            }
        });
        let y10 = dl10.cmds.iter().find_map(|c| {
            if let ui::core::DrawCmd::TextLayout { y_baseline, .. } = c {
                Some(*y_baseline)
            } else {
                None
            }
        });
        assert!(
            y0.is_some() && y10.is_some(),
            "both renders should produce text, y0={:?} y10={:?}",
            y0,
            y10
        );
        assert_ne!(y0, y10, "scroll offset should change text y position");
    }

    #[test]
    fn e2e_content_height_positive() {
        let h = markdown_content_height("# Title\n\nParagraph text.", &default_style(), 400.0);
        assert!(h > 0.0, "content height should be positive, got {}", h);
    }

    #[test]
    fn e2e_content_height_grows_with_content() {
        let h_short = markdown_content_height("hi", &default_style(), 400.0);
        let h_long = markdown_content_height(
            "# H1\n\n## H2\n\nparagraph\n\n- list\n- items\n\n> quote",
            &default_style(),
            400.0,
        );
        assert!(
            h_long > h_short,
            "longer doc should have greater height: {} vs {}",
            h_long,
            h_short
        );
    }

    #[test]
    fn e2e_table_cells_have_content() {
        let md = "| A | B |\n|---|---|\n| hello | world |";
        let dl = render_markdown(md, &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("hello")),
            "table cell 'hello' missing, got: {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.contains("world")),
            "table cell 'world' missing, got: {:?}",
            texts
        );
    }

    #[test]
    fn e2e_ordered_list() {
        let dl =
            render_markdown("1. first\n2. second\n3. third", &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("first")),
            "ordered list 'first' missing, got: {:?}",
            texts
        );
        assert!(texts.iter().any(|t| t == "1."), "ordered number '1.' missing, got: {:?}", texts);
        assert!(texts.iter().any(|t| t == "2."), "ordered number '2.' missing, got: {:?}", texts);
        assert!(texts.iter().any(|t| t == "3."), "ordered number '3.' missing, got: {:?}", texts);
    }

    #[test]
    fn e2e_ordered_list_long_items_with_style() {
        // Regression: long ordered list items with bold/inline-code/CJK should
        // get sequential numbers (not all "1."). Verifies builder numbering
        // survives the full parse→build→layout→render pipeline.
        let md = r#"1. **first** item with `code`
2. **second** item 第二项
3. **third** item 第三项"#;
        let dl = render_markdown(md, &default_style(), 400.0, 600.0, 0.0);
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t == "1."), "ordered '1.' missing, got: {:?}", texts);
        assert!(texts.iter().any(|t| t == "2."), "ordered '2.' missing, got: {:?}", texts);
        assert!(texts.iter().any(|t| t == "3."), "ordered '3.' missing, got: {:?}", texts);
    }

    #[test]
    fn e2e_mixed_document() {
        let md = "# Title\n\nSome **bold** text.\n\n- item\n\n> quote\n\n---";
        let dl = render_markdown(md, &default_style(), 400.0, 800.0, 0.0);
        let has_text = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::TextLayout { .. }));
        let has_fill = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::FillRect { .. }));
        assert!(has_text, "mixed doc should have text commands");
        assert!(has_fill, "mixed doc should have fill commands");
    }

    #[test]
    fn e2e_yaml_metadata_block_reaches_drawlist() {
        let md = "---
title: hello
---

Body text";
        let dl = render_markdown(md, &default_style(), 400.0, 800.0, 0.0);
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        // Metadata block content should be rendered
        assert!(
            texts.iter().any(|t| t.contains("title: hello")),
            "YAML metadata content missing, got: {:?}",
            texts
        );
        // Body text should also be rendered
        assert!(
            texts.iter().any(|t| t.contains("Body text")),
            "Body text missing, got: {:?}",
            texts
        );
    }
}

// Style-specific e2e tests
#[cfg(test)]
mod style_tests {
    use super::*;
    use test_utils::default_style as dark_style;

    #[test]
    fn link_uses_link_color() {
        let dl =
            render_markdown("[click me](https://example.com)", &dark_style(), 400.0, 600.0, 0.0);
        // Find the link text
        let link_cmds: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, color, .. } = c {
                    if layout.text.contains("click") || layout.text.contains("me") {
                        Some((layout.text.as_str(), *color))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        // Link text should use link_color
        let link_color = dark_style().link_color;
        assert!(
            link_cmds.iter().any(|(_, c)| *c == link_color),
            "link text should use link_color {:?}, got {:?}",
            link_color,
            link_cmds
        );
    }

    #[test]
    fn inline_code_uses_code_color() {
        let dl = render_markdown("use `println!` here", &dark_style(), 400.0, 600.0, 0.0);
        let code_cmds: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, color, .. } = c {
                    if layout.text.contains("println") {
                        Some((layout.text.as_str(), *color))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        let code_color = dark_style().code_color;
        assert!(
            code_cmds.iter().any(|(_, c)| *c == code_color),
            "inline code should use code_color {:?}, got {:?}",
            code_color,
            code_cmds
        );
    }

    #[test]
    fn bold_text_uses_base_color() {
        let dl = render_markdown("some **bold** text", &dark_style(), 400.0, 600.0, 0.0);
        let bold_cmds: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, color, .. } = c {
                    if layout.text.contains("bold") {
                        Some((layout.text.as_str(), *color))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        let base_color = dark_style().text_color;
        assert!(
            bold_cmds.iter().any(|(_, c)| *c == base_color),
            "bold text should use base text_color {:?}, got {:?}",
            base_color,
            bold_cmds
        );
    }

    #[test]
    fn mixed_styles_have_multiple_colors() {
        let md = "normal [link](https://x.com) and `code`";
        let dl = render_markdown(md, &dark_style(), 400.0, 600.0, 0.0);
        let colors: Vec<[f32; 4]> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { color, .. } = c {
                    Some(*color)
                } else {
                    None
                }
            })
            .collect();
        let unique_count = {
            let mut sorted = colors.clone();
            sorted.dedup_by(|a, b| {
                (a[0] - b[0]).abs() < 0.001
                    && (a[1] - b[1]).abs() < 0.001
                    && (a[2] - b[2]).abs() < 0.001
            });
            sorted.len()
        };
        assert!(
            unique_count >= 2,
            "mixed doc should have >= 2 unique colors, got {}",
            unique_count
        );
    }
}

// TaskListMarker tests
#[cfg(test)]
mod tasklist_tests {
    use super::*;
    use test_utils::default_style as dark_style;

    #[test]
    fn tasklist_marker_parsed() {
        let md = "- [x] done\n- [ ] todo";
        let parsed = parser::parse_markdown(md);
        let markers: Vec<_> = parsed
            .events
            .iter()
            .filter(|e| matches!(e, parser::MarkdownEvent::TaskListMarker(_)))
            .collect();
        assert_eq!(markers.len(), 2, "should parse 2 task list markers");
    }

    #[test]
    fn tasklist_renders_checkbox() {
        let md = "- [x] done\n- [ ] todo";
        let dl = render_markdown(md, &dark_style(), 400.0, 600.0, 0.0);
        // Should have StrokeRect for checkbox outlines
        let has_stroke = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::StrokeRect { .. }));
        assert!(has_stroke, "task list should render checkbox StrokeRect");
        // Should have text content
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("done")), "should contain 'done' text");
        assert!(texts.iter().any(|t| t.contains("todo")), "should contain 'todo' text");
    }
}

// Boundary and edge case tests
#[cfg(test)]
mod boundary_tests {
    use super::*;
    use test_utils::default_style as dark_style;

    #[test]
    fn cjk_text_reaches_drawlist() {
        let dl = render_markdown("你好世界", &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("你好世界")), "CJK text missing, got: {:?}", texts);
    }

    #[test]
    fn emoji_text_no_panic() {
        let dl = render_markdown("🎉🚀✨ emoji test", &dark_style(), 400.0, 600.0, 0.0);
        let has_text = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::TextLayout { .. }));
        assert!(has_text, "emoji text should produce text commands");
    }

    #[test]
    fn nested_bold_italic() {
        let dl = render_markdown("***bold and italic***", &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("bold and italic")),
            "nested style text missing, got: {:?}",
            texts
        );
    }

    #[test]
    fn strikethrough_renders() {
        let dl = render_markdown("~~deleted~~", &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("deleted")),
            "strikethrough text missing, got: {:?}",
            texts
        );
    }

    #[test]
    fn very_long_line_no_panic() {
        let long_text = "a".repeat(10000);
        let dl = render_markdown(&long_text, &dark_style(), 400.0, 600.0, 0.0);
        // Should not panic, should produce some output
        let _ = dl.cmds.len();
    }

    #[test]
    fn cjk_inline_code_narrow_table_cell_no_panic() {
        // Regression: exact text from the crash report.
        // The inline code span's byte boundary lands on a CJK char boundary;
        // wrapping in a narrow cell must not produce invalid byte offsets.
        let md = "| 状图（├── `mod.rs # comment`）整行长度可能 > 200px，在 188px 的窄 cell 中既填不满也不会正确折行—— |\n|---|\n| short |";
        let style = dark_style();
        // viewport_w=1577 matches the crash; table cells are narrower internally
        let dl = render_markdown(md, &style, 1577.0, 800.0, 0.0);
        assert!(!dl.cmds.is_empty());
    }

    #[test]
    fn multiple_blank_lines() {
        let dl = render_markdown("a\n\n\n\nb", &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("a")), "text 'a' missing");
        assert!(texts.iter().any(|t| t.contains("b")), "text 'b' missing");
    }

    #[test]
    fn tasklist_mixed_with_regular_list() {
        let md = "- [x] checked\n- normal\n- [ ] unchecked";
        let dl = render_markdown(md, &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("checked")), "missing 'checked'");
        assert!(texts.iter().any(|t| t.contains("normal")), "missing 'normal'");
    }

    #[test]
    fn table_with_empty_cells() {
        let md = "| A | |\n|---|---|\n| | B |";
        let dl = render_markdown(md, &dark_style(), 400.0, 600.0, 0.0);
        // Should not panic, table should render
        let has_fill = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::FillRect { .. }));
        assert!(has_fill, "table should have fill commands");
    }
    #[test]
    fn blockquote_with_inline_styles() {
        let dl = render_markdown("> **bold** in quote", &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("bold")), "bold text in blockquote missing");
    }

    #[test]
    fn scroll_negative_clamps_to_zero() {
        // Should not panic with negative scroll
        let dl = render_markdown("text", &dark_style(), 400.0, 600.0, -100.0);
        assert!(dl.cmds.len() >= 2, "should have at least clip commands");
    }

    #[test]
    fn zero_viewport_width_no_panic() {
        let dl = render_markdown("test", &dark_style(), 0.0, 600.0, 0.0);
        let _ = dl.cmds.len();
    }

    #[test]
    fn heading_with_inline_code() {
        let dl = render_markdown("# Title with `code`", &dark_style(), 400.0, 600.0, 0.0);
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("Title")), "heading text missing");
        assert!(texts.iter().any(|t| t.contains("code")), "inline code in heading missing");
    }

    #[test]
    fn image_placeholder_renders() {
        let dl = render_markdown(
            "![alt text](https://example.com/img.png)",
            &dark_style(),
            400.0,
            600.0,
            0.0,
        );
        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("Image")),
            "image placeholder missing, got: {:?}",
            texts
        );
    }
    #[test]
    fn e2e_code_block_with_highlighting() {
        use crate::builder::{CodeHighlighter, HighlightSpan};

        struct MockHighlighter;
        impl CodeHighlighter for MockHighlighter {
            fn highlight(&self, _language: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
                code.lines()
                    .map(|line| {
                        if line.contains("fn") {
                            vec![HighlightSpan {
                                start: 0,
                                len: 2,
                                color: [1.0, 0.5, 0.0, 1.0], // orange for keyword
                            }]
                        } else {
                            vec![]
                        }
                    })
                    .collect()
            }
        }

        let md = "```rust\nfn main() {\n    let x = 1;\n}\n```";
        let style = dark_style();
        let hl: &dyn CodeHighlighter = &MockHighlighter;
        let mut shaper = shaping::Shaper::new().ok();
        let dl = render_markdown_with_highlighter(
            md,
            &style,
            400.0,
            600.0,
            0.0,
            shaper.as_mut(),
            0.0,
            0.0,
            Some(hl),
        );

        // Verify that "fn" in the code block has the orange highlight color
        let orange_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { layout, color, .. } = c {
                    if layout.text.contains("fn") && color[0] > 0.9 && color[1] < 0.6 {
                        Some(layout.text.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        assert!(!orange_texts.is_empty(), "keyword 'fn' should have highlight color");
    }

    #[test]
    fn e2e_code_block_without_language_no_highlight() {
        use crate::builder::{CodeHighlighter, HighlightSpan};

        struct MockHighlighter;
        impl CodeHighlighter for MockHighlighter {
            fn highlight(&self, _language: &str, _code: &str) -> Vec<Vec<HighlightSpan>> {
                vec![vec![HighlightSpan { start: 0, len: 2, color: [1.0, 0.5, 0.0, 1.0] }]]
            }
        }

        // No language tag — highlighter should NOT be called
        let md = "```\nfn main() {\n}\n```";
        let style = dark_style();
        let hl: &dyn CodeHighlighter = &MockHighlighter;
        let mut shaper = shaping::Shaper::new().ok();
        let dl = render_markdown_with_highlighter(
            md,
            &style,
            400.0,
            600.0,
            0.0,
            shaper.as_mut(),
            0.0,
            0.0,
            Some(hl),
        );

        // "fn" should NOT have orange color (should be base code color)
        let orange_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { color, .. } = c {
                    if color[0] > 0.9 && color[1] < 0.6 { Some(()) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert!(orange_texts.is_empty(), "no language tag means no highlighting");
    }
    #[test]
    fn e2e_empty_code_block_no_panic() {
        use crate::builder::{CodeHighlighter, HighlightSpan};

        struct MockHighlighter;
        impl CodeHighlighter for MockHighlighter {
            fn highlight(&self, _language: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
                // Return spans for each line
                code.lines().map(|_| vec![]).collect()
            }
        }

        // Empty code block: ```rust + ```
        let md = "```rust
```";
        let style = dark_style();
        let hl: &dyn CodeHighlighter = &MockHighlighter;
        let mut shaper = shaping::Shaper::new().ok();
        let dl = render_markdown_with_highlighter(
            md,
            &style,
            400.0,
            600.0,
            0.0,
            shaper.as_mut(),
            0.0,
            0.0,
            Some(hl),
        );
        // Should not panic; empty code block produces minimal output
        assert!(dl.cmds.len() >= 1, "empty code block should still render background");
    }

    #[test]
    fn e2e_highlighter_returns_fewer_lines() {
        use crate::builder::{CodeHighlighter, HighlightSpan};

        struct ShortHighlighter;
        impl CodeHighlighter for ShortHighlighter {
            fn highlight(&self, _language: &str, _code: &str) -> Vec<Vec<HighlightSpan>> {
                // Return only 1 line of spans for a 3-line block
                vec![vec![HighlightSpan { start: 0, len: 2, color: [1.0, 0.0, 0.0, 1.0] }]]
            }
        }

        let md = "```rust
fn main() {
    let x = 1;
}
```";
        let style = dark_style();
        let hl: &dyn CodeHighlighter = &ShortHighlighter;
        let mut shaper = shaping::Shaper::new().ok();
        let dl = render_markdown_with_highlighter(
            md,
            &style,
            400.0,
            600.0,
            0.0,
            shaper.as_mut(),
            0.0,
            0.0,
            Some(hl),
        );
        // Should not panic; lines without spans render with base color
        let red_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { color, .. } = c {
                    if color[0] > 0.9 && color[1] < 0.1 && color[2] < 0.1 { Some(()) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(red_texts.len(), 1, "only first line should have red highlight");
    }

    #[test]
    fn e2e_highlighter_returns_more_lines() {
        use crate::builder::{CodeHighlighter, HighlightSpan};

        struct LongHighlighter;
        impl CodeHighlighter for LongHighlighter {
            fn highlight(&self, _language: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
                // Return 10 lines of spans for a 3-line block
                let n = code.lines().count();
                let mut result: Vec<Vec<HighlightSpan>> = code.lines().map(|_| vec![]).collect();
                // Add extra entries
                for _ in n..10 {
                    result.push(vec![HighlightSpan {
                        start: 0,
                        len: 1,
                        color: [0.0, 1.0, 0.0, 1.0],
                    }]);
                }
                result
            }
        }

        let md = "```rust
fn main() {
}
```";
        let style = dark_style();
        let hl: &dyn CodeHighlighter = &LongHighlighter;
        let mut shaper = shaping::Shaper::new().ok();
        let dl = render_markdown_with_highlighter(
            md,
            &style,
            400.0,
            600.0,
            0.0,
            shaper.as_mut(),
            0.0,
            0.0,
            Some(hl),
        );
        // Should not panic; extra lines are ignored via v.get(i) returning None
        let green_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::DrawCmd::TextLayout { color, .. } = c {
                    if color[1] > 0.9 && color[0] < 0.1 { Some(()) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert!(green_texts.is_empty(), "extra highlight lines should be ignored");
    }
}
