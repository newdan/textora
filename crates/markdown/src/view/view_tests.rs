use super::*;

#[cfg(test)]
mod dpi_style_tests {
    use super::*;

    const LOGICAL_FONT_SIZE: f32 = 15.0;
    const LOGICAL_LINE_HEIGHT: f32 = 24.0;

    #[test]
    fn spacing_mode_message_updates_engine_state_once() {
        let mut engine: PreviewEngine = PreviewEngine::new();
        assert_eq!(engine.text_spacing_mode, ui::typography::TextSpacingMode::Natural);

        let message = PluginMessage::SetTextSpacingMode(ui::typography::TextSpacingMode::Verbatim);
        assert_eq!(engine.handle_message_common(&message), Some(true));
        assert_eq!(engine.text_spacing_mode, ui::typography::TextSpacingMode::Verbatim);
        assert_eq!(engine.handle_message_common(&message), Some(true));
        assert_eq!(engine.text_spacing_mode, ui::typography::TextSpacingMode::Verbatim);
    }

    fn assert_dimension_close(actual: f32, expected: f32, dimension: &str, dpi_scale: f32) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{dimension} at {dpi_scale}x DPI: actual={actual}, expected={expected}"
        );
    }

    fn render_bounds(dpi_scale: f32) -> ui::Rect {
        ui::Rect::new(0.0, 0.0, 800.0 * dpi_scale, 600.0 * dpi_scale)
    }

    #[test]
    fn preview_plugin_resolves_fixed_dimensions_at_render_dpi() {
        let theme = ui::theme::test_theme();

        for dpi_scale in [1.0, 1.5, 2.0] {
            let mut view = MarkdownView::new();
            view.set_source("before\n\n---\n\nafter".into(), 1);
            let document = core::document::StringDocView::new("");
            let mut shaper = shaping::Shaper::new().expect("test shaper must initialize");
            ViewPlugin::render(
                &mut view,
                &document,
                render_bounds(dpi_scale),
                &theme,
                &mut shaper,
                dpi_scale,
            );

            let style = view
                .engine
                .cached_query_style
                .as_ref()
                .expect("preview render must cache its resolved style");
            assert_dimension_close(
                style.rule_spacing,
                theme.markdown.spacing.rule_spacing * dpi_scale,
                "preview rule spacing",
                dpi_scale,
            );
            assert_dimension_close(
                style.rule_thickness,
                theme.markdown.spacing.rule_thickness * dpi_scale,
                "preview rule thickness",
                dpi_scale,
            );
            assert_dimension_close(
                style.paragraph_spacing,
                LOGICAL_LINE_HEIGHT * dpi_scale * theme.markdown.spacing.paragraph_spacing_ratio,
                "preview paragraph spacing",
                dpi_scale,
            );
        }
    }

    #[test]
    fn editor_plugin_resolves_fixed_dimensions_at_render_dpi() {
        let theme = ui::theme::test_theme();

        for dpi_scale in [1.0, 1.5, 2.0] {
            let mut view = MarkdownEditorView::new();
            view.set_source("before\n\n---\n\nafter".into(), 1);
            let document = core::document::StringDocView::new("before\n\n---\n\nafter");
            let mut shaper = shaping::Shaper::new().expect("test shaper must initialize");
            ViewPlugin::render(
                &mut view,
                &document,
                render_bounds(dpi_scale),
                &theme,
                &mut shaper,
                dpi_scale,
            );

            let style = view
                .engine
                .cached_query_style
                .as_ref()
                .expect("editor render must cache its resolved style");
            assert_dimension_close(
                style.rule_spacing,
                theme.markdown.spacing.rule_spacing * dpi_scale,
                "editor rule spacing",
                dpi_scale,
            );
            assert_dimension_close(
                style.rule_thickness,
                theme.markdown.spacing.rule_thickness * dpi_scale,
                "editor rule thickness",
                dpi_scale,
            );
            assert_dimension_close(
                style.body_font_size,
                LOGICAL_FONT_SIZE * dpi_scale,
                "editor font size",
                dpi_scale,
            );
        }
    }

    #[test]
    fn novel_plugin_resolves_fixed_dimensions_at_render_dpi() {
        let theme = ui::theme::test_theme();

        for dpi_scale in [1.0, 1.5, 2.0] {
            let mut view = NovelView::new();
            let document = core::document::StringDocView::new("chapter text");
            let mut shaper = shaping::Shaper::new().expect("test shaper must initialize");
            ViewPlugin::render(
                &mut view,
                &document,
                render_bounds(dpi_scale),
                &theme,
                &mut shaper,
                dpi_scale,
            );

            let style = view
                .engine
                .cached_query_style
                .as_ref()
                .expect("novel render must cache its resolved style");
            assert_dimension_close(
                style.rule_spacing,
                theme.novel.spacing.rule_spacing * dpi_scale,
                "novel rule spacing",
                dpi_scale,
            );
            assert_dimension_close(
                style.rule_thickness,
                theme.novel.spacing.rule_thickness * dpi_scale,
                "novel rule thickness",
                dpi_scale,
            );
            assert_dimension_close(
                style.paragraph_spacing,
                LOGICAL_LINE_HEIGHT * dpi_scale * theme.novel.spacing.paragraph_spacing_ratio,
                "novel paragraph spacing",
                dpi_scale,
            );
        }
    }

    #[test]
    fn changing_rule_spacing_rebuilds_cached_preview_geometry() {
        let mut theme = ui::theme::test_theme();
        let mut view = MarkdownView::new();
        view.set_source("before\n\n---\n\nafter".into(), 1);
        let settings = MarkdownRenderSettings {
            font_family: "PingFang SC",
            font_size: LOGICAL_FONT_SIZE,
            line_height: LOGICAL_LINE_HEIGHT,
            toc_max_depth: 3,
            markdown_first_line_indent: false,
            text_spacing_mode: ui::typography::TextSpacingMode::Natural,
        };

        view.render(&theme, 800.0, 600.0, 0.0, 0.0, settings, None);
        let initial_height = view.engine.content_height;
        theme.markdown.spacing.rule_spacing += LOGICAL_LINE_HEIGHT;
        view.render(&theme, 800.0, 600.0, 0.0, 0.0, settings, None);

        assert!(
            view.engine.content_height > initial_height,
            "changing rule spacing must invalidate and rebuild cached block geometry"
        );
    }

    #[test]
    fn every_block_geometry_dimension_participates_in_style_hash() {
        let theme = ui::theme::test_theme();
        let style = MarkdownStyle::from_theme(&theme, LOGICAL_FONT_SIZE, LOGICAL_LINE_HEIGHT);
        let expected_hash = style_hash_quick(&style);

        macro_rules! assert_dimension_hashed {
            ($field:ident) => {{
                let mut changed = style.clone();
                changed.$field += 1.0;
                assert_ne!(
                    style_hash_quick(&changed),
                    expected_hash,
                    concat!(stringify!($field), " must participate in the style hash")
                );
            }};
        }

        assert_dimension_hashed!(body_font_size);
        assert_dimension_hashed!(code_font_size);
        assert_dimension_hashed!(border_radius_base);
        assert_dimension_hashed!(border_radius_small);
        assert_dimension_hashed!(list_item_spacing);
        assert_dimension_hashed!(list_group_spacing);
        assert_dimension_hashed!(rule_spacing);
        assert_dimension_hashed!(paragraph_spacing);
        assert_dimension_hashed!(paragraph_first_line_indent);
        assert_dimension_hashed!(heading_spacing_top);
        assert_dimension_hashed!(heading_spacing_bottom);
        assert_dimension_hashed!(code_block_padding);
        assert_dimension_hashed!(blockquote_padding);
        assert_dimension_hashed!(list_indent);
        assert_dimension_hashed!(table_cell_padding);
        assert_dimension_hashed!(line_height);
        assert_dimension_hashed!(code_line_height);
        assert_dimension_hashed!(rule_thickness);
        assert_dimension_hashed!(rule_width_ratio);

        let mut changed_heading = style.clone();
        changed_heading.heading_font_sizes[0] += 1.0;
        assert_ne!(style_hash_quick(&changed_heading), expected_hash);
    }
}

#[cfg(test)]
mod heading_tests {
    use super::*;

    fn make_headings(entries: Vec<(u8, f32)>) -> Vec<HeadingEntry> {
        entries
            .into_iter()
            .map(|(level, y_offset)| HeadingEntry { text: format!("H{level}"), level, y_offset })
            .collect()
    }

    fn make_engine_with_headings(headings: Vec<HeadingEntry>) -> PreviewEngine {
        let mut e = PreviewEngine::new();
        e.headings = headings;
        e
    }

    #[test]
    fn current_heading_index_empty() {
        let e = make_engine_with_headings(vec![]);
        assert_eq!(e.current_heading_index(0.0), None);
    }

    #[test]
    fn current_heading_index_single_at_zero() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0)]));
        assert_eq!(e.current_heading_index(0.0), Some(0));
    }

    #[test]
    fn current_heading_index_single_below() {
        let e = make_engine_with_headings(make_headings(vec![(1, 100.0)]));
        assert_eq!(e.current_heading_index(50.0), Some(0));
    }

    #[test]
    fn current_heading_index_single_above() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0)]));
        assert_eq!(e.current_heading_index(50.0), Some(0));
    }

    #[test]
    fn current_heading_index_multiple_exact_match() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (1, 200.0)]));
        assert_eq!(e.current_heading_index(100.0), Some(1));
    }

    #[test]
    fn current_heading_index_multiple_between() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (1, 200.0)]));
        assert_eq!(e.current_heading_index(150.0), Some(1));
    }

    #[test]
    fn current_heading_index_at_last_heading() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (3, 200.0)]));
        assert_eq!(e.current_heading_index(200.0), Some(2));
    }

    #[test]
    fn current_heading_index_beyond_last() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0)]));
        assert_eq!(e.current_heading_index(500.0), Some(1));
    }

    #[test]
    fn scroll_to_heading_valid_index() {
        let mut e =
            make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (1, 200.0)]));
        e.content_height = 300.0;
        e.scroll_to_heading(1);
        assert_eq!(e.scroll_y, 100.0);
    }

    #[test]
    fn scroll_to_heading_first() {
        let mut e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0)]));
        e.content_height = 200.0;
        e.scroll_y = 150.0;
        e.scroll_to_heading(0);
        assert_eq!(e.scroll_y, 0.0);
    }

    #[test]
    fn scroll_to_heading_out_of_bounds() {
        let mut e = make_engine_with_headings(make_headings(vec![(1, 0.0)]));
        e.content_height = 100.0;
        e.scroll_y = 50.0;
        e.scroll_to_heading(99);
        assert_eq!(e.scroll_y, 50.0);
    }

    #[test]
    fn scroll_to_heading_clamps_to_content_height() {
        let mut e = make_engine_with_headings(make_headings(vec![(1, 500.0)]));
        e.content_height = 300.0;
        e.scroll_to_heading(0);
        assert_eq!(e.scroll_y, 300.0);
    }

    #[test]
    fn scroll_y_accessor() {
        let mut v = MarkdownView::new();
        v.engine.scroll_y = 42.0;
        assert_eq!(v.scroll_y(), 42.0);
    }

    #[test]
    fn headings_accessor_empty() {
        let v = MarkdownView::new();
        assert!(v.headings().is_empty());
    }

    #[test]
    fn headings_accessor_returns_slice() {
        let mut v = MarkdownView::new();
        v.engine.headings = make_headings(vec![(1, 0.0), (2, 50.0)]);
        assert_eq!(v.headings().len(), 2);
        assert_eq!(v.headings()[0].level, 1);
        assert_eq!(v.headings()[1].level, 2);
    }

    #[test]
    fn first_line_indent_settings_scale_and_invalidate_style_cache() {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut settings = ui::settings::Settings::new();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, 2.0);
        let disabled = MarkdownRenderSettings::from_metrics(&settings, &metrics).style(&theme);
        settings.markdown_first_line_indent = true;
        let enabled = MarkdownRenderSettings::from_metrics(&settings, &metrics).style(&theme);
        assert_eq!(disabled.paragraph_first_line_indent, 0.0);
        assert_eq!(enabled.paragraph_first_line_indent, 2.0 * metrics.font_size);
        assert_ne!(style_hash_quick(&disabled), style_hash_quick(&enabled));
        assert_eq!(
            MarkdownStyle::novel(&theme, metrics.font_size, metrics.line_height)
                .paragraph_first_line_indent,
            0.0
        );
    }

    #[test]
    fn markdown_render_settings_take_physical_metrics() {
        let settings = ui::settings::Settings::new();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, 2.0);
        let input = MarkdownRenderSettings::from_metrics(&settings, &metrics);
        assert_eq!(input.font_size, metrics.font_size);
        assert_eq!(input.line_height, metrics.line_height);
        assert_eq!(input.toc_max_depth, settings.toc_max_depth);
        assert_eq!(input.text_spacing_mode, settings.text_spacing_mode);
    }

    #[test]
    fn render_settings_control_style_and_toc_depth() {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = MarkdownRenderSettings {
            font_family: "PingFang SC",
            font_size: 36.0,
            line_height: 58.0,
            toc_max_depth: 5,
            markdown_first_line_indent: false,
            text_spacing_mode: ui::typography::TextSpacingMode::Natural,
        };
        let style = settings.style(&theme);
        assert_eq!(style.body_font_size, 36.0);
        assert_eq!(style.line_height, 58.0);

        let mut v = MarkdownView::new();
        v.set_source("# H1\n\n##### H5\n\n###### H6".into(), 1);
        let _ = v.render(&theme, 600.0, 400.0, 0.0, 0.0, settings, None);
        assert_eq!(v.headings().len(), 2);
        assert_eq!(v.headings()[1].level, 5);
    }
}

#[cfg(test)]
mod wysiwyg_tests {
    use super::*;
    use crate::projection::{CursorAffinity, ProjectionOwnerId, SourceAnchor};
    use ui::plugin::AugmentKind;
    use ui::plugin::MoveDirection;

    fn default_settings() -> MarkdownRenderSettings<'static> {
        MarkdownRenderSettings {
            font_family: ui::settings::platform_default_font_family(),
            font_size: 15.0,
            line_height: 24.0,
            toc_max_depth: 5,
            markdown_first_line_indent: false,
            text_spacing_mode: ui::typography::TextSpacingMode::Natural,
        }
    }

    #[test]
    fn novel_view_spacing_setting_changes_geometry_without_editing_source() {
        let source = "中文A,继续(说明)正文";
        let mut document = StubDoc::new(source);
        let mut view = NovelView::new();
        let theme = ui::theme::test_theme();
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("novel spacing test requires fonts");
        let render_width =
            |view: &mut NovelView, shaper: &mut shaping::Shaper, document: &StubDoc| {
                view.render(document, bounds, &theme, shaper, 1.0);
                view.engine.flat_lines()[0].shaped.as_ref().expect("novel line must shape").width
            };
        let natural_width = render_width(&mut view, &mut shaper, &document);
        assert!(view.handle_message(
            PluginMessage::SetTextSpacingMode(ui::typography::TextSpacingMode::Verbatim),
            &mut document,
        ));
        let verbatim_width = render_width(&mut view, &mut shaper, &document);
        assert!(natural_width > verbatim_width);
        assert_eq!(document.text, source);
        assert!(view.handle_message(
            PluginMessage::SetTextSpacingMode(ui::typography::TextSpacingMode::Natural),
            &mut document,
        ));
        let restored_width = render_width(&mut view, &mut shaper, &document);
        assert!((restored_width - natural_width).abs() < 0.01);
        assert_eq!(document.text, source);
    }

    #[test]
    fn notora_markdown_editor_keeps_the_first_h1_after_titles_become_independent() {
        let source = "# 页面标题\n\n正文内容\n\n# 正文章节";
        let document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);

        render_editor_once(&mut view, &document);

        let rendered_text =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        assert!(rendered_text.iter().any(|line| line.contains("页面标题")));
        assert!(rendered_text.iter().any(|line| line.contains("正文内容")));
        assert!(rendered_text.iter().any(|line| line.contains("正文章节")));
        assert!(
            view.engine()
                .projection_index()
                .visual_position_for_source(
                    source.find("正文内容").expect("body must exist"),
                    CursorAffinity::Downstream
                )
                .is_some(),
            "body source bytes must remain mapped while showing the independent H1"
        );
    }

    #[test]
    fn default_markdown_editor_keeps_the_first_h1_for_textora() {
        let source = "# 页面标题\n\n正文内容";
        let document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);

        render_editor_once(&mut view, &document);

        assert!(
            view.engine().flat_lines().iter().any(|line| line.text.contains("页面标题")),
            "the shared textora Markdown editor must retain its default rendering"
        );
    }

    /// Build a MarkdownView, set source, render, and return it.
    fn make_view(src: &str) -> MarkdownView {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = default_settings();
        let mut v = MarkdownView::new();
        v.set_source(src.into(), 1);
        v.engine_mut().set_edit_source(Some(src.into()));
        v.render(&theme, 800.0, 600.0, 0.0, 0.0, settings, None);
        v
    }

    fn make_projected_view(src: &str) -> MarkdownView {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = default_settings();
        let mut view = MarkdownView::new();
        view.set_source(src.into(), 1);
        view.engine_mut().set_edit_source(Some(src.into()));
        let mut shaper = shaping::Shaper::new().expect("WYSIWYG test view requires a shaper");
        view.render(&theme, 800.0, 600.0, 0.0, 0.0, settings, Some(&mut shaper));
        view
    }

    #[test]
    fn editable_empty_line_is_a_zero_grapheme_projection_line() {
        let source = "paragraph\n\n\nnext";
        let second_empty = "paragraph\n\n".len();
        let view = make_view(source);
        assert_eq!(
            view.engine().flat_lines().len(),
            3,
            "the editable empty paragraph must participate in the rendered-line collection"
        );
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(second_empty, CursorAffinity::Downstream)
            .expect("editable empty line must be projected");
        let line = &view.engine().projection_index().visual_lines()[position.flat_line_idx];

        assert_eq!(line.owner, ProjectionOwnerId::EmptyLine { source_byte: second_empty });
        assert_eq!(line.boundaries, vec![SourceAnchor::downstream(second_empty)]);
    }

    #[test]
    fn trailing_editable_empty_line_is_a_zero_grapheme_projection_line() {
        let source = "paragraph\n\n";
        let trailing_empty = source.len();
        let view = make_view(source);
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(trailing_empty, CursorAffinity::Downstream)
            .expect("trailing editable empty line must be projected");
        let line = &view.engine().projection_index().visual_lines()[position.flat_line_idx];

        assert_eq!(line.owner, ProjectionOwnerId::EmptyLine { source_byte: trailing_empty });
        assert_eq!(line.boundaries, vec![SourceAnchor::downstream(trailing_empty)]);
    }

    #[test]
    fn wysiwyg_can_scroll_to_trailing_empty_source_lines() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "paragraph\n\n\n";
        let viewport_height = 48.0;
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_with_offset_y(&mut view, &document, viewport_height, 0.0);

        let did_scroll = view.handle_message(
            PluginMessage::Scroll { delta: 100.0, viewport_h: viewport_height },
            &mut document,
        );

        assert!(did_scroll, "trailing empty source lines must extend the scrollable range");
        assert!(view.engine().scroll_y > 0.0, "scroll position must advance past the text block");
    }

    fn click_point_for_visible_text(
        engine: &PreviewEngine<MarkdownDoc>,
        needle: &str,
    ) -> (f32, f32) {
        let line = engine
            .flat_lines()
            .iter()
            .find(|line| line.text.contains(needle))
            .expect("needle must be rendered");
        let byte = line.text.find(needle).expect("needle must be in selected line");
        let grapheme = crate::grapheme_map::grapheme_index_at_byte(&line.text, byte);
        (line.rect.x + crate::layout::grapheme_x(line, grapheme), line.rect.y + line.rect.h * 0.5)
    }

    #[test]
    fn promotion_line_three_real_rect_hits_line_three_source_range() {
        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches and campaigns.\n> Style anchor: Apple Keynote and exhibitions.\n";
        let line_three = source.find("Applicable").expect("fixture must contain line three");
        let line_four = source.find("\n> Style").expect("fixture must contain line four") + 1;
        let view = make_projected_view(source);
        let (x, y) = click_point_for_visible_text(view.engine(), "Applicable");
        let hit = view.engine().hit_test_byte(x, y, 0.0, 0.0).expect("hit required");
        assert!((line_three..line_four).contains(&hit));
    }

    #[test]
    fn promotion_line_three_cursor_position_matches_its_visible_text_line() {
        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches and campaigns.\n> Style anchor: Apple Keynote and exhibitions.\n";
        let line_three = source.find("Applicable").expect("fixture must contain line three");
        let view = make_projected_view(source);
        let visible_line = view
            .engine()
            .flat_lines()
            .iter()
            .position(|line| line.text.contains("Applicable"))
            .expect("line three text must be visible");
        let position = view
            .engine()
            .cursor_visual_position_for_byte(line_three, CursorAffinity::Downstream)
            .expect("line three source byte must have a visual position");
        assert_eq!(position.flat_line_idx, visible_line);
        assert_eq!(position.grapheme_pos, 0);
    }

    #[test]
    fn promotion_blockquote_click_roundtrip_and_vertical_navigation_reach_line_three() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        const NARROW_VIEWPORT_WIDTH: f32 = 180.0;
        const WIDE_VIEWPORT_WIDTH: f32 = 800.0;
        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n\n- **Top-tier visual impact**: Large images and bold typography, using extreme visual tension to convey brand tone, artistic quality, or cultural depth — every page should leave the audience breathless\n";
        let line_three_start =
            source.find("Applicable scenarios").expect("fixture must contain real line three text");
        let line_four_start =
            source.find("\n> Style anchor").expect("fixture must contain real line four") + 1;
        let title_start = source.find("Promotion").expect("fixture must contain title text");

        for width in [NARROW_VIEWPORT_WIDTH, WIDE_VIEWPORT_WIDTH] {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (flat_idx, grapheme_idx) = view
                .engine()
                .flat_line_projection_boundaries()
                .iter()
                .find_map(|map| {
                    map.boundaries
                        .iter()
                        .position(|&byte| byte == line_three_start)
                        .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
                })
                .expect("initial layout must expose real line three source byte");
            let flat_line = &view.engine().flat_lines()[flat_idx];
            let click_x = flat_line.rect.x + crate::layout::grapheme_x(flat_line, grapheme_idx);
            let click_y = flat_line.rect.y + flat_line.rect.h * 0.5;

            let first_hit = view
                .engine()
                .hit_test_byte(click_x, click_y, 0.0, 0.0)
                .expect("first click on real line three must produce a source byte");
            view.handle_message(PluginMessage::SetCursorByte(first_hit), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("selected line three byte must have a cursor screen position");
            let second_hit = view
                .engine()
                .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
                .expect("cursor-position click must produce a source byte");

            assert_eq!(first_hit, line_three_start);
            assert_eq!(second_hit, line_three_start);

            view.handle_message(PluginMessage::SetCursorByte(title_start), &mut document);
            render_editor_narrow(&mut view, &document, width);
            let (title_x, _title_y, _title_width, _title_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("title must have a cursor screen position");
            let maximum_visual_moves = view.engine().flat_lines().len();
            let mut moved = title_start;
            if width == NARROW_VIEWPORT_WIDTH {
                let title_flat_idx = view
                    .engine()
                    .flat_line_projection_boundaries()
                    .iter()
                    .find(|map| map.boundaries.contains(&title_start))
                    .map(|map| map.flat_idx)
                    .expect("active heading must expose the title source byte");
                let heading_projections = view.engine().flat_line_projection_boundaries();
                let second_heading_segment = heading_projections
                    .get(title_flat_idx + 1)
                    .expect("narrow active heading must wrap to a second visual segment");
                let first_down = view
                    .engine()
                    .visual_move(title_start, MoveDirection::Down, Some(title_x))
                    .expect("first Down from title must produce a source byte");

                assert!(
                    second_heading_segment.boundaries.contains(&first_down),
                    "first Down must enter the active heading's second visual segment, got {first_down}",
                );
                assert!(
                    first_down < source.find('\n').expect("title fixture must end with a newline"),
                    "first Down must remain within the heading source line, got {first_down}",
                );
                moved = first_down;
            }
            for _ in 0..maximum_visual_moves {
                moved = view
                    .engine()
                    .visual_move(moved, MoveDirection::Down, Some(title_x))
                    .expect("Down from title must produce a source byte");
                if (line_three_start..line_four_start).contains(&moved) {
                    break;
                }
            }

            assert!(
                (line_three_start..line_four_start).contains(&moved),
                "Down from title must reach real line three source bytes {line_three_start}..{line_four_start} at width {width}, got {moved}",
            );
        }
    }

    #[test]
    fn promotion_em_dash_click_roundtrip_never_lands_inside_utf8_sequence() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n\n- **Top-tier visual impact**: Large images and bold typography, using extreme visual tension to convey brand tone, artistic quality, or cultural depth — every page should leave the audience breathless\n";
        let em_dash = source.find('—').expect("fixture must contain real line eight em dash");
        let after_em_dash = em_dash + '—'.len_utf8();

        for width in [180.0, 800.0] {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (flat_idx, grapheme_idx) = view
                .engine()
                .flat_line_projection_boundaries()
                .iter()
                .find_map(|map| {
                    map.boundaries
                        .iter()
                        .position(|&byte| byte == after_em_dash)
                        .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
                })
                .expect("initial layout must expose byte after real em dash");
            let flat_line = &view.engine().flat_lines()[flat_idx];
            let click_x = flat_line.rect.x + crate::layout::grapheme_x(flat_line, grapheme_idx);
            let click_y = flat_line.rect.y + flat_line.rect.h * 0.5;

            let first_hit = view
                .engine()
                .hit_test_byte(click_x, click_y, 0.0, 0.0)
                .expect("first click after real em dash must produce a source byte");
            view.handle_message(PluginMessage::SetCursorByte(first_hit), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("selected em-dash-adjacent byte must have a cursor screen position");
            let second_hit = view
                .engine()
                .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
                .expect("cursor-position click must produce a source byte");

            assert_eq!(first_hit, after_em_dash);
            assert_eq!(second_hit, after_em_dash);
            assert!(source.is_char_boundary(second_hit));
            assert_eq!(
                view.engine().visual_move(after_em_dash, MoveDirection::Left, None),
                Some(em_dash),
                "Left must cross the complete real em dash at width {width}",
            );
            assert_eq!(
                view.engine().visual_move(em_dash, MoveDirection::Right, None),
                Some(after_em_dash),
                "Right must cross the complete real em dash at width {width}",
            );
        }
    }

    fn styled_line_for_source_byte(
        block: &crate::layout::LaidOutBlock,
        source_byte: usize,
    ) -> Option<&crate::layout::LaidOutLine> {
        fn line_for_source_byte(
            lines: &[crate::layout::LaidOutLine],
            source_byte: usize,
        ) -> Option<&crate::layout::LaidOutLine> {
            lines.iter().find(|line| {
                line.source_projection.as_ref().is_some_and(|projection| {
                    projection.boundaries.iter().any(|anchor| anchor.byte == source_byte)
                })
            })
        }

        match &block.kind {
            crate::layout::LaidOutBlockKind::Text { lines }
            | crate::layout::LaidOutBlockKind::MetadataBlock { lines } => {
                line_for_source_byte(lines, source_byte)
            }
            crate::layout::LaidOutBlockKind::ListItem { lines, blocks, .. } => {
                line_for_source_byte(lines, source_byte).or_else(|| {
                    blocks.iter().find_map(|child| styled_line_for_source_byte(child, source_byte))
                })
            }
            crate::layout::LaidOutBlockKind::BlockQuote { blocks } => {
                blocks.iter().find_map(|child| styled_line_for_source_byte(child, source_byte))
            }
            crate::layout::LaidOutBlockKind::Table { header, rows, .. } => {
                header.iter().flatten().chain(rows.iter().flatten().flatten()).find(|line| {
                    line.source_projection.as_ref().is_some_and(|projection| {
                        projection.boundaries.iter().any(|anchor| anchor.byte == source_byte)
                    })
                })
            }
            crate::layout::LaidOutBlockKind::CodeBlock { .. }
            | crate::layout::LaidOutBlockKind::HorizontalRule
            | crate::layout::LaidOutBlockKind::Embedded { .. } => None,
        }
    }

    #[test]
    fn styled_prefix_cursor_and_hit_test_use_rendered_advance() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n\n- **Top-tier visual impact**: Large images and bold typography, using extreme visual tension to convey brand tone, artistic quality, or cultural depth — every page should leave the audience breathless\n";
        let colon_byte = source.find("**:").expect("fixture must contain bold closing marker") + 2;
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(colon_byte), &mut document);
        render_editor_once(&mut view, &document);

        let (flat_idx, grapheme_idx) = view
            .engine()
            .flat_line_projection_boundaries()
            .iter()
            .find_map(|map| {
                map.boundaries
                    .iter()
                    .position(|&byte| byte == colon_byte)
                    .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
            })
            .expect("materialized list line must expose the colon source byte");
        let flat_line = &view.engine().flat_lines()[flat_idx];
        let styled_line = view
            .engine()
            .lazy
            .as_ref()
            .and_then(|lazy| {
                lazy.laid_out
                    .iter()
                    .flatten()
                    .find_map(|block| styled_line_for_source_byte(block, colon_byte))
            })
            .expect("precise layout must retain the materialized list line");
        let bold = styled_line
            .style_segments
            .iter()
            .find(|segment| matches!(segment.style, crate::builder::InlineStyle::Bold))
            .expect("precise layout must retain the bold prefix segment");
        let closing_marker = styled_line
            .style_segments
            .iter()
            .find(|segment| {
                matches!(segment.style, crate::builder::InlineStyle::SourceMarker)
                    && segment.start == bold.start + bold.len
            })
            .expect("precise layout must retain the bold closing marker");
        assert!(
            (closing_marker.x_offset - (bold.x_offset + bold.width)).abs() < 0.01,
            "bold closing marker must start at the styled visual boundary"
        );
        let expected_x = flat_line.rect.x + closing_marker.x_offset + closing_marker.width;
        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("colon cursor must resolve to the materialized list line");
        assert!(
            (cursor_x - expected_x).abs() < 0.01,
            "cursor x {cursor_x} must match the bold visual boundary {expected_x}"
        );

        let hit = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
            .expect("click at the styled boundary must produce a source byte");
        assert_eq!(hit, colon_byte);
        assert_eq!(
            crate::layout::grapheme_x(flat_line, grapheme_idx) + flat_line.rect.x,
            cursor_x,
            "cursor must use the same visual grapheme geometry as hit-testing"
        );
    }

    #[test]
    fn active_heading_marker_cursor_and_hit_test_use_heading_advance() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# Title";
        let marker_end = "#".len();
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(marker_end), &mut document);
        render_editor_once(&mut view, &document);

        let (flat_idx, grapheme_idx) = view
            .engine()
            .flat_line_projection_boundaries()
            .iter()
            .find_map(|map| {
                map.boundaries
                    .iter()
                    .position(|&byte| byte == marker_end)
                    .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
            })
            .expect("active heading marker must expose its source boundary");
        let flat_line = &view.engine().flat_lines()[flat_idx];
        let styled_line = view
            .engine()
            .lazy
            .as_ref()
            .and_then(|lazy| {
                lazy.laid_out
                    .iter()
                    .flatten()
                    .find_map(|block| styled_line_for_source_byte(block, marker_end))
            })
            .expect("precise layout must retain the active heading line");
        let marker = styled_line
            .style_segments
            .iter()
            .find(|segment| matches!(segment.style, crate::builder::InlineStyle::SourceMarker))
            .expect("active heading marker must retain a source-marker segment");

        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let style = default_settings().style(&theme);
        shaper.set_font_size(styled_line.font_size);
        shaper.set_font_weight(styled_line.font_weight);
        shaper.set_font_family(style.body_font_family.first().map(String::as_str));
        let expected_marker_width = shaper
            .shape("# ")
            .expect("heading marker should shape with its line font weight")
            .width;
        let expected_x = flat_line.rect.x + shaper.shape("#").expect("prefix cursor advance").width;

        assert!(
            (marker.width - expected_marker_width).abs() < 0.01,
            "marker width {} must match the heading weight advance {expected_marker_width}",
            marker.width
        );

        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("heading marker boundary must have a cursor position");
        assert!(
            (cursor_x - expected_x).abs() < 0.01,
            "cursor x {cursor_x} must match the heading marker advance {expected_x}"
        );

        let hit = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
            .expect("click at the heading marker boundary must produce a source byte");
        assert_eq!(hit, marker_end);
        assert_eq!(
            crate::layout::grapheme_x(flat_line, grapheme_idx) + flat_line.rect.x,
            cursor_x,
            "cursor and hit-testing must share the heading marker geometry"
        );
    }

    // ── augment_edit ─────────────────────────────────────────────────────

    #[test]
    fn augment_edit_bullet_list_inserts_marker() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n- "));
    }

    #[test]
    fn augment_edit_nested_bullet_list_preserves_indent() {
        let source = "- parent\n  - child";
        let mut v = make_view(source);
        let cursor_byte = source.len();
        v.engine_mut().handle_set_cursor_byte(cursor_byte);
        let aug = v.engine().augment_edit(cursor_byte, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n  - "));
    }

    #[test]
    fn augment_edit_ordered_list_increments_number() {
        let mut v = make_view("1. item");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n2. "));
    }

    #[test]
    fn augment_edit_task_list_inserts_checkbox() {
        let mut v = make_view("- [ ] task");
        v.engine_mut().handle_set_cursor_byte(7);
        let aug = v.engine().augment_edit(7, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n- [ ] "));
    }

    #[test]
    fn augment_edit_backspace_list_item_marker() {
        let mut v = make_view("- hello");
        v.engine_mut().handle_set_cursor_byte(2); // "- |hello"
        let aug = v.engine().augment_edit(2, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..2));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_ordered_list_item_marker() {
        let mut v = make_view("12. hello");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..4));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_blockquote_marker() {
        let mut v = make_view("> hello");
        v.engine_mut().handle_set_cursor_byte(2);
        let aug = v.engine().augment_edit(2, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..2));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_heading_marker() {
        let mut v = make_view("### hello");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..4));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_inside_heading_marker_returns_none() {
        let mut v = make_view("### hello");
        v.engine_mut().handle_set_cursor_byte(2); // "##|# hello"
        let aug = v.engine().augment_edit(2, AugmentKind::Backspace);
        assert!(aug.is_none());
    }

    #[test]
    fn augment_edit_backspace_indented_list_item_marker() {
        let mut v = make_view("  - hello");
        v.engine_mut().handle_set_cursor_byte(4); // "  - |hello"
        let aug = v.engine().augment_edit(4, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..4));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_paragraph_middle_inserts_paragraph_break() {
        let mut v = make_view("hello world");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.replace_range, None);
        assert_eq!(aug.cursor_byte_after, 6);
    }

    #[test]
    fn augment_edit_paragraph_before_timestamp_inserts_visible_break() {
        let source = "主要使用外部模型进行写作辅助。29:49";
        let timestamp_start = source.find("29:49").expect("fixture should contain timestamp");
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(timestamp_start);

        let aug = view.engine.augment_edit(timestamp_start, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.replace_range, None);
        assert_eq!(aug.cursor_byte_after, timestamp_start + 2);
    }

    #[test]
    fn augment_edit_heading_middle_splits_at_cursor_without_empty_line() {
        let source = "# hello world";
        let cursor_byte = 4;
        let mut v = make_view(source);
        v.engine_mut().handle_set_cursor_byte(cursor_byte); // "# he|llo world"

        let aug = v.engine().augment_edit(cursor_byte, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.replace_range, Some(cursor_byte..cursor_byte));
        assert_eq!(aug.cursor_byte_after, cursor_byte + 1);

        let mut edited_source = source.to_owned();
        let replace_range =
            aug.replace_range.expect("heading interior Enter must edit at the current cursor");
        edited_source.replace_range(
            replace_range,
            aug.insert_text
                .as_deref()
                .expect("heading interior Enter must insert one logical newline"),
        );
        assert_eq!(edited_source, "# he\nllo world");
    }

    #[test]
    fn augment_edit_blockquote_middle_splits_into_two_paragraphs() {
        let mut v = make_view("> hello world");
        v.engine_mut().handle_set_cursor_byte(5); // "> hel|lo world"
        let aug = v.engine().augment_edit(5, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n>\n> "));
        assert_eq!(aug.replace_range, None);
        assert_eq!(aug.cursor_byte_after, 10); // 5 + 5
    }

    #[test]
    fn augment_edit_paragraph_end_inserts_paragraph_break() {
        let source = "hello world";
        let mut v = make_view(source);
        v.engine_mut().handle_set_cursor_byte(source.len());
        let aug = v.engine().augment_edit(source.len(), AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, source.len() + 2);
    }

    #[test]
    fn augment_edit_paragraph_end_before_existing_newline_inserts_paragraph_break() {
        let source = "hello world\n";
        let paragraph_end = "hello world".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(paragraph_end);

        let aug = view.engine.augment_edit(paragraph_end, AugmentKind::Enter).unwrap();
        // Preserve the existing EOF paragraph while creating one more.
        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, paragraph_end + 2);
    }

    #[test]
    fn augment_edit_paragraph_end_before_next_block_inserts_single_editable_line() {
        let source = "hello world\n\n# Next";
        let paragraph_end = "hello world".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(paragraph_end);

        let aug = view.engine.augment_edit(paragraph_end, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, paragraph_end + 2);
    }

    #[test]
    fn augment_edit_heading_end_before_next_block_places_cursor_on_visible_empty_line() {
        let source = "# Heading\n\nparagraph";
        let heading_end = "# Heading".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(heading_end);

        let aug = view
            .engine
            .augment_edit(heading_end, AugmentKind::Enter)
            .expect("heading-end Enter must create an editable paragraph");

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, heading_end + 2);
    }

    #[test]
    fn heading_enter_before_next_block_cursor_rect_is_visible() {
        let source = "# Heading\n\nparagraph";
        let heading_end = "# Heading".len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(heading_end);

        let aug = view.engine.augment_edit(heading_end, AugmentKind::Enter).unwrap();
        if let Some(insert_text) = aug.insert_text.as_deref() {
            doc.text.replace_range(heading_end..heading_end, insert_text);
        }
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        assert!(
            view.engine().cursor_screen_pos().is_some(),
            "cursor after heading Enter should land on a visible editable empty line"
        );
    }

    #[test]
    fn heading_enter_before_adjacent_block_creates_visible_empty_line() {
        let source = "# Heading\nparagraph";
        let heading_end = "# Heading".len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(heading_end);

        let aug = view.engine.augment_edit(heading_end, AugmentKind::Enter).unwrap();
        doc.text.replace_range(
            heading_end..heading_end,
            aug.insert_text.as_deref().expect("heading Enter should insert a block break"),
        );
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        assert_eq!(doc.text, "# Heading\n\n\nparagraph");
        assert!(
            view.engine().cursor_screen_pos().is_some(),
            "cursor should land on the editable empty line, not the next block"
        );
    }

    #[test]
    fn augment_edit_softbreak_boundary_before_newline_inserts_source_newline() {
        let source = "first line\nsecond line";
        let softbreak_byte = "first line".len();
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(softbreak_byte);

        let aug = view.engine.augment_edit(softbreak_byte, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, softbreak_byte + 2);

        let mut edited_source = source.to_owned();
        edited_source.replace_range(
            softbreak_byte..softbreak_byte,
            aug.insert_text
                .as_deref()
                .expect("paragraph Enter before a soft break must insert a block break"),
        );
        assert_eq!(edited_source, "first line\n\n\nsecond line");
    }

    #[test]
    fn augment_edit_softbreak_boundary_after_newline_inserts_source_newline() {
        let source = "first line\nsecond line";
        let second_line_start = "first line\n".len();
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(second_line_start);

        let aug = view.engine.augment_edit(second_line_start, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, second_line_start + 1);
    }

    #[test]
    fn augment_edit_paragraph_end_uses_current_source_before_next_render() {
        let source = "fresh paragraph";
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(source.len());

        let aug = view.engine.augment_edit(source.len(), AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, source.len() + 2);
    }

    #[test]
    fn augment_edit_backspace_returns_none() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(4);
        assert!(v.engine().augment_edit(4, AugmentKind::Backspace).is_none());
    }

    #[test]
    fn augment_edit_backspace_on_empty_line_between_blocks_keeps_cursor_visible() {
        let source = "first\n\nsecond";
        let first_end = "first".len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(first_end);

        let enter_aug = view
            .engine
            .augment_edit(first_end, AugmentKind::Enter)
            .expect("paragraph-end Enter should create a trailing empty paragraph");
        doc.text.replace_range(
            first_end..first_end,
            enter_aug.insert_text.as_deref().expect("Enter augmentation should insert text"),
        );
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(enter_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        let backspace_aug = view
            .engine
            .augment_edit(enter_aug.cursor_byte_after, AugmentKind::Backspace)
            .expect("backspace on editable empty line should be handled by markdown augmenter");
        let replace_range = backspace_aug
            .replace_range
            .clone()
            .expect("empty-line backspace should delete one newline");
        doc.text.replace_range(replace_range, backspace_aug.insert_text.as_deref().unwrap_or(""));
        view.set_source(doc.text.clone(), 3);
        view.engine.handle_set_cursor_byte(backspace_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        assert_eq!(doc.text, source, "backspace should restore the original block separator");
        assert_eq!(backspace_aug.cursor_byte_after, first_end);
        assert!(
            view.engine.cursor_screen_pos().is_some(),
            "cursor should remain visible after deleting the editable empty line"
        );
    }

    #[test]
    fn augment_edit_backspace_on_trailing_empty_paragraph_removes_blank_line() {
        let source = "first";
        let first_end = source.len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(first_end);

        let enter_aug = view
            .engine
            .augment_edit(first_end, AugmentKind::Enter)
            .expect("paragraph-end Enter should create a trailing empty paragraph");
        doc.text.replace_range(
            first_end..first_end,
            enter_aug.insert_text.as_deref().expect("Enter augmentation should insert text"),
        );
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(enter_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        let insert_aug = view
            .engine
            .augment_edit(enter_aug.cursor_byte_after, AugmentKind::InsertText(String::from("x")))
            .expect("typing into trailing empty paragraph should normalize markdown newlines");
        let insert_range = insert_aug
            .replace_range
            .clone()
            .unwrap_or(enter_aug.cursor_byte_after..enter_aug.cursor_byte_after);
        doc.text.replace_range(insert_range, insert_aug.insert_text.as_deref().unwrap_or(""));
        view.set_source(doc.text.clone(), 3);
        view.engine.handle_set_cursor_byte(insert_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        doc.text.replace_range(insert_aug.cursor_byte_after - 1..insert_aug.cursor_byte_after, "");
        view.set_source(doc.text.clone(), 4);
        view.engine.handle_set_cursor_byte(insert_aug.cursor_byte_after - 1);
        render_editor_once(&mut view, &doc);

        let backspace_aug = view
            .engine
            .augment_edit(insert_aug.cursor_byte_after - 1, AugmentKind::Backspace)
            .expect("backspace on trailing empty paragraph should be handled");
        let replace_range = backspace_aug
            .replace_range
            .clone()
            .expect("trailing empty paragraph backspace should delete source newlines");
        doc.text.replace_range(replace_range, backspace_aug.insert_text.as_deref().unwrap_or(""));

        assert_eq!(doc.text, source, "backspace should remove the trailing empty paragraph");
        assert_eq!(backspace_aug.cursor_byte_after, first_end);
    }

    #[test]
    fn augment_edit_insert_text_in_empty_separator_line() {
        use ui::plugin::AugmentKind;
        let v = make_view("para1\n\npara2");
        let aug = v.engine().augment_edit(6, AugmentKind::InsertText(String::from("A"))).unwrap();

        let mut source = String::from("para1\n\npara2");
        source.replace_range(
            aug.replace_range.expect("explicit insertion range"),
            aug.insert_text.as_deref().expect("paragraph insertion"),
        );
        assert_eq!(source, "para1\n\nA\n\npara2");
        assert_eq!(aug.cursor_byte_after, 5 + 3);
    }

    #[test]
    fn augment_edit_insert_text_in_triple_empty_separator_line() {
        use ui::plugin::AugmentKind;
        let v = make_view("para1\n\n\npara2");
        let aug = v.engine().augment_edit(7, AugmentKind::InsertText(String::from("A"))).unwrap();

        let mut source = String::from("para1\n\n\npara2");
        source.replace_range(
            aug.replace_range.expect("explicit insertion range"),
            aug.insert_text.as_deref().expect("paragraph insertion"),
        );
        assert_eq!(source, "para1\n\nA\n\npara2");
        assert_eq!(aug.cursor_byte_after, 5 + 3);
    }

    #[test]
    fn augment_edit_tab_returns_none() {
        let mut plain_view = make_view("plain");
        plain_view.engine_mut().handle_set_cursor_byte(2);
        assert!(plain_view.engine().augment_edit(2, AugmentKind::Tab).is_none());

        let mut list_view = make_view("- item");
        list_view.engine_mut().handle_set_cursor_byte(4);
        let list_aug = list_view
            .engine()
            .augment_edit(4, AugmentKind::Tab)
            .expect("first list item Tab must return consuming augmentation");
        assert_eq!(list_aug.insert_text.as_deref(), Some(""));
        assert_eq!(list_aug.replace_range, None);
        assert_eq!(list_aug.cursor_byte_after, 4);
    }

    #[test]
    fn augment_edit_cursor_after_insert_text() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(3);
        let aug = v.engine().augment_edit(3, AugmentKind::Enter).unwrap();
        assert_eq!(aug.cursor_byte_after, 3 + "\n- ".len());
    }

    // ── Phase-0 guardrails (2026-07-06 plan) ─────────────────────────────
    // These pin down invariants the L1/L2/L3 refactor must preserve.

    /// L3 guardrail: setting preedit MUST NOT move the source-of-truth cursor.
    /// If this ever fails, IME composition has started overwriting
    /// `edit_ctx.cursor_byte`, which breaks arrow-key navigation during IME.
    #[test]
    fn set_preedit_does_not_move_edit_ctx_cursor() {
        let mut v = make_view("hello world");
        v.engine_mut().handle_set_cursor_byte(5);
        assert_eq!(
            v.engine().edit_ctx.as_ref().map(|c| c.cursor_byte),
            Some(5),
            "precondition: cursor should be at 5"
        );

        v.engine_mut().set_preedit_text("ab".into(), Some((1, 1)));
        assert_eq!(
            v.engine().edit_ctx.as_ref().map(|c| c.cursor_byte),
            Some(5),
            "SetPreedit must not overwrite cursor_byte (source of truth stays on DocumentView)"
        );

        v.engine_mut().set_preedit_text(String::new(), None);
        assert_eq!(
            v.engine().edit_ctx.as_ref().map(|c| c.cursor_byte),
            Some(5),
            "clearing preedit must also leave cursor_byte untouched"
        );
    }

    /// L3 guardrail: setting preedit without a prior cursor MUST NOT fabricate
    /// a phantom cursor at byte 0. Before 2026-07-06, `set_preedit_text` would
    /// synthesize an edit_ctx with cursor_byte=0 whenever no cursor was set,
    /// then visual_move/augment_edit would read this bogus state.
    #[test]
    fn set_preedit_without_prior_cursor_does_not_fabricate_zero_cursor() {
        // Fresh view: no handle_set_cursor_byte issued yet.
        let mut v = make_view("hello world");
        assert!(v.engine().edit_ctx.is_none(), "precondition: no cursor set");

        v.engine_mut().set_preedit_text("中".into(), Some((3, 3)));
        assert!(
            v.engine().edit_ctx.is_none(),
            "preedit with no prior cursor must be dropped, not stored with cursor_byte=0"
        );
    }

    #[test]
    fn hidden_separator_has_no_independent_caret_geometry() {
        let source = "para1\n\npara2";
        let view = make_view(source);
        let lazy = view.engine().lazy.as_ref().expect("layout should be built");
        let separator = source_line_at_byte(source, "para1\n".len())
            .expect("fixture has a separator source line");
        assert_eq!(
            view.engine().empty_source_line_role(separator, lazy, source),
            EmptySourceLineRole::HiddenBlockSeparator
        );
        assert!(view.engine().empty_source_line_metrics(separator, lazy, source).is_none());
    }

    #[test]
    fn visual_move_left_at_byte_zero() {
        let v = make_view("hello");
        assert_eq!(v.engine().visual_move(0, MoveDirection::Left, None), Some(0));
    }

    #[test]
    fn visual_move_right_at_end_returns_current() {
        let v = make_view("ab");
        // Block text is "ab" at source bytes 0..2; last char is byte 2.
        let result = v.engine().visual_move(2, MoveDirection::Right, None);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn visual_move_right_advances_one_utf8_char() {
        let v = make_view("- item");
        // Block text "item" starts at source byte 2; byte 2 → byte 3.
        let result = v.engine().visual_move(2, MoveDirection::Right, None);
        assert_eq!(result, Some(3));
    }

    #[test]
    fn visual_move_left_retreats_one_utf8_char() {
        let v = make_view("- item");
        // byte 3 (second char of "item") → byte 2 (first char of "item").
        let result = v.engine().visual_move(3, MoveDirection::Left, None);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn visual_move_right_skips_combining_mark() {
        let source = "**e\u{0301}x**";
        let mut v = make_view(source);
        v.engine_mut().handle_set_cursor_byte(2);

        let result = v.engine().visual_move(2, MoveDirection::Right, None);

        // Should skip the entire é grapheme (3 bytes) and land at byte 5 (x).
        assert_eq!(result, Some(2 + "e\u{0301}".len()));
    }

    #[test]
    fn visual_move_right_skips_zwj_emoji_cluster() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let source = format!("**{emoji}x**");
        let mut v = make_view(&source);
        v.engine_mut().handle_set_cursor_byte(2);

        let result = v.engine().visual_move(2, MoveDirection::Right, None);

        // Should skip the entire ZWJ emoji cluster and land at byte 2 + emoji.len().
        assert_eq!(result, Some(2 + emoji.len()));
    }

    #[test]
    fn promotion_blockquote_left_right_traversal_is_ordered_and_terminates() {
        let source = "> first physical line\n> second physical line";
        let view = make_view(source);
        let mut byte = source.find("second").expect("fixture must contain second");
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..=source.len() * 2 {
            assert!(visited.insert(byte), "horizontal navigation must not loop at byte {byte}");
            let next =
                view.engine().visual_move(byte, MoveDirection::Left, None).expect("move left");
            if next == 0 {
                return;
            }
            byte = next;
        }
        panic!("left traversal did not reach document start");
    }

    #[test]
    fn virtual_preedit_roundtrip_keeps_committed_source_byte() {
        let source = "ab";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(1);
        view.engine.set_preedit_text("中文".to_string(), Some((3, 3)));
        render_editor_once(&mut view, &doc);

        let _index = view.engine.projection_index();
        let rect = view.engine().cursor_screen_pos().expect("preedit cursor rect");
        let hit = view.engine().hit_test_byte(rect.0, rect.1 + rect.3 * 0.5, 0.0, 0.0);
        assert_eq!(hit, Some(1));
    }

    // ── cursor_in_span (PreviewEngine wrapper) ───────────────────────────

    #[test]
    fn cursor_in_span_inside_returns_true() {
        let mut v = make_view("**bold**");
        // Bold marker "**" is 2 bytes; text "bold" is source bytes 2..6.
        let span = crate::builder::StyleSpan {
            start: 0,
            len: 4,
            style: crate::builder::InlineStyle::Bold,
            source_range: std::ops::Range { start: 2, end: 6 },
        };
        v.engine_mut().handle_set_cursor_byte(3);
        assert!(v.engine().cursor_in_span(&span));
    }

    #[test]
    fn cursor_in_span_outside_returns_false() {
        let mut v = make_view("**bold** text");
        let span = crate::builder::StyleSpan {
            start: 0,
            len: 4,
            style: crate::builder::InlineStyle::Bold,
            source_range: std::ops::Range { start: 2, end: 6 },
        };
        v.engine_mut().handle_set_cursor_byte(8);
        assert!(!v.engine().cursor_in_span(&span));
    }

    #[test]
    fn cursor_in_span_no_edit_ctx_returns_false() {
        let v = make_view("**bold**");
        let span = crate::builder::StyleSpan {
            start: 0,
            len: 4,
            style: crate::builder::InlineStyle::Bold,
            source_range: std::ops::Range { start: 2, end: 6 },
        };
        assert!(!v.engine().cursor_in_span(&span));
    }

    // ── byte mapping: expanded span roundtrip ──────────────────────────

    #[test]
    fn hit_test_byte_roundtrip_at_expanded_cursor_position() {
        // "**bold**" → bold span is expanded when cursor is inside.
        // Source: "**bold**" = 8 bytes; bold text "bold" is at bytes 2..6.
        let mut v = make_view("**bold**");
        v.engine_mut().handle_set_cursor_byte(3); // cursor inside "bold"

        let pos = v.engine().cursor_screen_pos().unwrap();
        let (cx, cy, _cw, ch) = pos;
        // hit_test_byte roundtrip: screen pos → byte should map back to 3.
        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        assert_eq!(result, Some(3), "roundtrip at expanded cursor should return same byte");
    }

    #[test]
    fn visual_move_within_single_line_expanded_span() {
        // When cursor is inside a bold span, the span is expanded to show markers.
        // visual_move Right should advance one source byte within the expanded text.
        let mut v = make_view("**bold**");
        v.engine_mut().handle_set_cursor_byte(3); // inside "bold"

        let result = v.engine().visual_move(3, MoveDirection::Right, None);
        assert_eq!(result, Some(4), "Right inside expanded span should advance one byte");
    }

    #[test]
    fn hit_test_byte_at_end_of_single_line_expanded_span() {
        // Cursor at byte 5 (last char of "bold" in "**bold**")
        let mut v = make_view("**bold** text");
        v.engine_mut().handle_set_cursor_byte(5);

        let pos = v.engine().cursor_screen_pos().unwrap();
        let (cx, cy, _cw, ch) = pos;
        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        // Should map back to a byte within the bold text range
        assert!(result.is_some(), "hit_test at expanded span tail should return a byte");
    }

    #[test]
    fn hit_test_byte_roundtrip_second_plain_paragraph() {
        // Regression test for P1: plain-text source maps starting from 0.
        // Two plain paragraphs: "first" at bytes 0..5, "second" at bytes 7..13.
        // Cursor at byte 7 (start of "second") must roundtrip correctly.
        let mut v = make_view("first\n\nsecond");
        v.engine_mut().handle_set_cursor_byte(7);

        let pos = v
            .engine()
            .cursor_screen_pos()
            .expect("cursor at byte 7 (start of second paragraph) should resolve to screen pos");
        let (cx, cy, _cw, ch) = pos;

        // Cursor on second paragraph: y should be > first line height
        assert!(cy > 20.0, "second paragraph cursor y ({cy}) should be below first line");

        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        assert_eq!(result, Some(7), "roundtrip should return byte 7 for second plain paragraph");
    }

    #[test]
    fn extra_empty_source_line_reserves_vertical_space_before_next_paragraph() {
        let baseline = make_editor_view_with_cursor("first\n\nsecond", 7);
        let expanded = make_editor_view_with_cursor("first\n\n\nsecond", 7);

        let baseline_second_y = flat_line_y_for_source_byte(&baseline, 7)
            .expect("baseline second paragraph should be laid out");
        let expanded_second_y = flat_line_y_for_source_byte(&expanded, 8)
            .expect("expanded second paragraph should be laid out");

        assert!(
            expanded_second_y >= baseline_second_y + baseline.engine.base_line_height - 1.0,
            "extra empty source line should push following paragraph down by one line: \
             baseline={baseline_second_y}, expanded={expanded_second_y}"
        );
    }

    #[test]
    fn editable_empty_paragraph_between_paragraphs_keeps_both_boundaries() {
        // 空段落拥有独立行高，且前后各保留一份 paragraph_spacing。
        let source = "first\n\n\nsecond";
        let second_start = source.find("second").expect("fixture should contain second paragraph");
        let view = make_editor_view_with_cursor(source, second_start);
        let first_line = view.engine().flat_lines().first().expect("first paragraph should render");
        let second_y = flat_line_y_for_source_byte(&view, second_start)
            .expect("second paragraph should render");
        let actual_gap = second_y - (first_line.rect.y + first_line.rect.h);
        let expected_gap = view.engine().paragraph_spacing * 2.0 + view.engine().base_line_height;

        assert!(
            (actual_gap - expected_gap).abs() < 1.0,
            "an editable blank paragraph must keep spacing on both boundaries: \
             actual={actual_gap}, expected={expected_gap}"
        );
    }

    #[test]
    fn editable_empty_paragraphs_keep_spacing_on_both_sides() {
        for (source, empty_paragraph_starts) in [
            ("first\n\n\nsecond", vec!["first\n\n".len()]),
            ("first\n\n\n\nsecond", vec!["first\n\n".len(), "first\n\n\n".len()]),
        ] {
            let mut doc = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.to_owned(), 1);
            render_editor_once(&mut view, &doc);

            let first_line = view
                .engine()
                .flat_lines()
                .iter()
                .find(|line| line.text == "first")
                .expect("first paragraph should render");
            let mut previous_bottom = first_line.rect.y + first_line.rect.h;
            let expected_spacing = view.engine().paragraph_spacing;
            let expected_line_height = view.engine().base_line_height;

            for empty_paragraph_start in empty_paragraph_starts {
                view.handle_message(PluginMessage::SetCursorByte(empty_paragraph_start), &mut doc);
                render_editor_once(&mut view, &doc);

                let (_cursor_x, cursor_y, _cursor_width, cursor_height) = view
                    .engine()
                    .cursor_screen_pos()
                    .expect("editable empty paragraph should expose a cursor rect");
                let cursor_line_top = cursor_y + view.engine().scroll_y
                    - cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);
                let actual_spacing = cursor_line_top - previous_bottom;

                assert!(
                    (actual_spacing - expected_spacing).abs() < 1.0,
                    "empty paragraph at byte {empty_paragraph_start} should keep spacing above: \
                     actual={actual_spacing}, expected={expected_spacing}, source={source:?}"
                );
                previous_bottom = cursor_line_top + expected_line_height;
            }

            let second_line = view
                .engine()
                .flat_lines()
                .iter()
                .find(|line| line.text == "second")
                .expect("second paragraph should render");
            let actual_spacing = second_line.rect.y - previous_bottom;
            assert!(
                (actual_spacing - expected_spacing).abs() < 1.0,
                "last empty paragraph should keep spacing below: actual={actual_spacing}, \
                 expected={expected_spacing}, source={source:?}"
            );
        }
    }

    #[test]
    fn typing_into_editable_empty_paragraph_keeps_vertical_geometry_stable() {
        let empty_source = "first\n\n\nsecond";
        let empty_paragraph_start = "first\n\n".len();
        let empty_view = make_editor_view_with_cursor(empty_source, empty_paragraph_start);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) = empty_view
            .engine()
            .cursor_screen_pos()
            .expect("editable empty paragraph should expose a cursor rect");
        let empty_line_top = cursor_y + empty_view.engine().scroll_y
            - cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);
        let empty_second_y = empty_view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "second")
            .expect("second paragraph should render")
            .rect
            .y;

        let filled_source = "first\n\nx\n\nsecond";
        let filled_view = make_editor_view_with_cursor(filled_source, "first\n\n".len());
        let filled_line_y = filled_view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "x")
            .expect("typed paragraph should render")
            .rect
            .y;
        let filled_second_y = filled_view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "second")
            .expect("second paragraph should render after typing")
            .rect
            .y;

        assert!(
            (empty_line_top - filled_line_y).abs() < 1.0,
            "typing should not move the active paragraph: empty={empty_line_top}, \
             filled={filled_line_y}"
        );
        assert!(
            (empty_second_y - filled_second_y).abs() < 1.0,
            "typing should not reflow the following paragraph vertically: empty={empty_second_y}, \
             filled={filled_second_y}"
        );
    }

    #[test]
    fn cursor_leaving_offscreen_heading_clears_active_marker_when_scrolled_back() {
        use ui::plugin::PluginMessage;

        let filler =
            (0..40).map(|i| format!("filler paragraph {i}")).collect::<Vec<_>>().join("\n\n");
        let source = format!("# Title\n\n{filler}\n\nlast paragraph");
        let last_byte = source.find("last paragraph").expect("fixture has a last paragraph");
        let mut doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(1), &mut doc);
        render_editor_once(&mut view, &doc);

        fn heading_text(view: &MarkdownEditorView) -> String {
            view.engine()
                .flat_lines()
                .iter()
                .find(|line| line.text.contains("Title"))
                .map(|line| line.text.clone())
                .expect("heading flat line must exist")
        }
        assert_eq!(heading_text(&view), "# Title", "cursor inside heading shows the marker");

        // 滚到文档底部，再把光标移到最后一段：标题块已在渲染窗口外。
        let bottom_scroll = (view.engine.content_height - 600.0).max(0.0);
        assert!(bottom_scroll > 0.0, "fixture document must exceed one viewport");
        view.engine.scroll_y = bottom_scroll;
        view.handle_message(PluginMessage::SetCursorByte(last_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        // 滚回顶部：标题必须按非 active 重排，不得残留 `# ` 标记。
        view.engine.scroll_y = 0.0;
        render_editor_once(&mut view, &doc);
        assert_eq!(
            heading_text(&view),
            "Title",
            "cursor left the offscreen heading; its active marker must not persist"
        );
    }

    #[test]
    fn editable_empty_paragraph_before_list_keeps_trailing_spacing() {
        let baseline_source = "first\n\n- item";
        let expanded_source = "first\n\n\n- item";
        let baseline_list_start =
            baseline_source.find("- item").expect("fixture should contain list");
        let expanded_list_start =
            expanded_source.find("- item").expect("fixture should contain list");
        let baseline = make_editor_view_with_cursor(baseline_source, baseline_list_start);
        let expanded = make_editor_view_with_cursor(expanded_source, expanded_list_start);
        let baseline_y = flat_line_y_for_source_byte(&baseline, baseline_list_start)
            .expect("list should render");
        let expanded_y = flat_line_y_for_source_byte(&expanded, expanded_list_start)
            .expect("list should render");

        assert!(
            ((expanded_y - baseline_y)
                - baseline.engine().base_line_height
                - baseline.engine().paragraph_spacing)
                .abs()
                < 1.0,
            "an editable empty paragraph before a list should add its line height and trailing \
             spacing: \
             baseline={baseline_y}, expanded={expanded_y}"
        );
    }

    #[test]
    fn source_update_with_new_empty_line_reflows_next_paragraph_immediately() {
        let mut view = MarkdownEditorView::new();
        let mut doc = StubDoc::new("first\n\nsecond");
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let baseline_second_y = flat_line_y_for_source_byte(&view, 7)
            .expect("baseline second paragraph should be laid out");

        doc.text = String::from("first\n\n\nsecond");
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(7);
        render_editor_once(&mut view, &doc);

        let expanded_second_y = flat_line_y_for_source_byte(&view, 8)
            .expect("expanded second paragraph should be laid out after source update");

        assert!(
            expanded_second_y >= baseline_second_y + view.engine.base_line_height - 1.0,
            "source update should rebuild layout in the next render, not wait for cursor blink: \
             baseline={baseline_second_y}, expanded={expanded_second_y}"
        );
    }

    #[test]
    fn editable_empty_source_line_between_paragraphs_uses_full_line_height() {
        let source = "hello\n\n\nnext";
        let cursor_byte = "hello\n\n".len();
        let next_byte = "hello\n\n\n".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);

        let source_line =
            source_line_at_byte(source, cursor_byte).expect("cursor should be on an empty line");
        let lazy = view.engine.lazy.as_ref().expect("layout should be built");
        let (_x, line_top, _font_size, line_height) = view
            .engine
            .empty_source_line_metrics(source_line, lazy, source)
            .expect("editable source line must have an explicit layout row");
        let next_y = flat_line_y_for_source_byte(&view, next_byte)
            .expect("following paragraph should be laid out");

        assert!(
            (line_height - view.engine.base_line_height).abs() < 1.0,
            "editable empty line should use full line height, got {line_height}"
        );
        assert!(
            next_y >= line_top + line_height - 1.0,
            "following line should stay below the editable empty line: top={line_top}, \
             height={line_height}, next_y={next_y}"
        );
    }

    #[test]
    fn cursor_after_paragraph_enter_uses_paragraph_spacing_immediately() {
        let source = "hello\n\n";
        let cursor_byte = source.len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);

        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("cursor on the newly inserted paragraph line should be visible");
        let first_line =
            view.engine().flat_lines().first().expect("first paragraph line should be laid out");
        let cursor_doc_y = cursor_y + view.engine.scroll_y;
        let cursor_top_in_line = cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);
        let cursor_line_top = cursor_doc_y - cursor_top_in_line;
        let expected_line_top =
            first_line.rect.y + first_line.rect.h + view.engine.paragraph_spacing;

        assert_eq!(cursor_x, first_line.rect.x, "empty paragraph cursor should keep line x");
        assert!(
            (cursor_line_top - expected_line_top).abs() < 1.0,
            "new paragraph cursor should include paragraph spacing immediately: \
             got line_top={cursor_line_top}, expected={expected_line_top}"
        );
    }

    #[test]
    fn leading_empty_source_lines_reserve_vertical_space_before_first_block() {
        let baseline = make_editor_view_with_cursor("## title", 0);
        let expanded = make_editor_view_with_cursor("\n\n\n## title", 3);

        let baseline_heading_y =
            flat_line_y_for_source_byte(&baseline, 0).expect("baseline heading should be laid out");
        let expanded_heading_y =
            flat_line_y_for_source_byte(&expanded, 3).expect("expanded heading should be laid out");

        assert!(
            expanded_heading_y >= baseline_heading_y + baseline.engine.base_line_height * 3.0 - 1.0,
            "leading empty source lines should push first block down: \
             baseline={baseline_heading_y}, expanded={expanded_heading_y}"
        );
    }

    #[test]
    fn cursor_after_trailing_newline_moves_to_empty_paragraph_line() {
        let source = "hello\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.engine.handle_set_cursor_byte(source.len());
        let (_x, cursor_y, _w, _h) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve to the new paragraph line");

        assert!(
            cursor_y > 24.0,
            "cursor after trailing newline should be below old paragraph, got y={cursor_y}"
        );
    }

    #[test]
    fn cursor_after_single_trailing_newline_uses_paragraph_geometry() {
        let source = "hello\n";
        let cursor_byte = source.len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);

        let (_cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve");
        let first_line =
            view.engine().flat_lines().first().expect("first paragraph line should be laid out");
        let cursor_doc_y = cursor_y + view.engine.scroll_y;
        let cursor_top_in_line = cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);
        let cursor_line_top = cursor_doc_y - cursor_top_in_line;
        let expected_line_top =
            first_line.rect.y + first_line.rect.h + view.engine.paragraph_spacing;

        assert!(
            (cursor_line_top - expected_line_top).abs() < 1.0,
            "trailing empty paragraph must already include the spacing used after typing: \
             got line_top={cursor_line_top}, expected={expected_line_top}"
        );
    }

    #[test]
    fn cursor_after_paragraph_end_enter_y_position() {
        let source = "hello\n\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        // Act as if we pressed enter at end of paragraph and \n\n was inserted
        view.engine.handle_set_cursor_byte(source.len());
        let (_x, cursor_y, _w, cursor_height) =
            view.engine.cursor_screen_pos().expect("cursor should resolve");

        let baseline_y = cursor_y + cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;

        // the previous line's bottom is roughly base_line_height (24.0)
        // the new line's top should be prev_bottom + paragraph_spacing (24.0 + 12.0 = 36.0)
        // the new line's baseline is top + cursor_height
        let expected_top = view.engine.base_line_height + view.engine.paragraph_spacing;
        assert!(
            baseline_y >= expected_top && baseline_y <= expected_top + cursor_height,
            "cursor baseline_y should be exactly paragraph_spacing below the first line (expected ~{}, got {})",
            expected_top + cursor_height,
            baseline_y
        );
    }

    #[test]
    fn enter_on_empty_block_separator_keeps_inserting_source_newlines() {
        let source = "para1\n\npara2";
        let ctx = classify_enter_context(source, 6);
        match ctx {
            EnterContext::EmptyBlockSeparatorLine => {}
            _ => panic!("Expected EmptyBlockSeparatorLine at byte 6, got {:?}", ctx),
        }

        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(6);

        let aug = view.engine.augment_edit(6, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, 7);
    }

    #[test]
    fn cursor_after_trailing_newline_uses_physical_line_metrics() {
        let source = "hello\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.base_font_size = 15.0;
        view.engine.base_line_height = 24.0;
        render_editor_viewport_with_dpi(&mut view, &doc, 800.0, 600.0, 2.0);

        view.engine.handle_set_cursor_byte(source.len());
        let (_x, cursor_y, _w, cursor_height) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve to the new paragraph line");

        assert!(
            cursor_y > 48.0,
            "2x cursor after trailing newline should advance by a physical line, got y={cursor_y}"
        );
        assert!(
            cursor_height > 24.0,
            "2x cursor height should use physical font metrics, got h={cursor_height}"
        );
    }

    #[test]
    fn cursor_after_trailing_newline_roundtrips_to_empty_line_byte() {
        let source = "hello\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.engine.handle_set_cursor_byte(source.len());
        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve to the new source line");

        let hit_byte =
            view.engine.hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0);

        assert_eq!(
            hit_byte,
            Some(source.len()),
            "cursor rect on the empty source line should map back to the empty line byte"
        );
    }

    #[test]
    fn selection_endpoint_after_trailing_newline_preserves_empty_line_byte() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut doc);

        let selection_range = match view.query(PluginQuery::SelectionRange, &doc) {
            PluginResponse::PositionPair(range) => range,
            other => panic!("expected selection range, got {other:?}"),
        };

        assert_eq!(
            selection_range,
            Some(((0, 0), (source.len(), 0))),
            "selection endpoint on trailing empty line should preserve the source end byte"
        );
    }

    #[test]
    fn backward_selection_from_trailing_newline_preserves_empty_line_anchor() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len())), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(0)), &mut doc);

        let selection_range = match view.query(PluginQuery::SelectionRange, &doc) {
            PluginResponse::PositionPair(range) => range,
            other => panic!("expected selection range, got {other:?}"),
        };

        assert_eq!(
            selection_range,
            Some(((0, 0), (source.len(), 0))),
            "backward selection from trailing empty line should preserve the source end byte"
        );
    }

    #[test]
    fn set_selection_byte_clears_visual_endpoint_when_byte_cannot_map() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);

        view.handle_message(PluginMessage::SetSelCursor(Some((0, 1))), &mut doc);
        assert!(matches!(
            view.query(PluginQuery::SelCursor, &doc),
            PluginResponse::Position(Some((0, 1)))
        ));

        view.handle_message(PluginMessage::SetSelCursorByte(Some(0)), &mut doc);

        assert!(matches!(view.query(PluginQuery::SelCursor, &doc), PluginResponse::Position(None)));
    }

    #[test]
    fn cursor_screen_pos_query_uses_requested_byte() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "abc\n\ndef";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        let first = match view.query(PluginQuery::CursorScreenPos(0), &doc) {
            PluginResponse::CursorScreenRect(Some(rect)) => rect,
            response => panic!("expected first cursor rect, got {response:?}"),
        };
        let second = match view.query(PluginQuery::CursorScreenPos(5), &doc) {
            PluginResponse::CursorScreenRect(Some(rect)) => rect,
            response => panic!("expected second cursor rect, got {response:?}"),
        };

        assert_ne!(first.1, second.1, "different requested bytes should resolve different rows");
    }

    #[test]
    fn trailing_newline_selection_keeps_range_without_empty_line_highlight() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len() - 1)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut doc);

        assert!(
            matches!(view.query(PluginQuery::HasSelection, &doc), PluginResponse::Bool(true)),
            "trailing newline byte range should count as an active selection"
        );

        let selection_range = match view.query(PluginQuery::SelectionRange, &doc) {
            PluginResponse::PositionPair(range) => range,
            other => panic!("expected selection range, got {other:?}"),
        };
        assert_eq!(
            selection_range,
            Some(((source.len() - 1, 0), (source.len(), 0))),
            "trailing newline selection should preserve the source byte range"
        );

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let has_empty_line_highlight = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. } if rect.y > view.engine.base_line_height * 0.5
            )
        });

        assert!(
            !has_empty_line_highlight,
            "trailing newline selection should not synthesize an empty-line highlight"
        );
    }

    #[test]
    fn select_all_highlights_last_physical_line_before_trailing_blanks() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "\
first physical line
second physical line
third physical line
final visible physical line

";
        let final_line_byte =
            source.find("final visible").expect("fixture should contain final line");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(final_line_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(final_line_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on final physical line, got {other:?}"),
            };

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut doc);

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let final_line_is_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_y + cursor_height * 0.5
                        && rect.y + rect.h >= cursor_y + cursor_height * 0.5
            )
        });

        assert!(
            final_line_is_highlighted,
            "select-all should visibly highlight the final physical text line"
        );
    }

    #[test]
    fn selecting_real_last_physical_text_line_reports_highlight() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
        let line_start = source.find("C608-01").expect("fixture should contain final text line");
        let line_end = source[line_start..]
            .find('\n')
            .map(|offset| line_start + offset)
            .expect("fixture should end final text line with newline");
        let probe_byte = source.find("要接受成本").expect("fixture should contain probe text");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(probe_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(probe_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on final text line, got {other:?}"),
            };

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(line_start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(line_end)), &mut doc);

        assert!(
            matches!(view.query(PluginQuery::HasSelection, &doc), PluginResponse::Bool(true)),
            "the final physical text line byte range should count as an active selection"
        );

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let final_line_is_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_y + cursor_height * 0.5
                        && rect.y + rect.h >= cursor_y + cursor_height * 0.5
            )
        });

        assert!(
            final_line_is_highlighted,
            "selection should visibly highlight the real final physical text line"
        );
    }

    #[test]
    fn backward_selection_from_trailing_blank_highlights_real_final_text_line() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
        let probe_byte = source.find("要接受成本").expect("fixture should contain probe text");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(probe_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(probe_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on final text line, got {other:?}"),
            };

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len())), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(probe_byte)), &mut doc);

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let final_line_is_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_y + cursor_height * 0.5
                        && rect.y + rect.h >= cursor_y + cursor_height * 0.5
            )
        });

        assert!(
            final_line_is_highlighted,
            "backward selection from trailing blank should highlight the final text line"
        );
    }

    #[test]
    fn selection_highlight_at_bottom_respects_render_offset_y() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "top line\nbottom line\n\n";
        let bottom_byte = source.find("bottom").expect("fixture should contain bottom line");
        let line_end = source[bottom_byte..]
            .find('\n')
            .map(|offset| bottom_byte + offset)
            .expect("fixture should end bottom line with newline");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.handle_message(PluginMessage::Scroll { delta: 48.0, viewport_h: 48.0 }, &mut doc);
        render_editor_with_offset_y(&mut view, &doc, 48.0, 80.0);

        view.handle_message(PluginMessage::SetCursorByte(bottom_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(bottom_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on bottom line, got {other:?}"),
            };
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(bottom_byte)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(line_end)), &mut doc);

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let cursor_screen_y = 80.0 + cursor_y + cursor_height * 0.5;
        let bottom_line_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_screen_y && rect.y + rect.h >= cursor_screen_y
            )
        });

        assert!(
            bottom_line_highlighted,
            "selection highlight near viewport bottom should not be clipped by offset_y"
        );
    }

    #[test]
    fn blockquote_marker_byte_selection_highlight_uses_constant_source_line_lookups() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut source = String::new();
        for index in 0..180 {
            source.push_str(&format!("paragraph {index}\n\n"));
            if index == 40 || index == 140 {
                source.push_str(&format!("> quoted block {index}\n\n"));
            }
        }
        let first_quote = source.find("> quoted block 40").expect("fixture should contain quote");
        let second_quote = source.find("> quoted block 140").expect("fixture should contain quote");
        let mut doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);
        render_editor_once(&mut view, &doc);

        reset_source_line_at_byte_call_count();
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(first_quote)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(second_quote)), &mut doc);
        match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
            PluginResponse::DrawList(_) => {}
            other => panic!("expected selection highlights, got {other:?}"),
        }

        assert!(
            source_line_at_byte_call_count() <= 4,
            "blockquote marker byte fallback should not rescan source lines per mapped byte; calls={}",
            source_line_at_byte_call_count()
        );
    }

    #[test]
    fn cursor_after_trailing_blank_line_uses_nearby_paragraph_position() {
        let mut source =
            (0..40).map(|idx| format!("paragraph {idx}")).collect::<Vec<_>>().join("\n\n");
        source.push_str("\n\n");
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(&source);
        view.set_source(source.clone(), 1);
        render_editor_once(&mut view, &doc);

        let previous_line = view
            .engine
            .flat_lines()
            .last()
            .expect("fixture should render a previous paragraph")
            .clone();

        view.engine.handle_set_cursor_byte(source.len());
        let (_cursor_x, cursor_y, _cursor_width, _cursor_height) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing blank line should resolve near previous paragraph");

        let max_expected_y = previous_line.rect.y + previous_line.rect.h * 3.0;
        assert!(
            cursor_y < max_expected_y,
            "cursor y should be near previous paragraph, got {cursor_y}, previous={:?}",
            previous_line.rect
        );
    }

    #[test]
    fn markdown_editor_reflows_immediately_when_viewport_width_changes() {
        let source = "这是一段很长的中文测试文本 mixed with english and some more text that \
                      should wrap narrowly but fit into fewer lines in a wide editor viewport.";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);

        render_editor_narrow(&mut view, &doc, 180.0);
        let narrow_line_count = view.engine().flat_lines().len();

        render_editor_narrow(&mut view, &doc, 800.0);
        let wide_line_count = view.engine().flat_lines().len();

        assert!(
            wide_line_count < narrow_line_count,
            "editor should reflow on width change without requiring scroll: narrow={narrow_line_count}, wide={wide_line_count}"
        );
    }

    #[test]
    fn hit_test_byte_after_horizontal_rule_keeps_adjacent_paragraphs_distinct() {
        let source = "# 党内法规发展历程回眸\n\n**作者：林希存**\n> 栏目：党内法规\n---\n第六行普通段落\n\n第八行普通段落";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let flat_lines = view.engine().flat_lines();
        let sixth_line = flat_lines
            .iter()
            .find(|line| line.text.contains("第六行普通段落"))
            .expect("rendered flat lines should contain source line 6 paragraph");
        let eighth_line = flat_lines
            .iter()
            .find(|line| line.text.contains("第八行普通段落"))
            .expect("rendered flat lines should contain source line 8 paragraph");

        let sixth_start = source.find("第六行普通段落").expect("source should contain line 6");
        let sixth_end = sixth_start + "第六行普通段落".len();
        let eighth_start = source.find("第八行普通段落").expect("source should contain line 8");
        let eighth_end = eighth_start + "第八行普通段落".len();

        let sixth_hit = view
            .engine()
            .hit_test_byte(
                sixth_line.rect.x + 1.0,
                sixth_line.rect.y + sixth_line.rect.h * 0.5,
                0.0,
                0.0,
            )
            .expect("hit-test on source line 6 should return a byte");
        let eighth_hit = view
            .engine()
            .hit_test_byte(
                eighth_line.rect.x + 1.0,
                eighth_line.rect.y + eighth_line.rect.h * 0.5,
                0.0,
                0.0,
            )
            .expect("hit-test on source line 8 should return a byte");

        assert!(
            (sixth_start..=sixth_end).contains(&sixth_hit),
            "click on source line 6 mapped to byte {sixth_hit}, expected {sixth_start}..={sixth_end}"
        );
        assert!(
            (eighth_start..=eighth_end).contains(&eighth_hit),
            "click on source line 8 mapped to byte {eighth_hit}, expected {eighth_start}..={eighth_end}"
        );
    }

    #[test]
    fn horizontal_rule_and_its_leading_gap_map_to_marker_boundaries_at_each_dpi() {
        let source = "国内用增,\n\n---\n\n后续";
        let horizontal_rule_start = source.find("---").expect("fixture contains a rule");
        let horizontal_rule_end = horizontal_rule_start + 3;

        for dpi_scale in [1.0, 2.0] {
            let document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.to_owned(), 1);
            render_editor_viewport_with_dpi(&mut view, &document, 800.0, 600.0, dpi_scale);

            let horizontal_rule = view
                .engine()
                .flat_lines()
                .iter()
                .find(|line| line.atomic_source_range.is_some())
                .expect("fixture lays out an inactive horizontal rule");
            let hit_x = horizontal_rule.rect.x + 1.0;
            let leading_gap_y = horizontal_rule.rect.y - 1.0;
            let upper_half_y = horizontal_rule.rect.y + horizontal_rule.rect.h * 0.25;
            let lower_half_y = horizontal_rule.rect.y + horizontal_rule.rect.h * 0.75;

            assert_eq!(
                view.engine().hit_test_byte(hit_x, leading_gap_y, 0.0, 0.0),
                Some(horizontal_rule_start),
                "leading structural gap must snap to the rule start at {dpi_scale}x DPI"
            );
            assert_eq!(
                view.engine().hit_test_byte(hit_x, upper_half_y, 0.0, 0.0),
                Some(horizontal_rule_start),
                "upper rule half must map to the marker start at {dpi_scale}x DPI"
            );
            assert_eq!(
                view.engine().hit_test_byte(hit_x, lower_half_y, 0.0, 0.0),
                Some(horizontal_rule_end),
                "lower rule half must map to the marker end at {dpi_scale}x DPI"
            );
        }
    }

    #[test]
    fn first_click_on_inactive_horizontal_rule_expands_its_marker() {
        let source = "国内用增,\n\n---\n\n后续";
        let document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_once(&mut view, &document);

        let horizontal_rule = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("fixture lays out an inactive horizontal rule");
        let hit = view
            .engine()
            .hit_test_byte(
                horizontal_rule.rect.x + 1.0,
                horizontal_rule.rect.y + horizontal_rule.rect.h * 0.25,
                0.0,
                0.0,
            )
            .expect("inactive horizontal rule must be clickable");

        view.engine.handle_set_cursor_byte(hit);
        render_editor_once(&mut view, &document);

        assert!(
            view.engine().flat_lines().iter().any(|line| line.text.contains("---")),
            "clicking an inactive rule must expose its source marker for editing"
        );
    }

    #[test]
    fn removing_extra_blank_line_before_horizontal_rule_clears_reserved_height() {
        let roomy_source = "国内用增,\n\n\n---";
        let compact_source = "国内用增,\n\n---";
        let mut document = StubDoc::new(roomy_source);
        let mut view = MarkdownEditorView::new();
        view.set_source(roomy_source.to_owned(), 1);
        render_editor_once(&mut view, &document);
        let roomy_rule_y = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("roomy fixture lays out a horizontal rule")
            .rect
            .y;

        document = StubDoc::new(compact_source);
        view.set_source(compact_source.to_owned(), 2);
        render_editor_once(&mut view, &document);
        let updated_rule_y = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("updated fixture lays out a horizontal rule")
            .rect
            .y;

        let fresh_document = StubDoc::new(compact_source);
        let mut fresh_view = MarkdownEditorView::new();
        fresh_view.set_source(compact_source.to_owned(), 1);
        render_editor_once(&mut fresh_view, &fresh_document);
        let fresh_rule_y = fresh_view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("fresh fixture lays out a horizontal rule")
            .rect
            .y;

        let editable_blank_line_extent =
            view.engine().base_line_height + view.engine().paragraph_spacing;
        assert!((updated_rule_y - fresh_rule_y).abs() < 0.01);
        assert!((roomy_rule_y - updated_rule_y - editable_blank_line_extent).abs() < 0.01);
    }

    #[test]
    fn wrapped_bold_line_end_returns_correct_segment_end() {
        // LineEnd on a wrapped expanded line goes to the end of the current
        // visual segment (not the first segment of the logical line).
        // find_flat_and_grapheme_for_byte correctly identifies the segment
        // containing the cursor byte via per-segment exact matching.
        let long_text = "**".to_string() + &"word ".repeat(30) + "**";
        let mut v = make_view(&long_text);
        // Place cursor somewhere in the middle of the expanded span
        v.engine_mut().handle_set_cursor_byte(10);

        let result = v.engine().visual_move(10, MoveDirection::LineEnd, None);
        assert!(result.is_some(), "LineEnd on wrapped expanded line should return Some");
        let end_byte = result.unwrap();
        // The byte should be >= current position
        assert!(end_byte >= 10, "LineEnd byte should be >= current position");

        // Roundtrip: cursor_screen_pos at end_byte should hit back to end_byte
        v.engine_mut().handle_set_cursor_byte(end_byte);
        let (x, y, _w, h) = v.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = v.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);
        assert_eq!(hit, Some(end_byte), "LineEnd byte roundtrip should return same byte");
    }

    // ── visual_move: Up/Down across lines ───────────────────────────────

    #[test]
    fn visual_move_down_from_first_line() {
        let v = make_view("line one\nline two");
        // Start at byte 0 (first char of first line)
        let result = v.engine().visual_move(0, MoveDirection::Down, None);
        // Down should move to second line
        assert!(result.is_some(), "Down from first line should return Some");
    }

    #[test]
    fn visual_move_up_from_first_line_returns_same() {
        let v = make_view("hello");
        let result = v.engine().visual_move(0, MoveDirection::Up, None);
        // Up from first line should stay at same position
        assert_eq!(result, Some(0), "Up from first line should stay at byte 0");
    }

    #[test]
    fn visual_move_at_outer_visual_rows_reaches_document_boundaries() {
        let source = "hello\nworld";
        let view = make_view(source);

        assert_eq!(view.engine().visual_move(3, MoveDirection::Up, None), Some(0));
        assert_eq!(
            view.engine().visual_move(source.len() - 2, MoveDirection::Down, None),
            Some(source.len())
        );
    }

    #[test]
    fn projected_empty_paragraph_outside_quote_uses_body_indent() {
        let source = "> quoted\n\n";
        let view = make_view(source);
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(source.len(), CursorAffinity::Downstream)
            .expect("trailing empty line should have a projection position");
        let previous_line_x =
            view.engine().flat_lines().first().expect("quoted paragraph should render").rect.x;

        assert!(previous_line_x > 0.0, "fixture must provide a non-zero indentation");
        let lazy = view.engine().lazy.as_ref().expect("view should retain lazy layout state");
        assert_eq!(view.engine().projection_screen_x(lazy, position), Some(0.0));
    }

    #[test]
    fn unshaped_navigation_preserves_proportional_font_advances() {
        let mut view = make_projected_view("Wi");
        let settings = MarkdownRenderSettings { font_family: "PingFang SC", ..default_settings() };
        let mut shaper = shaping::Shaper::new().expect("proportional navigation requires fonts");
        view.render(&ui::theme::test_theme(), 800.0, 600.0, 0.0, 0.0, settings, Some(&mut shaper));
        view.engine_mut().lazy.as_mut().expect("projected view should retain layout").flat_lines
            [0]
        .shaped = None;
        let line = &view.engine().flat_lines()[0];
        let after_w = view.engine().grapheme_x_for_line(line, 1);
        let after_i = view.engine().grapheme_x_for_line(line, 2);
        let wide_advance = after_w;
        let narrow_advance = after_i - after_w;

        assert!(
            wide_advance > narrow_advance * 1.5,
            "unshaped navigation must retain proportional advances: W={wide_advance}, i={narrow_advance}"
        );
    }

    #[test]
    fn unshaped_code_navigation_preserves_monospace_advances() {
        let source = "```\nWi\n```";
        let mut view = make_projected_view(source);
        let code_line_index = view
            .engine()
            .flat_lines()
            .iter()
            .position(|line| line.text == "Wi")
            .expect("code content line should render");
        view.engine_mut().lazy.as_mut().expect("projected view should retain layout").flat_lines
            [code_line_index]
            .shaped = None;
        let line = &view.engine().flat_lines()[code_line_index];
        let after_w = view.engine().grapheme_x_for_line(line, 1);
        let after_i = view.engine().grapheme_x_for_line(line, 2);
        let wide_advance = after_w;
        let narrow_advance = after_i - after_w;

        assert!(
            (wide_advance - narrow_advance).abs() < wide_advance * 0.15,
            "unshaped code navigation must retain monospace advances: W={wide_advance}, i={narrow_advance}"
        );
    }

    #[test]
    fn visual_move_line_start_and_end() {
        let v = make_view("- list item");
        // byte 2 is 'l' (first char of "list item")
        let start = v.engine().visual_move(5, MoveDirection::LineStart, None);
        assert!(start.is_some(), "LineStart should return Some");
        let end = v.engine().visual_move(5, MoveDirection::LineEnd, None);
        assert!(end.is_some(), "LineEnd should return Some");
        // LineStart should be before LineEnd
        assert!(start.unwrap() <= end.unwrap(), "LineStart <= LineEnd");
    }

    // ── MarkdownEditorView (ViewPlugin impl) ─────────────────────────

    struct StubDoc {
        text: String,
    }

    impl StubDoc {
        fn new(text: &str) -> Self {
            Self { text: text.to_string() }
        }
    }

    impl core::document::DocView for StubDoc {
        fn line_count(&self) -> usize {
            self.text.lines().count().max(1)
        }

        fn doc_line_text(&self, line: usize) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Owned(self.text.lines().nth(line).unwrap_or("").to_string())
        }

        fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> std::borrow::Cow<'_, str> {
            let start = range.start.min(self.text.len());
            let end = range.end.min(self.text.len());
            std::borrow::Cow::Owned(self.text[start..end].to_string())
        }

        fn line_byte_offset(&self, line: usize) -> usize {
            let mut byte_offset = 0usize;
            for (idx, segment) in self.text.split_inclusive('\n').enumerate() {
                if idx == line {
                    return byte_offset;
                }
                byte_offset += segment.len();
            }
            self.text.len()
        }

        fn line_byte_length(&self, line: usize) -> usize {
            self.text.lines().nth(line).map(|s| s.len()).unwrap_or(0)
        }

        fn scroll_y(&self) -> f32 {
            0.0
        }

        fn viewport_height(&self) -> f32 {
            600.0
        }
    }

    impl core::document::DocViewMut for StubDoc {}

    fn render_editor_once(view: &mut MarkdownEditorView, doc: &StubDoc) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn render_editor_with_offset_y(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        height: f32,
        offset_y: f32,
    ) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, offset_y, 800.0, height);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn make_editor_view_with_cursor(source: &str, cursor_byte: usize) -> MarkdownEditorView {
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);
        view
    }

    fn render_editor_draw_list(view: &mut MarkdownEditorView, doc: &StubDoc) -> DrawList {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0)
    }

    #[test]
    fn heading_space_shows_empty_heading_feedback_without_source_prefix() {
        let mut view = MarkdownEditorView::new();
        for (generation, source, expected_text) in
            [(1, "###", "###"), (2, "### ", ""), (3, "### 标题", "标题")]
        {
            view.set_source(source.to_owned(), generation);
            view.engine.handle_set_cursor_byte(source.len());
            let document = StubDoc::new(source);
            let commands = render_editor_draw_list(&mut view, &document);
            let line = &view.engine.flat_lines()[0];
            assert_eq!(line.text, expected_text, "source={source:?}");
            assert!(line.source_projection.as_ref().is_some_and(|projection| {
                projection.boundaries.last().is_some_and(|anchor| anchor.byte == source.len())
            }));
            let has_placeholder = commands.cmds.iter().any(|command| {
                matches!(command, ui::DrawCmd::TextLayout { layout, .. } if layout.text == "三级标题")
            });
            assert_eq!(has_placeholder, source == "### ", "source={source:?}");
            let (cursor_x, cursor_y, _, _) =
                view.engine.cursor_screen_pos().expect("heading caret");
            assert_eq!(
                view.engine.hit_test_byte(cursor_x, cursor_y + line.rect.h * 0.5, 0.0, 0.0),
                Some(source.len())
            );
        }
    }

    #[test]
    fn empty_heading_enter_and_backspace_keep_paragraph_semantics() {
        use super::tests::apply_single_replacement;
        use ui::plugin::{EditIntent, EditPolicy, EditRequest};

        let source = "### ";
        let mut view = make_editor_view_with_cursor(source, source.len());
        for intent in [EditIntent::DeleteBackward, EditIntent::InsertParagraphBreak] {
            let request = EditRequest {
                source_generation: 1,
                cursor_byte: source.len(),
                selection: None,
                intent: intent.clone(),
            };
            let (edited, cursor) = apply_single_replacement(source, &view.plan_edit(&request));
            if intent == EditIntent::DeleteBackward {
                assert_eq!(edited, "");
            }
            view.set_source(edited.clone(), 2);
            view.engine.handle_set_cursor_byte(cursor);
            let commands = render_editor_draw_list(&mut view, &StubDoc::new(&edited));
            assert!(!commands.cmds.iter().any(|command| matches!(command,
                ui::DrawCmd::TextLayout { layout, .. } if layout.text == "三级标题")));
            let line = view.engine.flat_lines().last().expect("body row");
            let style = view.engine.cached_query_style.as_ref().expect("resolved body style");
            assert_eq!(line.font_size, style.body_font_size);
            view.set_source(source.to_owned(), 1);
        }
    }

    #[test]
    fn empty_heading_feedback_uses_heading_metrics_at_each_dpi_and_source_position() {
        let theme = ui::theme::test_theme();
        for dpi_scale in [1.0, 2.0] {
            for level in 1..=6 {
                for suffix in ["", "\n\n正文"] {
                    let prefix = format!("{} ", "#".repeat(level));
                    let source = format!("{prefix}{suffix}");
                    let mut view = MarkdownEditorView::new();
                    view.set_source(source.clone(), 1);
                    view.engine.handle_set_cursor_byte(prefix.len());
                    let mut shaper = shaping::Shaper::new().expect("heading test shaper");
                    let commands = view.render(
                        &StubDoc::new(&source),
                        ui::Rect::new(0.0, 0.0, 800.0 * dpi_scale, 600.0 * dpi_scale),
                        &theme,
                        &mut shaper,
                        dpi_scale,
                    );
                    let placeholder = commands
                        .cmds
                        .iter()
                        .find_map(|command| match command {
                            ui::DrawCmd::TextLayout { layout, .. }
                                if layout.text.ends_with("级标题") =>
                            {
                                Some(layout)
                            }
                            _ => None,
                        })
                        .expect("empty heading needs a visible placeholder");
                    let line = &view.engine.flat_lines()[0];
                    assert!(line.text.is_empty());
                    let style = view.engine.cached_query_style.as_ref().expect("resolved style");
                    assert_eq!(placeholder.font_size, style.heading_font_sizes[level - 1]);
                    assert_eq!(line.font_size, placeholder.font_size);
                    assert_eq!(
                        line.source_projection
                            .as_ref()
                            .expect("empty heading projection")
                            .boundaries[0]
                            .byte,
                        prefix.len()
                    );
                }
            }
        }
    }

    #[test]
    fn shift_enter_hides_terminal_hard_break_and_keeps_paragraph_line_spacing() {
        use super::tests::apply_single_replacement;
        use ui::plugin::{EditIntent, EditPolicy, EditRequest};

        for source in [
            "正文",
            "前文\n\n正文",
            "**正文**",
            "- 正文",
            "> 正文",
            "正文\r\n续行",
            "- > 正文",
            "> - 正文",
            "1. 正文",
        ] {
            let mut view = MarkdownEditorView::new();
            view.set_source(source.to_owned(), 1);
            let request = EditRequest {
                source_generation: 1,
                cursor_byte: source.len(),
                selection: None,
                intent: EditIntent::InsertLineBreak,
            };
            let (edited, cursor) = apply_single_replacement(source, &view.plan_edit(&request));
            view.set_source(edited.clone(), 2);
            view.engine.handle_set_cursor_byte(cursor);
            render_editor_once(&mut view, &StubDoc::new(&edited));
            let lines = view.engine.flat_lines();
            assert!(lines.iter().all(|line| !line.text.contains('\\')), "source={edited:?}");
            let continuation = lines.last().expect("hard break continuation");
            assert_eq!(continuation.text, "", "source={edited:?}");
            let previous = &lines[lines.len() - 2];
            assert!(
                (continuation.rect.y - previous.rect.y - previous.rect.h).abs() < 0.01,
                "hard break must have no paragraph gap: source={edited:?}"
            );
            let (cursor_x, cursor_y, _, _) =
                view.engine.cursor_screen_pos().expect("continuation caret");
            assert_eq!(
                view.engine.hit_test_byte(cursor_x, cursor_y + continuation.rect.h * 0.5, 0.0, 0.0),
                Some(cursor)
            );
            let backspace = EditRequest {
                source_generation: 2,
                cursor_byte: cursor,
                selection: None,
                intent: EditIntent::DeleteBackward,
            };
            let (restored, _) = apply_single_replacement(&edited, &view.plan_edit(&backspace));
            assert_eq!(restored, source);
        }
    }

    #[test]
    fn first_line_indent_plugin_setting_updates_cached_layout_caret_and_hit_testing() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        for source in ["普通段落内容", ""] {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.to_owned(), 1);
            view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
            let font_size = 15.0;
            let dpi_scale = 2.0;
            for enabled in [false, true, false] {
                view.handle_message(
                    PluginMessage::SetRenderSettings {
                        font_family: ui::settings::platform_default_font_family().to_string(),
                        font_size,
                        line_height: 24.0,
                        toc_max_depth: 3,
                        markdown_first_line_indent: enabled,
                    },
                    &mut document,
                );
                render_editor_draw_list_with_dpi(&mut view, &document, dpi_scale);
                let first_line = &view.engine().flat_lines()[0];
                let expected_x = if enabled {
                    PARAGRAPH_FIRST_LINE_INDENT_EMS * font_size * dpi_scale
                } else {
                    0.0
                };
                assert_eq!(first_line.rect.x, expected_x, "setting={enabled}, source={source:?}");
                let (cursor_x, _, _, _) = view
                    .engine()
                    .cursor_screen_pos()
                    .expect("editable paragraph must expose a cursor position");
                assert_eq!(cursor_x, expected_x);
                assert_eq!(
                    view.engine().hit_test_byte(
                        expected_x,
                        first_line.rect.y + first_line.rect.h / 2.0,
                        0.0,
                        0.0
                    ),
                    Some(0)
                );
            }
        }
    }

    fn rendered_text_commands(draw_list: &DrawList) -> Vec<&str> {
        draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn render_editor_draw_list_with_dpi(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        dpi_scale: f32,
    ) -> DrawList {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        <MarkdownEditorView as ViewPlugin>::render(
            view,
            doc,
            bounds,
            &theme,
            &mut shaper,
            dpi_scale,
        )
    }

    fn editor_cursor_rect_from_draw_list(dl: &DrawList) -> ui::core::geom::Rect {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        dl.cmds
            .iter()
            .filter_map(|cmd| match cmd {
                ui::core::paint::DrawCmd::FillRect { rect, color, .. }
                    if *color == theme.editor.cursor =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .next_back()
            .expect("editor cursor fill rect should be present")
    }

    fn flat_line_y_for_source_byte(view: &MarkdownEditorView, source_byte: usize) -> Option<f32> {
        let lazy = view.engine.lazy.as_ref()?;
        for flat_line in &lazy.flat_lines {
            if flat_line.source_projection.as_ref().is_some_and(|projection| {
                projection.boundaries.first().is_some_and(|anchor| anchor.byte == source_byte)
            }) {
                return Some(flat_line.rect.y);
            }
        }
        None
    }

    /// Render editor with narrow width to trigger soft wrapping of long lines.
    fn render_editor_narrow(view: &mut MarkdownEditorView, doc: &StubDoc, width: f32) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, width, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn render_editor_viewport(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        width: f32,
        height: f32,
    ) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, width, height);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn render_editor_viewport_with_dpi(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        width: f32,
        height: f32,
        dpi_scale: f32,
    ) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, width, height);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ = <MarkdownEditorView as ViewPlugin>::render(
            view,
            doc,
            bounds,
            &theme,
            &mut shaper,
            dpi_scale,
        );
    }

    #[test]
    fn markdown_editor_full_flat_lines_only_precisely_shapes_visible_blocks() {
        let source = (0..80)
            .map(|idx| format!("paragraph {idx} with enough text for layout\n\n"))
            .collect::<String>();
        let doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);

        render_editor_viewport(&mut view, &doc, 800.0, 80.0);

        let lazy = view.engine().lazy.as_ref().expect("editor render should build layout");
        let precise_count = lazy.precise.iter().filter(|is_precise| **is_precise).count();
        let materialized_count = lazy.laid_out.iter().filter(|block| block.is_some()).count();

        assert_eq!(materialized_count, lazy.estimated_heights.len());
        assert!(!lazy.flat_lines.is_empty());
        assert!(
            precise_count < lazy.estimated_heights.len(),
            "full-flat WYSIWYG render should keep precise shaping viewport-scoped"
        );
    }

    #[test]
    fn markdown_editor_table_resize_preserves_projection_boundaries() {
        let rows = [
            "| G-01 | P0 | 已确认 | LLM 写入路径未限制在 `wiki/` | 原始来源或状态文件可被覆盖 |",
            "| G-02 | P0 | 已确认 | 任意一个对象写入成功即可判定摄入成功 | 缺页、坏 frontmatter、截断输出被误报成功 |",
            "| G-03 | P0 | 已确认 | 增量批量部分失败后跳过全部后处理 | 页面已落盘但 index/search 不可见或陈旧 |",
            "| G-04 | P0 | 已确认 | 并行重建共享输出目录且没有跨文件上下文 | 同名页面覆盖、知识重复、跨文档关系缺失 |",
            "| G-05 | P0 | 已确认 | 重建只要一个文件成功就发布不完整 Wiki | 旧 Wiki 中对应失败来源的内容被整体丢弃 |",
            "| G-06 | P1 | 已确认 | 重试没有清理第一次尝试的部分输出 | 失败残留混入第二次结果，产生幽灵页/重复页 |",
            "| G-07 | P0 | 已确认 | 最大 10 MB 原文整段进入单次 prompt，无分块 | 上下文溢出、超时、高费用、重试放大成本 |",
            "| G-08 | P1 | 已确认 | index/hot/log/deadlink/search 后处理错误不向上返回 | UI 显示成功但派生数据不完整 |",
            "| G-09 | P1 | 已确认 | 影子重建时 topic registry 写入 live wiki | 破坏影子目录隔离，失败重建也会改线上状态 |",
            "| G-10 | P1 | 已确认 | 死链标记不可逆 | 后续目标页生成后仍保持删除线死链 |",
            "| G-11 | P2 | 已确认 | `log.md` 每次被重建为快照，不是追加日志 | 历史摄入记录丢失，审计能力失效 |",
            "| G-12 | P1 | 已确认 | provider/send 未配置时上传锁不释放 | 文件不生成，后续上传/重建永久被锁住 |",
            "| G-13 | P1 | 已确认 | 重建状态只存在于前端内存 | 断线/切换 persona 后无法恢复进度和结果 |",
            "| G-14 | P1 | 已确认 | 重试和失败也发送 `complete` 进度 | 重建进度可超过总文件数、提前显示完成 |",
            "| G-15 | P2 | 风险 | 原文直接嵌入 prompt，且输出路径权限过宽 | 文档内提示注入可能放大为错误写入 |",
        ];
        let source = format!(
            "| ID | 优先级 | 状态 | 问题 | 主要影响 |\n|---|---|---|---|---|\n{}",
            rows.join("\n")
        );
        let doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);
        for viewport_width in [200.0, 400.0, 800.0, 1200.0, 1600.0, 2000.0] {
            render_editor_viewport(&mut view, &doc, viewport_width, 600.0);
        }
    }

    #[test]
    fn markdown_editor_initial_render_highlights_only_visible_code_blocks() {
        let mut source = String::new();
        for idx in 0..300 {
            source.push_str(&format!("```rust\nfn function_{idx}() {{}}\n```\n\n"));
        }
        let doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);

        render_editor_viewport(&mut view, &doc, 800.0, 240.0);

        let lazy = view.engine().lazy.as_ref().expect("layout should exist");
        let mut code_blocks = 0usize;
        let mut highlighted_blocks = 0usize;
        for block in lazy.laid_out.iter().filter_map(|block| block.as_ref()) {
            if let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind {
                code_blocks += 1;
                if lines.iter().any(|line| !line.highlight_spans.is_empty()) {
                    highlighted_blocks += 1;
                }
            }
        }

        assert!(highlighted_blocks > 0, "visible code blocks should be highlighted");
        assert!(
            highlighted_blocks < code_blocks / 2,
            "initial editor render should not highlight the whole document: {highlighted_blocks}/{code_blocks}"
        );
    }

    fn render_preview_narrow(view: &mut MarkdownView, width: f32) {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = default_settings();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        view.render(&theme, width, 600.0, 0.0, 0.0, settings, Some(&mut shaper));
    }

    #[test]
    fn preview_select_all_includes_final_line_outside_initial_lazy_viewport() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut source = String::new();
        for idx in 0..80 {
            source.push_str(&format!("ordinary paragraph {idx}\n\n"));
        }
        source.push_str("final selectable line\n\n");

        let mut doc = StubDoc::new(&source);
        let mut view = MarkdownView::new();
        view.set_source(source, 1);
        render_preview_narrow(&mut view, 800.0);

        view.handle_message(PluginMessage::SelectAll, &mut doc);

        let selected_text = match view.query(PluginQuery::SelectedText, &doc) {
            PluginResponse::String(text) => text,
            other => panic!("expected selected text, got {other:?}"),
        };

        assert!(
            selected_text.contains("final selectable line"),
            "preview select-all must include the final visible line, got {selected_text:?}"
        );
    }

    #[test]
    fn preview_copy_preserves_paragraph_boundary() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "first paragraph\n\nsecond paragraph";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownView::new();
        view.set_source(source.to_owned(), 1);
        render_preview_narrow(&mut view, 800.0);
        view.handle_message(PluginMessage::SelectAll, &mut doc);

        let copied = view.query(PluginQuery::SelectedText, &doc);
        assert!(matches!(
            copied,
            PluginResponse::String(text) if text == "first paragraph\n\nsecond paragraph"
        ));
    }

    #[test]
    fn preview_copy_preserves_extra_blank_line_without_copying_visual_wraps() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "first paragraph has enough words to wrap at a narrow width\n\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownView::new();
        view.set_source(source.to_owned(), 1);
        render_preview_narrow(&mut view, 180.0);
        assert!(view.engine().flat_lines().len() > 2, "first paragraph should wrap");
        view.handle_message(PluginMessage::SelectAll, &mut doc);

        let copied = view.query(PluginQuery::SelectedText, &doc);
        assert!(matches!(copied, PluginResponse::String(text) if text == source));
    }

    #[test]
    fn editor_copy_uses_visible_text_and_paragraph_breaks() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "# Heading\n\nfirst **bold** paragraph\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_narrow(&mut view, &doc, 800.0);
        view.handle_message(PluginMessage::SelectAll, &mut doc);

        let copied = view.query(PluginQuery::SelectedText, &doc);
        assert!(matches!(
            copied,
            PluginResponse::String(text) if text == "Heading\n\nfirst bold paragraph\n\nsecond"
        ));
    }

    #[test]
    fn preview_copy_excludes_fence_lines_from_paragraph_spacing() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "first\n\n```rust\ncode\n```\n\nlast";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownView::new();
        view.set_source(source.to_owned(), 1);
        render_preview_narrow(&mut view, 800.0);
        view.handle_message(PluginMessage::SelectAll, &mut doc);

        let copied = view.query(PluginQuery::SelectedText, &doc);
        assert!(matches!(copied, PluginResponse::String(text) if text == "first\n\ncode\n\nlast"));
    }

    fn visible_line_snapshots(engine: &PreviewEngine) -> Vec<(String, i32)> {
        engine
            .flat_lines()
            .iter()
            .map(|line| (line.text.clone(), line.rect.y.round() as i32))
            .collect()
    }

    fn layout_snapshot(engine: &PreviewEngine) -> (Vec<(String, i32)>, i32) {
        (visible_line_snapshots(engine), engine.content_height.round() as i32)
    }

    fn byte_offset_for_line(source: &str, line_number: usize) -> usize {
        source.split_inclusive('\n').take(line_number.saturating_sub(1)).map(str::len).sum()
    }

    #[test]
    fn editor_view_cursor_blink_ownership() {
        use ui::plugin::ViewPlugin;
        let v = MarkdownEditorView::new();
        assert!(!v.shows_cursor(), "WYSIWYG draws its own cursor — app must not overlay one");
        assert!(
            v.needs_cursor_blink_wakeup(),
            "WYSIWYG needs app to compute and forward blink phase"
        );
    }

    #[test]
    fn editor_view_render_draws_cursor_rect_when_cursor_set() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut v = MarkdownEditorView::new();
        v.set_source("hello world".into(), 1);

        // First render to build the lazy layout (required for cursor_screen_pos).
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut doc = StubDoc::new("hello world");
        let _ = <MarkdownEditorView as ViewPlugin>::render(
            &mut v,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        // Place cursor at byte 5 via plugin message.
        v.handle_message(ui::plugin::PluginMessage::SetCursorByte(5), &mut doc);
        assert!(v.engine().cursor_screen_pos().is_some(), "cursor must be positioned after render");

        // Second render: should include the cursor rect.
        let dl = <MarkdownEditorView as ViewPlugin>::render(
            &mut v,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        // The DrawList should contain at least one FillRect for the cursor.
        let has_cursor_fill = dl.cmds.iter().any(
            |cmd| matches!(cmd, DrawCmd::FillRect { color, .. } if *color == theme.editor.cursor),
        );
        assert!(
            has_cursor_fill,
            "render() must draw a cursor rect using theme.editor.cursor color"
        );
    }

    #[test]
    fn new_empty_editor_draws_cursor_without_source_update() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let mut document = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);

        let draw_list = render_editor_draw_list(&mut view, &document);
        let cursor_rect = editor_cursor_rect_from_draw_list(&draw_list);

        assert!(
            cursor_rect.x + cursor_rect.w > 0.0,
            "empty document cursor must overlap editor bounds"
        );
        assert!(
            cursor_rect.y + cursor_rect.h > 0.0,
            "empty document cursor must overlap editor bounds"
        );
    }

    #[test]
    fn new_empty_editor_renders_preedit_without_source_update() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut document = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut document,
        );

        let draw_list = render_editor_draw_list(&mut view, &document);
        let (preedit_x, preedit_width) = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, x, .. } if layout.text == "拼音" => {
                    Some((*x, layout.shaped.width))
                }
                _ => None,
            })
            .expect("new empty editor should emit the IME preedit text layout");
        let cursor_x = match view.query(PluginQuery::CursorScreenPos(0), &document) {
            PluginResponse::CursorScreenRect(Some((x, _, _, _))) => x,
            other => panic!("expected preedit cursor rect, got {other:?}"),
        };

        assert!(
            (cursor_x - (preedit_x + preedit_width)).abs() < 0.01,
            "preedit cursor must follow shaped text: cursor={cursor_x}, text_end={}",
            preedit_x + preedit_width
        );
    }

    #[test]
    fn new_empty_editor_uses_physical_typography_at_high_dpi() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut document = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut document,
        );

        let dpi_scale = 2.0;
        let expected_font_size = view.engine().base_font_size * dpi_scale;
        let draw_list = render_editor_draw_list_with_dpi(&mut view, &document, dpi_scale);
        let cursor_rect = editor_cursor_rect_from_draw_list(&draw_list);
        let preedit_font_size = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "拼音" => {
                    Some(layout.font_size)
                }
                _ => None,
            })
            .expect("new empty editor should emit the IME preedit text layout");
        let queried_cursor_height = match view.query(PluginQuery::CursorScreenPos(0), &document) {
            PluginResponse::CursorScreenRect(Some((_, _, _, height))) => height,
            other => panic!("expected preedit cursor rect, got {other:?}"),
        };

        assert!((cursor_rect.h - expected_font_size).abs() < 0.01);
        assert!((preedit_font_size - expected_font_size).abs() < 0.01);
        assert!((queried_cursor_height - expected_font_size).abs() < 0.01);
    }

    #[test]
    fn editor_view_hides_cursor_when_blink_phase_hidden() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, ViewPlugin};

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut doc = StubDoc::new("hello world");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(PluginMessage::SetCursorVisible(false), &mut doc);

        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let dl = <MarkdownEditorView as ViewPlugin>::render(
            &mut view,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        let has_cursor_fill = dl.cmds.iter().any(
            |cmd| matches!(cmd, DrawCmd::FillRect { color, .. } if *color == theme.editor.cursor),
        );
        assert!(!has_cursor_fill, "hidden blink phase must not draw WYSIWYG cursor");
    }

    #[test]
    fn editor_cursor_rect_uses_text_height_not_line_box() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::CursorScreenPos(5), &doc);
        let Some((_x, _y, _w, height)) = (match response {
            PluginResponse::CursorScreenRect(rect) => rect,
            other => panic!("expected CursorScreenRect, got {other:?}"),
        }) else {
            panic!("cursor rect should resolve");
        };

        assert!(
            (height - view.engine().base_font_size).abs() < 0.01,
            "cursor height should match glyph height, got {height}"
        );
    }

    #[test]
    fn editor_preedit_cursor_follows_materialized_preedit_layout() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        let base_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        view.handle_message(
            PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((1, 1)) },
            &mut doc,
        );
        let preedit_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        view.handle_message(
            PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((4, 4)) },
            &mut doc,
        );
        let end_rect = editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        assert!(
            preedit_rect.x > base_rect.x,
            "preedit cursor should advance from base cursor; base={}, preedit={}",
            base_rect.x,
            preedit_rect.x
        );
        assert!(
            preedit_rect.x < end_rect.x,
            "preedit cursor at byte 1 should stay before end cursor; preedit={}, end={}",
            preedit_rect.x,
            end_rect.x
        );
    }

    #[test]
    fn editor_multiline_preedit_cursor_uses_the_virtual_second_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        let base_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        let preedit = "中\n文";
        view.handle_message(
            PluginMessage::SetPreedit {
                text: preedit.into(),
                cursor: Some((preedit.len(), preedit.len())),
            },
            &mut doc,
        );
        let preedit_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        assert!(
            preedit_rect.y > base_rect.y + base_rect.h * 0.5,
            "multiline preedit caret must move to its virtual second line: base={base_rect:?}, preedit={preedit_rect:?}"
        );
    }

    #[test]
    fn editor_preedit_cursor_uses_physical_font_size_at_high_dpi() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        fn actual_preedit_advance(dpi_scale: f32) -> f32 {
            let source = "hello world";
            let mut doc = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.into(), 1);

            let _ = render_editor_draw_list_with_dpi(&mut view, &doc, dpi_scale);
            view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
            let base_rect = editor_cursor_rect_from_draw_list(&render_editor_draw_list_with_dpi(
                &mut view, &doc, dpi_scale,
            ));

            view.handle_message(
                PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((1, 1)) },
                &mut doc,
            );
            let preedit_rect = editor_cursor_rect_from_draw_list(
                &render_editor_draw_list_with_dpi(&mut view, &doc, dpi_scale),
            );
            preedit_rect.x - base_rect.x
        }

        let advance_1x = actual_preedit_advance(1.0);
        let advance_2x = actual_preedit_advance(2.0);
        assert!(
            (advance_2x - advance_1x * 2.0).abs() < 0.05,
            "preedit cursor advance should scale with physical DPI; 1x={advance_1x}, 2x={advance_2x}"
        );
    }

    #[test]
    fn editor_cursor_screen_pos_includes_preedit_cursor_advance() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        let base_response = view.query(PluginQuery::CursorScreenPos(5), &doc);
        let Some((base_x, _base_y, _base_w, _base_h)) = (match base_response {
            PluginResponse::CursorScreenRect(rect) => rect,
            other => panic!("expected CursorScreenRect, got {other:?}"),
        }) else {
            panic!("cursor rect should resolve before preedit");
        };

        view.handle_message(
            PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let preedit_response = view.query(PluginQuery::CursorScreenPos(5), &doc);
        let Some((preedit_x, _preedit_y, _preedit_w, _preedit_h)) = (match preedit_response {
            PluginResponse::CursorScreenRect(rect) => rect,
            other => panic!("expected CursorScreenRect, got {other:?}"),
        }) else {
            panic!("cursor rect should resolve with preedit");
        };

        let cursor_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));
        let drawn_cursor_center_x = cursor_rect.x + cursor_rect.w * 0.5;

        assert!(
            preedit_x > base_x,
            "CursorScreenPos should advance into preedit; base={base_x}, preedit={preedit_x}"
        );
        assert!(
            (preedit_x - drawn_cursor_center_x).abs() < 0.01,
            "CursorScreenPos should match drawn preedit caret x; query={preedit_x}, drawn={drawn_cursor_center_x}"
        );
    }

    #[test]
    fn editor_preedit_is_materialized_at_cursor() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        assert_eq!(lines.first().map(|line| line.text.as_str()), Some("helloni world"));
    }

    #[test]
    fn editor_preedit_is_materialized_only_on_cursor_line() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "first line\n\nhello world\n\nthird line";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        let cursor_byte = "first line\n\nhello".len();
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let texts = lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert_eq!(texts, vec!["first line", "helloni world", "third line"]);
    }

    #[test]
    fn editor_renders_preedit_on_trailing_empty_source_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut doc,
        );

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let preedit_is_rendered = draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                ui::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "拼音"
            )
        });

        assert!(preedit_is_rendered, "preedit text must render on a trailing empty source line");
    }

    #[test]
    fn editor_renders_preedit_after_trailing_space() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello ";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut doc,
        );

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let rendered_text = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let preedit_is_rendered = rendered_text.iter().any(|text| text.contains("拼音"));

        assert!(
            preedit_is_rendered,
            "preedit text must render after a trailing source space; rendered={rendered_text:?}"
        );
    }

    #[test]
    fn empty_line_preedit_cursor_uses_shaped_advance() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut doc,
        );

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let (preedit_x, preedit_width) = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, x, .. } if layout.text == "拼音" => {
                    Some((*x, layout.shaped.width))
                }
                _ => None,
            })
            .expect("preedit text layout should be emitted");
        let cursor_x = match view.query(PluginQuery::CursorScreenPos(source.len()), &doc) {
            PluginResponse::CursorScreenRect(Some((x, _, _, _))) => x,
            other => panic!("expected preedit cursor rect, got {other:?}"),
        };

        assert!(
            (cursor_x - (preedit_x + preedit_width)).abs() < 0.01,
            "preedit cursor must use the shaped text advance: cursor={cursor_x}, \
             text_end={}",
            preedit_x + preedit_width
        );
    }

    #[test]
    fn trailing_empty_paragraph_after_heading_uses_body_metrics() {
        let source = "# Heading\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.into(), 1);
        view.engine.handle_set_cursor_byte(source.len());
        render_editor_once(&mut view, &doc);

        let lazy = view.engine.lazy.as_ref().expect("layout should be built");
        let source_line = source_line_at_byte(source, source.len())
            .expect("document end should resolve to the trailing empty source line");
        let (_, _, font_size, line_height) = view
            .engine
            .empty_source_line_metrics(source_line, lazy, source)
            .expect("editable source line must have an explicit layout row");

        assert_eq!(font_size, view.engine.rendered_body_font_size);
        assert_eq!(line_height, view.engine.rendered_line_height);
    }

    #[test]
    fn editor_preedit_clear_restores_source_text() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        view.handle_message(
            PluginMessage::SetPreedit { text: String::new(), cursor: None },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        assert_eq!(lines.first().map(|line| line.text.as_str()), Some("hello world"));
    }

    /// 段落末尾输入空格后,光标应向前推进一个空格宽度,即使 pulldown-cmark
    /// 剥离了尾部空白。
    #[test]
    fn editor_cursor_advances_past_trailing_source_space() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let mut view = MarkdownEditorView::new();
        let mut doc = StubDoc::new("abc");
        view.set_source("abc".into(), 1);
        render_editor_once(&mut view, &doc);
        view.handle_message(PluginMessage::SetCursorByte(3), &mut doc);
        let before_space =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        doc = StubDoc::new("abc ");
        view.set_source("abc ".into(), 2);
        view.handle_message(PluginMessage::SetCursorByte(4), &mut doc);
        let after_draw_list = render_editor_draw_list(&mut view, &doc);
        let after_space = editor_cursor_rect_from_draw_list(&after_draw_list);

        assert!(
            (after_space.y - before_space.y).abs() < 0.5,
            "cursor after trailing space must stay on the same line; before_y={}, after_y={}",
            before_space.y,
            after_space.y,
        );
        assert!(
            after_space.x > before_space.x + 1.0,
            "cursor after trailing space must advance visibly; before_x={}, after_x={}",
            before_space.x,
            after_space.x,
        );
    }

    /// 段落末尾输入 IME 时,光标应留在同一 flat_line,而非跳到下一段落。
    #[test]
    fn editor_preedit_cursor_at_paragraph_end_stays_on_same_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello\n\nworld";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        let cursor_byte = "hello".len();
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        let base_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        for (preedit, cursor) in [("n", (1, 1)), ("ni", (2, 2)), ("nih", (3, 3)), ("nihao", (5, 5))]
        {
            view.handle_message(
                PluginMessage::SetPreedit { text: preedit.into(), cursor: Some(cursor) },
                &mut doc,
            );
            let preedit_rect =
                editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

            assert!(
                (preedit_rect.y - base_rect.y).abs() < 0.5,
                "preedit={preedit:?} cursor must stay on same y; base_y={}, preedit_y={}",
                base_rect.y,
                preedit_rect.y
            );
            assert!(
                preedit_rect.x > base_rect.x,
                "preedit={preedit:?} cursor must advance from base; base_x={}, preedit_x={}",
                base_rect.x,
                preedit_rect.x
            );
        }
    }

    #[test]
    fn editor_selection_range_query_returns_source_bytes() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(6)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(11)), &mut doc);

        let response = view.query(PluginQuery::SelectionRange, &doc);
        match response {
            PluginResponse::PositionPair(Some(((start, _), (end, _)))) => {
                assert_eq!((start, end), (6, 11));
            }
            other => panic!("expected source byte selection range, got {other:?}"),
        }
    }

    #[test]
    fn selection_intersecting_ascii_diagram_rebuilds_code_block_without_grid() {
        let source = "```\n┌────┐\n│中文│\n└────┘\n```\noutside";
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &document);

        let selection_start = source.find("中文").expect("fixture has diagram text");
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(selection_start)), &mut document);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut document);
        render_editor_once(&mut view, &document);

        let lazy = view.engine.lazy.as_ref().expect("selection render must build layout");
        let code_block = lazy.laid_out[0].as_ref().expect("code block must remain materialized");
        let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(
            lazy.ascii_diagrams().diagram_for(lines).is_none(),
            "selection must disable the diagram grid path"
        );
    }

    #[test]
    fn reverse_selection_intersecting_ascii_diagram_rebuilds_code_block_without_grid() {
        let source = "```\n┌────┐\n│中文│\n└────┘\n```\noutside";
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &document);

        let selection_end = source.find("中文").expect("fixture has diagram text");
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len())), &mut document);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(selection_end)), &mut document);
        render_editor_once(&mut view, &document);

        let lazy = view.engine.lazy.as_ref().expect("selection render must build layout");
        let code_block = lazy.laid_out[0].as_ref().expect("code block must remain materialized");
        let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(
            lazy.ascii_diagrams().diagram_for(lines).is_none(),
            "reverse selection must disable the diagram grid path"
        );
    }

    #[test]
    fn selection_change_reuses_existing_layout_and_disables_intersecting_diagram_grid() {
        use std::cell::Cell;

        let source = "```\n┌────┐\n│中文│\n└────┘\n```\n\nunchanged paragraph";
        let document = StubDoc::new(source);
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let style = crate::test_utils::default_style();
        let build_count = Cell::new(0usize);
        let mut engine = PreviewEngine::<crate::builder::MarkdownDoc>::new();
        engine.set_edit_source(Some(source.to_owned()));
        let mut shaper = shaping::Shaper::new().expect("selection test requires a shaper");

        engine.render(
            &theme,
            800.0,
            600.0,
            0.0,
            0.0,
            &style,
            |style| {
                build_count.set(build_count.get() + 1);
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, style)
            },
            Some(&mut shaper),
            &document,
            true,
            false,
        );

        let selection_start = source.find("中文").expect("fixture has diagram text");
        engine.set_sel_anchor_byte(Some(selection_start));
        engine.set_sel_cursor_byte(Some(source.len()));
        engine.render(
            &theme,
            800.0,
            600.0,
            0.0,
            0.0,
            &style,
            |style| {
                build_count.set(build_count.get() + 1);
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, style)
            },
            Some(&mut shaper),
            &document,
            true,
            false,
        );

        let lazy = engine.lazy.as_ref().expect("selection render must retain the lazy layout");
        let code_block = lazy.laid_out[0].as_ref().expect("code block must remain materialized");
        let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(lazy.ascii_diagrams().diagram_for(lines).is_none());
        assert_eq!(build_count.get(), 1, "selection updates must not rebuild MarkdownDoc");
    }

    #[test]
    fn entering_editor_keeps_softbreak_paragraph_layout() {
        use ui::plugin::PluginMessage;

        let source = "\
你这个分数，别冲体育统考本科.

一. 定向军士:冲，不押宝.只填愿意去的军士院校和专业；优先技术型、军工/航空/信息/船舶/交通背景强的院校
二. 定体育高职高专批: 主线.\t
主报体育专科里“能考证、能升本、能接就业”的专业；学校上优先湖体职、武体科院专科组、荆州职院、黄冈职院这类稳妥组合，专业上优先体康、体教、体育运营，调剂尽量服从，专升本从进校第一天就开始准备。
体育保健与康复 / 运动健康指导 > 体育教育 > 体育运营与管理 / 健身指导与管理 / 社会体育 > 运动训练。
原因很简单：前两类要么接健康产业，要么接教师资格与校园岗位，路径更稳；后两类更吃城市、平台和个人能力；单纯运动训练则更吃资源和项目背景。
三. 高职高专普通批 : 兜底备选\t
若体育和军士都不理想，再考虑就业型普通专科，如护理康复、机电、轨道、计算机应用、汽车检测等

定向军士
如果考生真想走军士，我建议提前批这样排：
前 3–5 个：好学校好专业冲一下
长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。
中间 5–8 个：本省或相邻省份务实型军士院校
湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。

体育专科
体育高职高专批：主线兜底
前 3–4 个冲：武汉文理、荆州理工、湖北幼专、鄂州/咸宁等。
中间 8–10 个稳：湖北体育职业学院、武汉体育学院体育科技学院、黄冈职院、荆州职院等。
最后 6–8 个保：三峡旅游职院及其他往年线更低、计划数较多的体育专科组。";
        let width = 420.0;

        let mut preview = MarkdownView::new();
        preview.set_source(source.into(), 1);
        render_preview_narrow(&mut preview, width);

        let doc = StubDoc::new(source);
        let mut editor = MarkdownEditorView::new();
        editor.set_source(source.into(), 1);
        render_editor_narrow(&mut editor, &doc, width);

        assert_eq!(
            visible_line_snapshots(editor.engine()),
            visible_line_snapshots(preview.engine()),
            "clicking into WYSIWYG editor must not reflow ordinary softbreak paragraphs"
        );

        let before_click = visible_line_snapshots(editor.engine());
        for line_number in [3, 6, 8, 12, 16, 19, 23] {
            let byte_offset = byte_offset_for_line(source, line_number);
            editor.handle_message(
                PluginMessage::SetCursorByte(byte_offset),
                &mut StubDoc::new(source),
            );
            render_editor_narrow(&mut editor, &doc, width);
            assert_eq!(
                visible_line_snapshots(editor.engine()),
                before_click,
                "placing the cursor on physical line {line_number} must not reflow softbreak paragraphs"
            );
        }
    }

    #[test]
    fn first_cursor_focus_keeps_softbreak_layout_in_short_viewport() {
        use ui::plugin::PluginMessage;

        let source = "\
你这个分数，别冲体育统考本科.

一. 定向军士:冲，不押宝.只填愿意去的军士院校和专业；优先技术型、军工/航空/信息/船舶/交通背景强的院校
二. 定体育高职高专批: 主线.\t
主报体育专科里“能考证、能升本、能接就业”的专业；学校上优先湖体职、武体科院专科组、荆州职院、黄冈职院这类稳妥组合，专业上优先体康、体教、体育运营，调剂尽量服从，专升本从进校第一天就开始准备。
体育保健与康复 / 运动健康指导 > 体育教育 > 体育运营与管理 / 健身指导与管理 / 社会体育 > 运动训练。
原因很简单：前两类要么接健康产业，要么接教师资格与校园岗位，路径更稳；后两类更吃城市、平台和个人能力；单纯运动训练则更吃资源和项目背景。
三. 高职高专普通批 : 兜底备选\t
若体育和军士都不理想，再考虑就业型普通专科，如护理康复、机电、轨道、计算机应用、汽车检测等

定向军士
如果考生真想走军士，我建议提前批这样排：
前 3–5 个：好学校好专业冲一下
长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。
中间 5–8 个：本省或相邻省份务实型军士院校
湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。

体育专科
体育高职高专批：主线兜底
前 3–4 个冲：武汉文理、荆州理工、湖北幼专、鄂州/咸宁等。
中间 8–10 个稳：湖北体育职业学院、武汉体育学院体育科技学院、黄冈职院、荆州职院等。
最后 6–8 个保：三峡旅游职院及其他往年线更低、计划数较多的体育专科组。";
        let width = 420.0;
        let height = 260.0;
        let doc = StubDoc::new(source);

        for line_number in [3, 6, 8, 12, 16, 19, 23] {
            let mut editor = MarkdownEditorView::new();
            editor.set_source(source.into(), 1);
            render_editor_viewport(&mut editor, &doc, width, height);
            let before_focus = layout_snapshot(editor.engine());

            let byte_offset = byte_offset_for_line(source, line_number);
            editor.handle_message(
                PluginMessage::SetCursorByte(byte_offset),
                &mut StubDoc::new(source),
            );
            render_editor_viewport(&mut editor, &doc, width, height);

            assert_eq!(
                layout_snapshot(editor.engine()),
                before_focus,
                "first cursor focus on physical line {line_number} must not reflow softbreak layout"
            );
        }
    }

    #[test]
    fn editor_render_expands_cursor_span_source_markers() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut doc = StubDoc::new("hello **world** here");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        let joined = lines.into_iter().map(|line| line.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("hello **world** here"),
            "cursor inside bold span must materialize markdown markers, got {joined:?}"
        );
    }

    #[test]
    fn hit_test_byte_roundtrip_inside_cjk_bold_span() {
        let mut view = make_view("前缀 **世界** 后缀");
        view.engine_mut().handle_set_cursor_byte("前缀 **世".len());

        let (cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor should resolve");
        let result = view.engine().hit_test_byte(cursor_x, cursor_y + cursor_h * 0.5, 0.0, 0.0);

        assert_eq!(
            result,
            Some("前缀 **世".len()),
            "CJK cursor screen position must hit-test back to the same source byte"
        );
    }

    #[test]
    fn cursor_move_does_not_require_source_generation_change() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut doc = StubDoc::new("hello **world** here");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 7);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::NeedsSourceUpdate(7), &doc);
        assert!(matches!(response, PluginResponse::Bool(false)));
    }

    // ── Two-phase hit test ─────────────────────────────────────────────────

    /// Simulates the two-phase WYSIWYG hit test flow:
    /// 1. Hit-test on folded layout (cursor outside bold span) → candidate byte
    /// 2. SetCursorByte(candidate) + render → expand inline spans
    /// 3. Hit-test again on expanded layout → final byte
    ///
    /// Verifies that both phases land in the correct source range and that
    /// the expanded layout shows the markdown markers.
    #[test]
    fn two_phase_hit_test_from_folded_to_expanded_span() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        // Source: "hello **world** here" (21 bytes)
        // "hello "  = bytes 0..6
        // "**"      = bytes 6..8  (markers, folded when cursor not inside)
        // "world"   = bytes 8..13 (bold text)
        // "**"      = bytes 13..15 (markers)
        // " here"   = bytes 15..21
        let source = "hello **world** here";
        let world_source_range = 8..13;

        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // ---- Build FOLDED layout (cursor at byte 0, not in bold span) ----
        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        // ---- Find pixel position of "world" inside the FOLDED layout ----
        // In the folded text "hello world here", "world" starts at visual char 6
        // (after "hello " which is 6 visual chars).
        let flat_lines = view.engine().flat_lines();
        let fl = &flat_lines[0];
        assert!(
            fl.text.contains("hello"),
            "folded flat_line text should contain 'hello', got: {:?}",
            fl.text
        );
        // "world" starts at visual index 6 in "hello world here"
        let world_vis_start = 6;
        let px = fl.rect.x + crate::layout::grapheme_x(fl, world_vis_start) + 2.0;
        let py = fl.rect.y + fl.rect.h / 2.0;

        // ---- Phase 1: hit-test on folded layout ----
        let candidate = view
            .engine()
            .hit_test_byte(px, py, 0.0, 0.0)
            .expect("Phase 1 hit test on folded layout must find a byte");

        // ---- Phase 1.5: move cursor + sync refresh (expand the span) ----
        view.handle_message(PluginMessage::SetCursorByte(candidate), &mut doc);
        render_editor_once(&mut view, &doc);

        // Verify expanded layout: markers must be visible.
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|line| line.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("hello **world** here"),
            "expanded layout must contain markdown markers after SetCursorByte + render, got: {joined:?}"
        );

        // ---- Phase 2: hit-test at cursor's visual position after expansion ----
        // After SetCursorByte + render, the cursor is at the candidate byte in
        // the expanded layout. We hit-test at the cursor's screen position to
        // get the final byte from the expanded source maps.
        let (cursor_x, cursor_y, _cw, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor screen pos must resolve after render");
        let final_byte = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_h / 2.0, 0.0, 0.0)
            .expect("Phase 2 hit test on expanded layout must return a byte");

        // Both candidate and final should be within or very near the "world" source range.
        assert!(
            candidate >= world_source_range.start && candidate <= world_source_range.end,
            "Phase 1 candidate byte {candidate} should be in 'world' source range {world_source_range:?}",
        );
        assert!(
            final_byte >= world_source_range.start && final_byte <= world_source_range.end,
            "Phase 2 final byte {final_byte} should be in 'world' source range {world_source_range:?}",
        );

        // The two phases may produce slightly different bytes at pixel boundaries,
        // but they should be very close (within 2 bytes, the width of "**").
        let diff = (final_byte as isize - candidate as isize).abs();
        assert!(
            diff <= 2,
            "two-phase hit-test: final byte {final_byte} should be close to candidate {candidate} (diff={diff})"
        );
    }

    // ── Block marker editing ───────────────────────────────────────────────

    /// When cursor enters a heading's source range, the block marker
    /// (e.g. "# ") must become visible in FlatLines.
    #[test]
    fn heading_marker_visible_when_cursor_in_heading_source_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "# Title\n\nparagraph";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // Cursor in paragraph (byte 10) → heading marker NOT visible.
        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(
            !joined.contains("# "),
            "marker should NOT be visible when cursor is in paragraph, got: {joined:?}"
        );

        // Cursor at byte 1 (inside the heading prefix) → marker visible.
        view.handle_message(PluginMessage::SetCursorByte(1), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines after cursor move, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("# "),
            "marker '# ' must be visible when cursor is inside heading, got: {joined:?}"
        );
    }

    /// When cursor enters a blockquote's source range, the marker "> " must
    /// become visible in FlatLines.
    #[test]
    fn blockquote_marker_visible_when_cursor_in_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "> quoted";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // Cursor at byte 3 (inside content "quoted") → marker visible.
        view.handle_message(PluginMessage::SetCursorByte(3), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("> "), "blockquote marker '> ' must be visible, got: {joined:?}");
    }

    /// Empty documents contain one editable paragraph with zero graphemes.
    #[test]
    fn empty_document_has_one_editable_layout_row() {
        use ui::plugin::{PluginQuery, PluginResponse, ViewPlugin};

        let doc = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.set_source(String::new(), 1);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        assert_eq!(lines.len(), 1, "empty document must lay out its editable paragraph");
        assert!(lines[0].text.is_empty());
    }

    /// Very long line: FlatLines still produces output without panic.
    #[test]
    fn very_long_line_does_not_panic() {
        use ui::plugin::{PluginQuery, PluginResponse, ViewPlugin};

        let long = "x".repeat(10_000);
        let doc = StubDoc::new(&long);
        let mut view = MarkdownEditorView::new();
        view.set_source(long, 1);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        assert!(matches!(response, PluginResponse::FlatLines(_)));
    }

    /// H2 heading marker "## " is visible.
    #[test]
    fn h2_marker_visible_when_cursor_in_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "## SubTitle";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(1), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("## "), "h2 marker must be visible, got: {joined:?}");
    }

    #[test]
    fn heading_content_click_keeps_prefix_hidden_and_caret_stable() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "## SubTitle\n\nparagraph";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        render_editor_once(&mut view, &doc);

        let folded_line = view.engine().flat_lines()[0].clone();
        assert!(
            !folded_line.text.starts_with("## "),
            "heading marker should start folded while cursor is outside, got {:?}",
            folded_line.text
        );

        let clicked_visual_grapheme = 2;
        let px =
            folded_line.rect.x + crate::layout::grapheme_x(&folded_line, clicked_visual_grapheme);
        let py = folded_line.rect.y + folded_line.rect.h * 0.5;
        let candidate = view
            .engine()
            .hit_test_byte(px, py, 0.0, 0.0)
            .expect("folded heading hit-test should return a source byte");

        view.handle_message(PluginMessage::SetCursorByte(candidate), &mut doc);
        render_editor_once(&mut view, &doc);

        let expanded_line = view.engine().flat_lines()[0].clone();
        assert!(
            expanded_line.text == folded_line.text,
            "heading prefix should stay hidden after clicking content, got {:?}",
            expanded_line.text
        );

        let stale_mouse_hit = view
            .engine()
            .hit_test_byte(px, py, 0.0, 0.0)
            .expect("expanded heading stale hit-test should still hit the line");
        assert_eq!(
            stale_mouse_hit, candidate,
            "clicking heading content must keep the original source position"
        );

        let (cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("expanded cursor rect should resolve");
        let rect_hit = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_h * 0.5, 0.0, 0.0)
            .expect("cursor-rect hit-test should return a source byte");

        assert_eq!(
            rect_hit, candidate,
            "second phase should preserve the folded hit byte by using the expanded cursor rect"
        );
    }

    /// H3 heading marker "### " is visible.
    #[test]
    fn h3_marker_visible_when_cursor_in_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "### Small";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(1), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("### "), "h3 marker must be visible, got: {joined:?}");
    }

    /// Task list checked marker "- [x] " is visible.
    #[test]
    fn task_list_checked_marker_visible() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "- [x] done";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(8), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("- [x] "), "checked task marker must be visible, got: {joined:?}");
    }

    #[test]
    fn activating_ordered_list_replaces_preview_number_with_source_marker() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        for number in [0, 1, 7, 42] {
            let source = format!("# Title\n\n{number}. item");
            let preview_marker = format!("{number}.");
            let source_marker = format!("{number}. ");
            let item_cursor = source.find("item").expect("fixture must contain list item text");
            let mut document = StubDoc::new(&source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(2), &mut document);

            let preview_draw_list = render_editor_draw_list(&mut view, &document);
            let preview_text = rendered_text_commands(&preview_draw_list);
            assert!(preview_text.contains(&preview_marker.as_str()), "source: {source:?}");
            assert!(!preview_text.contains(&source_marker.as_str()), "source: {source:?}");

            view.handle_message(PluginMessage::SetCursorByte(item_cursor), &mut document);
            let draw_list = render_editor_draw_list(&mut view, &document);
            let rendered_text = rendered_text_commands(&draw_list);
            assert!(rendered_text.contains(&source_marker.as_str()), "source: {source:?}");
            assert!(!rendered_text.contains(&preview_marker.as_str()), "source: {source:?}");

            view.handle_message(PluginMessage::SetCursorByte(2), &mut document);
            let draw_list = render_editor_draw_list(&mut view, &document);
            let rendered_text = rendered_text_commands(&draw_list);
            assert!(rendered_text.contains(&preview_marker.as_str()), "source: {source:?}");
            assert!(!rendered_text.contains(&source_marker.as_str()), "source: {source:?}");
            assert_eq!(document.text, source, "changing cursor must preserve the source");
        }
    }

    #[test]
    fn removing_middle_ordered_marker_recomputes_the_following_preview_number() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let initial_source = "1. first\n2. second\n3. third";
        let mut document = StubDoc::new(initial_source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        let initial_cursor =
            initial_source.find("second").expect("fixture must contain the middle item");
        view.handle_message(PluginMessage::SetCursorByte(initial_cursor), &mut document);
        render_editor_once(&mut view, &document);

        let updated_source = "1. first\nsecond\n3. third";
        document = StubDoc::new(updated_source);
        view.set_source(document.text.clone(), 2);
        let updated_cursor =
            updated_source.find("second").expect("fixture must contain the former middle item");
        view.handle_message(PluginMessage::SetCursorByte(updated_cursor), &mut document);

        let draw_list = render_editor_draw_list(&mut view, &document);
        let rendered_text = rendered_text_commands(&draw_list);

        assert!(
            rendered_text.contains(&"2."),
            "following item must be renumbered after removing the middle marker: {rendered_text:?}"
        );
        assert!(
            !rendered_text.contains(&"3."),
            "stale source number must not replace the recomputed preview number: {rendered_text:?}"
        );
    }

    /// HitTestByte returns None for a click far outside all content.
    #[test]
    fn hit_test_byte_returns_none_far_outside_content() {
        let v = make_view("- item");
        // Click at (99999, 99999) — way outside the rendered content.
        let result = v.engine().hit_test_byte(99999.0, 99999.0, 0.0, 0.0);
        assert!(result.is_none(), "HitTestByte far outside content should return None");
    }
    #[test]
    fn hit_test_byte_after_consecutive_empty_lines_stays_on_following_text() {
        let source = "\
### 真人感方面有做什么?
anchor 人物性格,背景描述,提示词来决定. 对他自己的了解和对你的了解

2 个


viebcoding 用过吗?

##  吴志全  
";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let target_start = source.find("viebcoding").expect("fixture should contain target line");
        let target_end = source.len();
        let target_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("viebcoding"))
            .expect("rendered flat lines should contain target line");

        for y_ratio in [0.1, 0.5, 0.9] {
            let hit = view
                .engine()
                .hit_test_byte(
                    target_line.rect.x + 1.0,
                    target_line.rect.y + target_line.rect.h * y_ratio,
                    0.0,
                    0.0,
                )
                .expect("click on target line should return a byte");

            assert!(
                (target_start..=target_end).contains(&hit),
                "click on target text at y ratio {y_ratio} mapped to byte {hit}, \
                 expected {target_start}..={target_end}"
            );
        }
    }

    #[test]
    fn hit_test_byte_on_visible_empty_line_after_heading_activates_empty_line() {
        let source = "\
## 李现民


对 AI Agent 产品形态有实际演进经验，对记忆、上下文和幻觉控制有较多工程实践";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let editable_empty_line_byte = "## 李现民\n\n".len();
        view.handle_message(PluginMessage::SetCursorByte(editable_empty_line_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let (cursor_x, cursor_y, cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("visible empty source line should have a cursor rect");
        let hit = view.engine().hit_test_byte(
            cursor_x + cursor_width * 0.5,
            cursor_y + cursor_height * 0.5,
            0.0,
            0.0,
        );

        assert_eq!(
            hit,
            Some(editable_empty_line_byte),
            "clicking the visible empty line should activate its source byte"
        );
    }

    #[test]
    fn hit_test_byte_in_hidden_block_separator_snaps_nearby_text() {
        let source = "first\n\nsecond";
        let view = make_view(source);
        let first_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("first"))
            .expect("first paragraph should be laid out");
        let second_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("second"))
            .expect("second paragraph should be laid out");
        let separator_mid_y = (first_line.rect.y + first_line.rect.h + second_line.rect.y) * 0.5;

        let hit = view.engine().hit_test_byte(first_line.rect.x, separator_mid_y, 0.0, 0.0);

        assert!(
            hit.is_some(),
            "clicking the hidden block separator gap should snap to nearby text, not return None"
        );
        assert_ne!(hit, Some(0), "gap click must not degrade to document start");
    }

    #[test]
    fn whitespace_and_crlf_blank_lines_keep_empty_line_cursor_and_navigation() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        for (source, blank_byte) in [("first\n \n", 6), ("first\r\n\r\n", 7)] {
            let mut doc = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.to_owned(), 1);
            view.handle_message(PluginMessage::SetCursorByte(blank_byte), &mut doc);
            render_editor_once(&mut view, &doc);

            assert!(view.engine().cursor_screen_pos().is_some());
            assert_eq!(
                view.engine().visual_move(blank_byte, MoveDirection::LineStart, None),
                Some(blank_byte),
            );
        }
    }

    #[test]
    fn whitespace_only_block_separator_is_not_clickable_as_editable_line() {
        let source = "first\n \nsecond";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_once(&mut view, &doc);
        let first = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("first"))
            .expect("first paragraph must be rendered");
        let second = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("second"))
            .expect("second paragraph must be rendered");
        let separator_y = (first.rect.y + first.rect.h + second.rect.y) * 0.5;

        assert_ne!(view.engine().hit_test_byte(first.rect.x, separator_y, 0.0, 0.0), Some(6),);
    }

    #[test]
    fn visual_move_down_from_empty_source_line_goes_to_next_line_start() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Down, None)
            .expect("Down from empty line should return a byte");

        assert_eq!(moved, 7, "Down from empty line should land at next source line start");
    }

    #[test]
    fn visual_move_up_from_empty_source_line_goes_to_previous_line_start() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Up, None)
            .expect("Up from empty line should return a byte");

        assert_eq!(moved, 0, "Up from empty line should land at previous source line start");
    }

    #[test]
    fn visual_move_from_second_editable_empty_line_reaches_adjacent_rendered_lines() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let second_empty_line_start = "first\n\n".len();
        let second_line_start = source.find("second").expect("fixture should contain second line");
        view.handle_message(PluginMessage::SetCursorByte(second_empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(second_empty_line_start, MoveDirection::Up, None),
            Some(0),
            "Up from the second editable empty line should reach the previous rendered line"
        );
        assert_eq!(
            view.engine().visual_move(second_empty_line_start, MoveDirection::Down, None),
            Some(second_line_start),
            "Down from the second editable empty line should reach the next rendered line"
        );
    }

    #[test]
    fn visual_move_up_from_trailing_editable_empty_line_reaches_previous_rendered_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let trailing_empty_line_start = "first\n".len();
        view.handle_message(PluginMessage::SetCursorByte(trailing_empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(trailing_empty_line_start, MoveDirection::Up, None),
            Some(0),
            "Up from a trailing editable empty line should reach the previous rendered line"
        );
    }

    #[test]
    fn visual_move_right_skips_inter_paragraph_empty_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_line_end = source.find('\n').expect("fixture should contain blank line");
        view.handle_message(PluginMessage::SetCursorByte(first_line_end), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(first_line_end, MoveDirection::Right, None)
            .expect("Right from first line should return a byte");

        assert_eq!(moved, 7, "Right should skip hidden paragraph separator lines");
    }

    #[test]
    fn visual_move_left_skips_inter_paragraph_empty_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let second_line_start = source.find("second").expect("fixture should contain second line");
        view.handle_message(PluginMessage::SetCursorByte(second_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(second_line_start, MoveDirection::Left, None)
            .expect("Left from second line should return a byte");

        assert_eq!(moved, 5, "Left should skip hidden paragraph separator lines");
    }

    #[test]
    fn visual_move_right_from_empty_source_line_goes_to_next_line_start() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Right, None)
            .expect("Right from empty line should return a byte");

        assert_eq!(moved, 7, "Right from empty line should land at next source line start");
    }

    #[test]
    fn visual_move_left_from_empty_source_line_goes_to_previous_line_end() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Left, None)
            .expect("Left from empty line should return a byte");

        assert_eq!(moved, 5, "Left from empty line should land at previous source line end");
    }

    #[test]
    fn visual_move_line_start_and_end_preserve_empty_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(empty_line_start, MoveDirection::LineStart, None),
            Some(empty_line_start),
            "Home on an empty source line should stay on that line"
        );
        assert_eq!(
            view.engine().visual_move(empty_line_start, MoveDirection::LineEnd, None),
            Some(empty_line_start),
            "End on an empty source line should stay on that line"
        );
    }

    #[test]
    fn cursor_on_blank_line_inside_code_block_stays_on_that_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "```rust\nfn a() {}\n\nfn b() {}\n```\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        view.handle_message(PluginMessage::SetCursorByte(blank_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let (cursor_x, cursor_y, cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("blank line inside a code block should have a cursor rect");
        let hit = view.engine().hit_test_byte(
            cursor_x + cursor_width * 0.5,
            cursor_y + cursor_height * 0.5,
            0.0,
            0.0,
        );

        assert_eq!(
            hit,
            Some(blank_line_start),
            "cursor on the blank line inside a code block must map back to that line's byte"
        );
    }

    #[test]
    fn visual_move_down_passes_through_blank_line_inside_active_fenced_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "```rust\nfn a() {}\n\nfn b() {}\n```\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_code_line_start = source.find("fn a() {}").expect("fixture has first code line");
        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        let second_code_line_start =
            source.find("fn b() {}").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(first_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_code_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down from a code line should land on the blank line inside the same code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_code_line_start),
            "Down from the blank line inside a code block should reach the next code line"
        );
    }

    #[test]
    fn visual_move_up_passes_through_blank_line_inside_active_fenced_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "```rust\nfn a() {}\n\nfn b() {}\n```\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_code_line_start = source.find("fn a() {}").expect("fixture has first code line");
        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        let second_code_line_start =
            source.find("fn b() {}").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(second_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(second_code_line_start, MoveDirection::Up, None),
            Some(blank_line_start),
            "Up from a code line should land on the blank line inside the same code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_code_line_start),
            "Up from the blank line inside a code block should reach the previous code line"
        );
    }

    #[test]
    fn visual_move_passes_through_blank_line_inside_active_indented_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "intro\n\n    first\n\n    second\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // 缩进代码块的块范围从首个文本字节开始（不含首行缩进）。
        let first_code_line_start = source.find("first").expect("fixture has first code line");
        let blank_line_start =
            source.find("first\n\n").expect("fixture has blank line") + "first\n".len();
        let second_code_line_start = source.find("second").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(first_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_code_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down should land on the blank line inside the same indented code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_code_line_start),
            "Down from the blank line should reach the next indented code line"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_code_line_start),
            "Up from the blank line should reach the previous indented code line"
        );
    }

    #[test]
    fn visual_move_passes_through_blank_line_inside_crlf_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "```\r\nalpha\r\n\r\nbeta\r\n```\r\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_code_line_start = source.find("alpha").expect("fixture has first code line");
        let blank_line_start =
            source.find("\r\n\r\n").expect("fixture has blank line") + "\r\n".len();
        let second_code_line_start = source.find("beta").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(first_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_code_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down should land on the blank line inside the same CRLF code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_code_line_start),
            "Down from the CRLF blank line should reach the next code line"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_code_line_start),
            "Up from the CRLF blank line should reach the previous code line"
        );
    }

    #[test]
    fn visual_move_passes_through_blank_line_inside_metadata_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "---\ntitle: a\n\ndesc: b\n---\nbody\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_line_start = source.find("title: a").expect("fixture has first metadata line");
        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        let second_line_start = source.find("desc: b").expect("fixture has second metadata line");
        view.handle_message(PluginMessage::SetCursorByte(first_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down should land on the blank line inside the same metadata block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_line_start),
            "Down from the blank line should reach the next metadata line"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_line_start),
            "Up from the blank line should reach the previous metadata line"
        );
    }

    #[test]
    fn wysiwyg_cursor_y_aligns_to_text_baseline() {
        let mut doc = StubDoc::new("hello markdown");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let text_baseline = draw_list
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                ui::core::paint::DrawCmd::TextLayout { y_baseline, layout, .. }
                    if layout.text.contains("hello markdown") =>
                {
                    Some(*y_baseline)
                }
                _ => None,
            })
            .expect("rendered markdown text must expose its baseline");

        let (_cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor should resolve after render");
        let expected_top = text_baseline - cursor_h * WYSIWYG_CURSOR_ASCENT_RATIO;

        assert!(
            (cursor_y - expected_top).abs() < 0.01,
            "cursor top {cursor_y} should align to text baseline {text_baseline} with height {cursor_h}, expected {expected_top}"
        );
    }

    // ── Grapheme hit-test roundtrip ─────────────────────────────────────────

    #[test]
    fn wysiwyg_hit_test_roundtrips_combining_grapheme() {
        // byte 2 is the start of 'e' (correct grapheme boundary).
        // byte 3 is inside the combining acute, so it snaps to grapheme start byte 2.
        let mut view = make_view("**e\u{0301}**");
        view.engine_mut().handle_set_cursor_byte(2);

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

        assert_eq!(hit, Some(2));
    }

    #[test]
    fn wysiwyg_hit_test_roundtrips_zwj_emoji() {
        // byte 2 is the start of the ZWJ emoji (on a grapheme boundary).
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let source = format!("**{emoji}**");
        let target_byte = 2usize;
        let mut view = make_view(&source);
        view.engine_mut().handle_set_cursor_byte(target_byte);

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

        assert_eq!(hit, Some(target_byte));
    }

    #[test]
    fn hit_test_byte_roundtrip_inside_list_item_respects_indent() {
        // "- item": bytes 0-1 are "- ", byte 2 start of "i".
        // List items have flat_line.rect.x > 0 (indent).
        let mut view = make_view("- item");
        view.engine_mut().handle_set_cursor_byte(2);

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        assert!(x > 0.0, "cursor x inside list item should include indent offset, got {x}");

        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);
        assert_eq!(hit, Some(2));
    }

    #[test]
    fn active_code_block_cursor_uses_shaped_code_font_geometry() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "```text\nabcdefghij\n```";
        let code_start = source.find("abcdefghij").expect("fixture must contain code text");
        let cursor_byte = code_start + 8;
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut document);
        render_editor_once(&mut view, &document);

        let visual_position = view
            .engine()
            .cursor_visual_position_for_byte(cursor_byte, CursorAffinity::Downstream)
            .expect("active code byte must have a visual position");
        let flat_line_idx = view
            .engine()
            .lazy
            .as_ref()
            .and_then(|lazy| lazy.flat_line_idx_for_projection(visual_position.flat_line_idx))
            .expect("active code projection must map to a flat line");
        let flat_line = &view.engine().flat_lines()[flat_line_idx];
        let shaped = flat_line.shaped.as_ref().expect("active code line must retain shaping");
        let local_byte = cursor_byte - code_start;
        let expected_advance: f32 = shaped
            .clusters
            .iter()
            .take_while(|cluster| cluster.byte_range.start < local_byte)
            .map(|cluster| cluster.advance)
            .sum();
        let (cursor_x, cursor_y, _cursor_width, cursor_height) =
            view.engine().cursor_screen_pos().expect("active code cursor must resolve");

        assert!(
            (cursor_x - (flat_line.rect.x + expected_advance)).abs() < 0.01,
            "cursor x {cursor_x} must use shaped code advance {expected_advance}"
        );
        assert_eq!(
            view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0,),
            Some(cursor_byte),
            "hit-testing at the shaped caret boundary must return the same source byte"
        );
    }

    #[test]
    fn hit_test_byte_roundtrips_table_middle_and_right_cells() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "| left | middle | right |\n| --- | --- | --- |\n| left cell | middle cell | right cell |";
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);

        for cell_text in ["middle cell", "right cell"] {
            let cell_byte = source.find(cell_text).expect("fixture must contain table cell text");
            view.handle_message(PluginMessage::SetCursorByte(cell_byte), &mut document);
            render_editor_once(&mut view, &document);

            let (cursor_x, cursor_y, _cursor_width, cursor_height) =
                view.engine().cursor_screen_pos().expect("table cell must have a cursor rect");
            let cell_line = view
                .engine()
                .flat_lines()
                .iter()
                .find(|line| {
                    line.source_projection.as_ref().is_some_and(|projection| {
                        projection.boundaries.iter().any(|anchor| anchor.byte == cell_byte)
                    })
                })
                .expect("table cell source byte must own a rendered line");
            assert!(
                cursor_x >= cell_line.rect.x && cursor_x <= cell_line.rect.x + cell_line.rect.w,
                "cursor x {cursor_x} must fall inside its table cell rect {:?}",
                cell_line.rect,
            );

            let hit =
                view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0);
            assert_eq!(
                hit,
                Some(cell_byte),
                "table cell {cell_text:?} must hit-test to its own source byte",
            );
        }
    }

    #[test]
    fn markdown_editor_hit_test_edit_target_exposes_table_cell_caret() {
        use ui::plugin::{EditHitTarget, PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "| first | second |\n| --- | --- |\n| left | right |";
        let target_byte = source.find("right").expect("fixture has a second-column cell");
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(target_byte), &mut document);
        render_editor_once(&mut view, &document);

        let (cursor_x, cursor_y, _, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("rendered second-column cell should expose its caret rect");
        let response = view.query(
            PluginQuery::HitTestEditTarget {
                x: cursor_x,
                y: cursor_y + cursor_height * 0.5,
                offset_x: 0.0,
                offset_y: 0.0,
            },
            &document,
        );

        let PluginResponse::EditHitTarget(Some(EditHitTarget::TextCaret {
            byte_offset,
            selection_scope,
        })) = response
        else {
            panic!("Markdown table hit-test should return a text caret target");
        };
        assert_eq!(byte_offset, target_byte);
        assert_eq!(selection_scope, None);
    }

    #[test]
    fn hit_test_byte_roundtrips_trailing_stripped_whitespace() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let plain_source = "plain trailing \t";
        let table_source = "| left | right |\n| --- | --- |\n| left cell | right cell \t|";
        let table_trailing_whitespace =
            table_source.find("\t|").expect("fixture must contain trailing table tab") + 1;

        for (source, cursor_byte) in
            [(plain_source, plain_source.len()), (table_source, table_trailing_whitespace)]
        {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut document);
            render_editor_once(&mut view, &document);

            let expected = canonical_source_byte(&view, cursor_byte);
            let hit = hit_test_source_byte_at_cursor(&view, cursor_byte);
            assert_eq!(
                hit, expected,
                "source={source:?}, cursor={cursor_byte} must preserve its canonical anchor",
            );
        }
    }

    fn source_grapheme_boundaries(source: &str) -> Vec<usize> {
        let grapheme_count = crate::grapheme_map::grapheme_count(source);
        (0..=grapheme_count)
            .map(|grapheme_index| {
                crate::grapheme_map::byte_at_grapheme_index(source, grapheme_index)
            })
            .collect()
    }

    fn canonical_source_byte(view: &MarkdownEditorView, source_byte: usize) -> usize {
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(source_byte, CursorAffinity::Downstream)
            .expect("every source grapheme boundary must have a canonical position");
        view.engine()
            .projection_index()
            .source_anchor_at(1, position)
            .expect("canonical position must map back to source")
            .byte
    }

    fn hit_test_source_byte_at_cursor(view: &MarkdownEditorView, source_byte: usize) -> usize {
        let (x, y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos_for_byte(source_byte)
            .expect("every canonical source position must have a cursor rect");
        view.engine()
            .hit_test_byte(x, y + cursor_height * 0.5, 0.0, 0.0)
            .expect("cursor rect center must be hittable")
    }

    #[test]
    fn projection_corpus_roundtrips_every_source_grapheme_boundary() {
        let corpus = [
            "plain paragraph",
            "# heading heading heading heading heading heading",
            "> outer\n> > **inner** — continuation",
            "- outer\n  - inner wrapped content wrapped content",
            "| left | middle | right |\n| --- | --- | --- |\n| 左侧内容 | middle content | 👨\u{200d}👩 |",
            "paragraph\n\n\nnext",
            "e\u{301} and 👨\u{200d}👩",
        ];

        for source in corpus {
            for width in [140.0, 320.0, 800.0] {
                let mut document = StubDoc::new(source);
                let mut view = MarkdownEditorView::new();
                view.set_source(document.text.clone(), 1);
                view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
                render_editor_narrow(&mut view, &document, width);

                for source_byte in source_grapheme_boundaries(source) {
                    let has_static_position = view
                        .engine()
                        .projection_index()
                        .visual_position_for_source(source_byte, CursorAffinity::Downstream);
                    let (mut expected, mut hit) = match has_static_position {
                        Some(_) => (
                            canonical_source_byte(&view, source_byte),
                            hit_test_source_byte_at_cursor(&view, source_byte),
                        ),
                        None => {
                            view.handle_message(
                                PluginMessage::SetCursorByte(source_byte),
                                &mut document,
                            );
                            render_editor_narrow(&mut view, &document, width);
                            (
                                canonical_source_byte(&view, source_byte),
                                hit_test_source_byte_at_cursor(&view, source_byte),
                            )
                        }
                    };

                    if has_static_position.is_some() && hit != expected {
                        view.handle_message(
                            PluginMessage::SetCursorByte(source_byte),
                            &mut document,
                        );
                        render_editor_narrow(&mut view, &document, width);
                        expected = canonical_source_byte(&view, source_byte);
                        hit = hit_test_source_byte_at_cursor(&view, source_byte);
                    }

                    assert_eq!(
                        hit, expected,
                        "source={source:?}, width={width}, byte={source_byte}"
                    );
                }
            }
        }
    }

    #[test]
    fn preedit_corpus_roundtrips_committed_source_anchor() {
        let cases = [
            ("nested > quote with 中文", "中", "输入"),
            (
                "| left | middle | right |\n| --- | --- | --- |\n| left cell | middle cell | right cell |",
                "middle cell",
                "中文",
            ),
        ];

        for (source, cursor_text, preedit) in cases {
            let cursor_byte = source.find(cursor_text).expect("fixture must contain cursor text");
            let document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.engine.handle_set_cursor_byte(cursor_byte);
            view.engine.set_preedit_text(preedit.to_string(), Some((preedit.len(), preedit.len())));
            render_editor_narrow(&mut view, &document, 140.0);

            let (x, y, _cursor_width, cursor_height) =
                view.engine().cursor_screen_pos().expect("preedit cursor must have a source rect");
            let hit = view.engine().hit_test_byte(x, y + cursor_height * 0.5, 0.0, 0.0);

            assert_eq!(
                hit,
                Some(cursor_byte),
                "source={source:?}, preedit={preedit:?} must retain its committed source anchor",
            );
        }
    }

    #[test]
    fn cursor_only_refresh_keeps_unaffected_block_projection_identical() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# first heading\n\nmiddle paragraph stays stable\n\n> final quote";
        let first = source.find("first").expect("fixture must contain first heading text");
        let final_quote = source.find("final").expect("fixture must contain final quote text");
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(first), &mut document);
        render_editor_narrow(&mut view, &document, 320.0);
        let before = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("middle paragraph"))
            .expect("middle paragraph must be visible")
            .source_projection
            .clone()
            .expect("middle paragraph must have a projection");

        view.handle_message(PluginMessage::SetCursorByte(final_quote), &mut document);
        render_editor_narrow(&mut view, &document, 320.0);
        let after = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("middle paragraph"))
            .expect("middle paragraph must stay visible")
            .source_projection
            .clone()
            .expect("middle paragraph must have a projection");

        assert_eq!(after, before);
    }

    #[test]
    fn hit_test_byte_roundtrip_inside_ordered_list_item_respects_marker_width() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "intro\n\n1. item";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        let folded_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "item")
            .expect("ordered list item should render folded content");
        let folded_content_x = folded_line.rect.x;
        let clicked_byte = source.find("item").expect("fixture should contain item") + 1;
        let folded_x = folded_line.rect.x + crate::layout::grapheme_x(folded_line, 1);
        let folded_y = folded_line.rect.y + folded_line.rect.h * 0.5;
        let candidate = view
            .engine()
            .hit_test_byte(folded_x, folded_y, 0.0, 0.0)
            .expect("folded ordered list hit-test should return a byte");
        assert_eq!(candidate, clicked_byte);

        view.handle_message(PluginMessage::SetCursorByte(candidate), &mut doc);
        render_editor_once(&mut view, &doc);

        let expanded_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "1. item")
            .expect("ordered list item should render expanded marker");
        let expanded_content_x =
            expanded_line.rect.x + crate::layout::grapheme_x(expanded_line, "1. ".len());
        assert!(
            (expanded_content_x - folded_content_x).abs() < 0.01,
            "ordered list body must keep its x position when source marker expands: folded={folded_content_x}, expanded={expanded_content_x}"
        );

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

        assert_eq!(hit, Some(clicked_byte));

        let shaped = expanded_line
            .shaped
            .as_ref()
            .expect("active ordered list line should keep shaped geometry");
        let prefix_len = "1. i".len();
        let prefix_width = shaped
            .clusters
            .iter()
            .take_while(|cluster| cluster.byte_range.start < prefix_len)
            .map(|cluster| cluster.advance)
            .sum::<f32>();
        let expected_x = expanded_line.rect.x + prefix_width;
        assert!(
            (x - expected_x).abs() < 0.01,
            "cursor x {x} should match shaped prefix x {expected_x} for active ordered list"
        );
    }

    #[test]
    fn active_list_collapses_lazy_continuation_softbreaks() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "- 前 3-5 个：好学校好专业冲一下\n长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。\n中间 5-8 个：本省或相邻省份务实型军士院校\n湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find("好学校").expect("fixture should contain first line text");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        let rendered_text = flat_texts.join("");
        assert!(
            rendered_text.contains("好学校好专业冲一下 长沙航空"),
            "first source newline must collapse to a space, got {flat_texts:?}"
        );
        assert!(
            rendered_text.contains("放前面。 中间 5-8 个"),
            "second source newline must collapse to a space, got {flat_texts:?}"
        );
        assert!(
            rendered_text.contains("务实型军士院校 湖北交通"),
            "third source newline must collapse to a space, got {flat_texts:?}"
        );
    }

    #[test]
    fn active_last_list_item_keeps_trailing_empty_paragraph_outside_item() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "- a\n- b\n\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find('b').expect("fixture should contain last item text");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert_eq!(flat_texts, vec!["a", "- b", ""]);
        assert_eq!(
            view.engine().flat_lines()[2].rect.x,
            0.0,
            "trailing paragraph belongs outside the list"
        );
    }

    #[test]
    fn active_list_down_follows_visual_wrap_before_physical_softbreak() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "- alpha beta gamma delta epsilon zeta eta theta iota kappa lambda\ncontinuation sentence";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let current_byte = source.find("alpha").expect("fixture should contain first line text");
        view.handle_message(PluginMessage::SetCursorByte(current_byte), &mut doc);
        render_editor_narrow(&mut view, &doc, 170.0);

        let continuation_start =
            source.find("continuation").expect("fixture should contain continuation text");
        let moved = view
            .engine()
            .visual_move(current_byte, MoveDirection::Down, None)
            .expect("Down from the first wrapped row should return a byte");

        assert!(
            current_byte < moved && moved < continuation_start,
            "Down must follow the next visual row before the physical softbreak at {continuation_start}, got {moved}"
        );
    }

    #[test]
    fn active_styled_list_collapses_lazy_continuation_softbreak() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "- 前 **好学校** 好专业冲一下\n长沙航空、南京 **信息**、江苏海事这类，有湖北计划就放前面。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let current_byte = source.find("好专业").expect("fixture should contain first line text");
        view.handle_message(PluginMessage::SetCursorByte(current_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        assert_eq!(
            flat_texts,
            vec![
                "- 前 好学校 好专业冲一下 长沙航空、南京 信息、江苏海事这类，有湖北计划就放前面。"
            ]
        );

        let next_line_start = source.find("长沙航空").expect("fixture should contain second line");
        let moved = view
            .engine()
            .visual_move(current_byte, MoveDirection::Down, None)
            .expect("Down from styled list line should return a byte");

        assert!(
            moved >= next_line_start,
            "Down should move into the second source line starting at {next_line_start}, got {moved}"
        );
    }

    #[test]
    fn visual_move_down_in_indented_soft_wrap_uses_screen_x_as_absolute_coordinate() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "- alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let current_byte = source.find("gamma").expect("fixture should contain current word");
        view.handle_message(PluginMessage::SetCursorByte(current_byte), &mut doc);
        render_editor_narrow(&mut view, &doc, 170.0);

        let flat_lines = view.engine().flat_lines();
        assert!(
            flat_lines.len() >= 3,
            "narrow list item should soft-wrap into at least 3 visual lines, got {}",
            flat_lines.len()
        );
        let (current_flat_idx, _) = view
            .engine()
            .find_flat_and_grapheme_for_byte(current_byte)
            .expect("current byte should map to a flat line");
        let target_flat_idx = current_flat_idx + 1;
        let target_line = flat_lines.get(target_flat_idx).expect("next soft-wrap line must exist");
        assert!(
            target_line.rect.x > 0.0,
            "fixture must exercise non-zero target line x, got {}",
            target_line.rect.x
        );
        let (cursor_x, _cursor_y, _cursor_w, _cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor rect should resolve");

        let expected_grapheme =
            view.engine().grapheme_at_x_for_line(target_line, cursor_x - target_line.rect.x);
        let expected_byte = view
            .engine()
            .byte_from_flat_line_and_visual_grapheme(target_flat_idx, expected_grapheme)
            .expect("expected target grapheme should map to source byte");
        let moved = view
            .engine()
            .visual_move(current_byte, MoveDirection::Down, Some(cursor_x))
            .expect("Down should move to the next soft-wrap line");

        assert_eq!(
            moved, expected_byte,
            "VisualMove target_x is in plugin coordinates and must be converted to target-line relative x"
        );
    }

    #[test]
    fn visual_move_up_from_empty_line_before_long_paragraph_returns_previous_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "\
# 版式部门 2026 OKR 战略陈述（新版）

**汇报口径：** 只回答两个问题：

1. **我的目标凭什么支撑战略？**
2. **目标的可达成性如何？**

**评审语境：** 这不是一次功能汇报，而是一次战略承接答辩。评委要集中业务判断力，判断版式 OKR 是否真正支撑公司战略、是否存在方向性问题、是否需要整改。CEO 最后需要定调：核心问题是什么、是否需要整改、方向是否认可。

---

## 0. 先给结论

版式部门 2026 OKR 的方向应当被认可。

它的核心价值不是“继续增强 PDF/OFD 工具”，而是把 PDF/OFD 这类**正式业务结果文档**，从传统阅读、编辑、转换工具，升级为 AI Office 和 WPS 365 中能够被理解、被调用、被处理、被交付、被合规归档的关键能力。

用一句话概括：

> **如果 AI Office 只能生成草稿、总结文档，却不能处理合同、公文、标书、审计报告、签章文件这些最终业务结果，那么它还没有真正进入用户的业务闭环。版式部门要补上的，正是 AI Office 通向正式业务结果的最后一公里。**

所以，版式 OKR 对公司战略的支撑关系是成立的。需要强化的不是方向，而是执行表达：要从“能力清单”进一步收敛为“正式文档处理场景闭环”和“可验证的业务结果”。

---

## 1. 汇报逻辑重构
";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let line_14_start =
            source.find("版式部门 2026 OKR 的方向").expect("fixture should contain line 14");
        let line_16_start = source.find("它的核心价值").expect("fixture should contain line 16");
        let line_26_start =
            source.find("## 1. 汇报逻辑重构").expect("fixture should contain line 26");

        view.handle_message(PluginMessage::SetCursorByte(line_16_start), &mut doc);
        render_editor_narrow(&mut view, &doc, 420.0);
        let (line_16_x, _line_16_y, _line_16_w, _line_16_h) =
            view.engine().cursor_screen_pos().expect("line 16 cursor should resolve");
        let moved_above = view
            .engine()
            .visual_move(line_16_start, MoveDirection::Up, Some(line_16_x))
            .expect("Up from line 16 should return a byte");

        assert!(
            (line_14_start..line_16_start).contains(&moved_above),
            "Up from line 16 should skip line 15 paragraph spacing and return to line 14 range {line_14_start}..{line_16_start}, got {moved_above}; line 26 starts at {line_26_start}"
        );
    }

    #[test]
    fn inactive_list_lazy_continuation_keeps_fast_collapsed_layout() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source =
            "intro\n\n- 前 3-5 个：好学校好专业冲一下\n长沙航空、南京信息、江苏海事、成都航空。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert!(
            flat_texts.iter().any(|line| line.contains("冲一下 长沙航空")),
            "inactive list should keep the parser-collapsed softbreak layout, got {flat_texts:?}"
        );
    }

    #[test]
    fn loose_list_items_do_not_render_parent_and_paragraph_twice() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "\
- 前 3-4 个冲：武汉文理、荆州理工、湖北幼专、鄂州/咸宁等。

- 中间 8-10 个稳：湖北体育职业学院、武汉体育学院体育科技学院、黄冈职院、荆州职院等。

最后 6-8 个保：三峡旅游职院及其他往年线更低、计划数较多的体育专科组。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find("武汉文理").expect("fixture should contain first list item");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        let first_item_count = flat_texts.iter().filter(|line| line.contains("武汉文理")).count();
        let second_item_count =
            flat_texts.iter().filter(|line| line.contains("湖北体育职业学院")).count();

        assert_eq!(first_item_count, 1, "first loose list item duplicated: {flat_texts:?}");
        assert_eq!(second_item_count, 1, "second loose list item duplicated: {flat_texts:?}");
        assert!(
            flat_texts.iter().any(|line| line.contains("最后 6-8 个保")),
            "following paragraph should still render, got {flat_texts:?}"
        );
    }

    #[test]
    fn active_parent_list_item_does_not_duplicate_nested_list_items() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "\
- parent
  - child one
  - child two
- sibling";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find("parent").expect("fixture should contain parent item");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        let first_child_count = flat_texts.iter().filter(|line| line.contains("child one")).count();
        let second_child_count =
            flat_texts.iter().filter(|line| line.contains("child two")).count();

        assert_eq!(first_child_count, 1, "first child duplicated: {flat_texts:?}");
        assert_eq!(second_child_count, 1, "second child duplicated: {flat_texts:?}");
    }

    // ── Soft-wrap cursor roundtrip ────────────────────────────────────────

    /// Cursor byte → screen position → hit-test back should roundtrip on a
    /// soft-wrapped line (CJK+ASCII mixed content with a narrow viewport).
    #[test]
    fn wysiwyg_cursor_roundtrips_on_second_soft_wrapped_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "这是一段很长的中文测试文本 mixed with english and some more text that \
                      should definitely wrap across multiple visual lines when the viewport \
                      is narrow enough for testing purposes";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // Render with narrow bounds to force soft wrapping.
        render_editor_narrow(&mut view, &doc, 160.0);

        let flat_lines = view.engine().flat_lines().to_vec();
        assert!(
            flat_lines.len() >= 3,
            "narrow viewport must wrap CJK+ASCII line into >= 3 flat lines, got {}",
            flat_lines.len()
        );

        // Use canonical projections to pick a source byte in the middle of the
        // second flat line's content (not the first or last entry).
        let source_maps = view.engine().flat_line_projection_boundaries();
        let map_1 = &source_maps[1];
        let bytes_1 = &map_1.boundaries;
        assert!(
            bytes_1.len() >= 3,
            "second flat line projection must have >= 3 boundaries, got {}",
            bytes_1.len()
        );
        let mid_idx = bytes_1.len() / 2;
        let mid_byte = bytes_1[mid_idx];

        // Set cursor to the middle byte of the second flat line.
        view.handle_message(PluginMessage::SetCursorByte(mid_byte), &mut doc);

        // Re-render so cursor position is computed from the updated layout.
        render_editor_narrow(&mut view, &doc, 160.0);

        // Verify cursor_screen_pos resolves.
        let (cx, cy, cw, ch) =
            view.engine().cursor_screen_pos().expect("cursor must resolve after render");

        // Cursor y must fall within the second flat line's rect (allow minor
        // rounding differences).
        let flat_lines = view.engine().flat_lines().to_vec();
        assert!(flat_lines.len() >= 3, "flat line count must remain >= 3 after cursor move");
        let fl1 = &flat_lines[1];
        assert!(
            cy >= fl1.rect.y - 0.5 && cy <= fl1.rect.y + fl1.rect.h + 0.5,
            "cursor y {cy} should be in second flat line y=[{}, {}], text={:?}",
            fl1.rect.y,
            fl1.rect.y + fl1.rect.h,
            fl1.text
        );

        // Hit-test at cursor horizontal midpoint + vertical midpoint.
        let hit_x = cx + cw * 0.5;
        let hit_y = cy + ch * 0.5;
        let roundtrip = view
            .engine()
            .hit_test_byte(hit_x, hit_y, 0.0, 0.0)
            .expect("hit-test at cursor position must return a byte");

        // The hit-test result should match the original byte. If it differs
        // due to pixel-snapping to a different grapheme boundary, verify
        // it is within the same projection boundary neighborhood (±1 grapheme).
        let source_maps = view.engine().flat_line_projection_boundaries();
        let map_1 = &source_maps[1];
        let bytes_after = &map_1.boundaries;
        let roundtrip_in_same_map = bytes_after.contains(&roundtrip);
        assert!(
            roundtrip_in_same_map || roundtrip.abs_diff(mid_byte) <= 4,
            "roundtrip byte {roundtrip} should match original byte {mid_byte} \
             or be within the same projection (boundaries: {bytes_after:?})",
        );
    }

    #[test]
    fn hit_test_byte_on_later_plain_soft_wrap_uses_segment_source_range() {
        let source = "段落00甲乙丙丁 段落01甲乙丙丁 段落02甲乙丙丁 段落03甲乙丙丁 \
                      段落04甲乙丙丁 段落05甲乙丙丁 段落06甲乙丙丁 段落07甲乙丙丁 \
                      段落08甲乙丙丁 段落09甲乙丙丁 段落10甲乙丙丁 段落11甲乙丙丁";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        render_editor_narrow(&mut view, &doc, 180.0);

        let flat_lines = view.engine().flat_lines();
        assert!(
            flat_lines.len() >= 3,
            "narrow viewport should soft-wrap plain paragraph into >= 3 flat lines, got {}",
            flat_lines.len()
        );
        let target_line = flat_lines
            .iter()
            .skip(2)
            .find(|line| !line.text.trim().is_empty())
            .expect("later soft-wrapped line should contain text");
        let expected_start = source.find(&target_line.text).unwrap_or_else(|| {
            panic!("source should contain target segment {:?}", target_line.text)
        });
        let expected_end = expected_start + target_line.text.len();

        let hit = view
            .engine()
            .hit_test_byte(
                target_line.rect.x + 1.0,
                target_line.rect.y + target_line.rect.h * 0.5,
                0.0,
                0.0,
            )
            .expect("hit-test on later soft-wrapped segment should return a byte");

        assert!(
            (expected_start..=expected_end).contains(&hit),
            "click on later soft-wrap segment {:?} mapped to byte {hit}, expected {expected_start}..={expected_end}",
            target_line.text
        );
    }

    /// 滚动后 cursor_screen_pos → hit_test_byte 仍 roundtrip。
    /// cursor_screen_pos 减去 scroll_y 使输出与渲染的 ly = rect.y - scroll_y + oy 一致；
    /// hit_test_byte 加回 scroll_y 使输入插件坐标映射到正确的文档行。
    #[test]
    fn wysiwyg_cursor_roundtrips_after_scroll_y() {
        let source = "这是一段很长的中文测试文本 mixed with english and some more text that \
                      should definitely wrap across multiple visual lines when the viewport \
                      is narrow enough for testing purposes";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        render_editor_narrow(&mut view, &doc, 160.0);

        let flat_lines = view.engine().flat_lines().to_vec();
        assert!(
            flat_lines.len() >= 3,
            "narrow viewport must wrap CJK+ASCII line into >= 3 flat lines, got {}",
            flat_lines.len()
        );

        // Pick a known byte in the third flat line (index 2).
        let (mid_byte, bytes_2) = {
            let source_maps = view.engine().flat_line_projection_boundaries();
            let map_2 = &source_maps[2];
            let b = &map_2.boundaries;
            assert!(
                b.len() >= 3,
                "third flat line projection must have >= 3 boundaries, got {}",
                b.len()
            );
            let mid_idx = b.len() / 2;
            (b[mid_idx], b.to_vec())
        };

        // Set cursor and scroll to a non-zero position.
        view.handle_message(PluginMessage::SetCursorByte(mid_byte), &mut doc);

        let scroll = 120.0;
        view.engine.scroll_y = scroll;

        // Re-render so layout reflects the cursor position.
        render_editor_narrow(&mut view, &doc, 160.0);

        // Forward: cursor_screen_pos must return something reasonable
        // (y reflects scroll projection, i.e. doc_y - scroll_y).
        let (cx, cy, _cw, ch) =
            view.engine().cursor_screen_pos().expect("cursor must resolve after render");

        // Cursor rect height must be non-zero.
        assert!(ch > 0.0, "cursor rect height must be positive, got {ch}");

        // Find the flat line our cursor landed on and verify scroll projection.
        {
            let flat_idx = {
                let lazy = view.engine().lazy.as_ref().unwrap();
                let mut found = None;
                for (fi, flat_line) in lazy.flat_lines.iter().enumerate() {
                    if flat_line.source_projection.as_ref().is_some_and(|projection| {
                        projection.boundaries.iter().any(|anchor| anchor.byte == mid_byte)
                    }) {
                        found = Some(fi);
                        break;
                    }
                }
                found.expect("mid_byte must be in some source map after re-render")
            };
            let fl = &view.engine().flat_lines()[flat_idx];
            let expected_cursor_y = fl.rect.y + (fl.rect.h - ch) * 0.5 - scroll;
            assert!(
                (cy - expected_cursor_y).abs() < 2.0,
                "cursor_screen_pos y={cy} should ≈ centered line y {} (fl.rect.y={}, scroll={scroll}, flat_idx={flat_idx})",
                expected_cursor_y,
                fl.rect.y,
            );
        }

        // Reverse: hit_test_byte at cursor position + container offset must
        // return the same source byte. offset_y models the app's bounds.y.
        let offset_y = 200.0;
        let hit_y = cy + ch * 0.5 + offset_y;
        let hit_x = cx;
        let roundtrip = view
            .engine()
            .hit_test_byte(hit_x, hit_y, 0.0, offset_y)
            .expect("hit-test at cursor position must return a byte");

        let roundtrip_matches = bytes_2.contains(&roundtrip) || roundtrip.abs_diff(mid_byte) <= 4;
        assert!(
            roundtrip_matches,
            "roundtrip byte {roundtrip} should match original byte {mid_byte} \
             or be within same source map (map bytes: {bytes_2:?})",
        );
    }

    #[test]
    fn cursor_moves_immediately_after_indenting_an_empty_paragraph() {
        const SOFT_TAB: &str = "    ";
        for prefix in ["", "before\n\n"] {
            let mut document = StubDoc::new(prefix);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(prefix.len()), &mut document);
            render_editor_once(&mut view, &document);
            let before = view.engine().cursor_screen_pos().expect("empty line must have a caret");

            document.text.push_str(SOFT_TAB);
            view.set_source(document.text.clone(), 2);
            view.handle_message(PluginMessage::SetCursorByte(document.text.len()), &mut document);
            let updated = view.engine().cursor_screen_pos().expect("indented line needs a caret");
            assert!(updated.0 > before.0, "Tab must move the caret before the next paint");
            render_editor_once(&mut view, &document);
            let painted = view.engine().cursor_screen_pos().expect("painted line needs a caret");
            assert!(painted.0 > before.0, "Tab must remain visible after painting");
            assert_eq!(painted.1, before.1, "indentation must preserve the caret row");
            let line = view.engine().flat_lines().last().expect("empty paragraph is rendered");
            let expected_advance = view.engine().navigation_font_metrics_for_line(line).grapheme_x(
                SOFT_TAB,
                SOFT_TAB.len(),
                line.font_size,
            );
            assert!(
                (painted.0 - before.0 - expected_advance).abs() < f32::EPSILON,
                "caret must advance by four body-font spaces",
            );
        }
    }

    #[test]
    fn indented_code_hit_testing_preserves_multibyte_source_boundaries() {
        for indentation in ["    ", "\t"] {
            let source = format!("before\n\n{indentation}第一行\n\n{indentation}第二行\n\nafter");
            let document = StubDoc::new(&source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.clone(), 1);
            render_editor_once(&mut view, &document);

            for content in ["第一行", "第二行"] {
                let line = view
                    .engine()
                    .flat_lines()
                    .iter()
                    .find(|line| line.is_code && line.text == content)
                    .expect("indented code content must have a rendered line");
                let content_start = source.find(content).expect("fixture contains code content");
                assert_eq!(
                    view.engine().hit_test_byte(
                        line.rect.x,
                        line.rect.y + line.rect.h * 0.5,
                        0.0,
                        0.0,
                    ),
                    Some(content_start),
                    "code hit must skip source indentation {indentation:?}",
                );
                let projection = line
                    .source_projection
                    .as_ref()
                    .expect("code line must retain its source projection");
                assert!(projection.boundaries.iter().all(|anchor| {
                    source.is_char_boundary(anchor.byte)
                        && (content_start..=content_start + content.len()).contains(&anchor.byte)
                }));
            }
        }
    }

    #[test]
    fn code_block_hit_test_returns_code_content_byte() {
        let src = "```rust\nabc\ndef\n```\n\nparagraph\n";
        let mut v = make_view(src);

        // Code block starts at byte 0.
        // "```rust\n" is 8 bytes.
        // "abc\n" is 4 bytes (8..12).
        // "def\n" is 4 bytes (12..16).
        // "```\n" is 4 bytes.
        // We want to hit-test on 'e' in "def", which is byte 13.

        // Let's set cursor at 13, get screen pos, and hit_test it back.
        let mut doc = StubDoc::new(src);
        use ui::plugin::{PluginMessage, ViewPlugin};
        v.handle_message(PluginMessage::SetCursorByte(13), &mut doc);

        let pos = v.engine().cursor_screen_pos().unwrap();
        let (cx, cy, _cw, ch) = pos;

        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        assert_eq!(result, Some(13), "hit_test on 'e' in 'def' should return byte 13");
    }

    #[test]
    fn enter_on_empty_bullet_exits_list() {
        let src = "- ";
        let v = make_view(src);

        let aug = v.engine().augment_edit(2, ui::plugin::AugmentKind::Enter);

        let aug = aug.expect("Should return an augmentation");
        assert_eq!(aug.replace_range, Some(0..2), "Should replace the entire list marker");
        assert_eq!(
            aug.insert_text,
            Some(String::from("")),
            "Should insert empty text to exit list"
        );
    }

    #[test]
    fn enter_on_empty_ordered_item_exits_list() {
        let src = "1. ";
        let v = make_view(src);

        let aug = v.engine().augment_edit(3, ui::plugin::AugmentKind::Enter);

        let aug = aug.expect("Should return an augmentation");
        assert_eq!(aug.replace_range, Some(0..3), "Should replace the entire ordered list marker");
        assert_eq!(
            aug.insert_text,
            Some(String::from("")),
            "Should insert empty text to exit list"
        );
    }

    #[test]
    fn enter_on_non_empty_ordered_item_continues_numbering() {
        let src = "1. abc";
        let v = make_view(src);

        let aug = v.engine().augment_edit(6, ui::plugin::AugmentKind::Enter);

        let aug = aug.expect("Should return an augmentation");
        assert_eq!(aug.replace_range, None, "Should not replace anything");
        assert_eq!(aug.insert_text, Some(String::from("\n2. ")), "Should continue numbering");
    }

    #[test]
    fn empty_fenced_code_block_exposes_editable_content_line() {
        const OPENING_FENCE: &str = "```\n";
        let source = "before\n\n```\n```\n\nafter";
        let content_start =
            source.find(OPENING_FENCE).expect("fixture must contain an opening fence")
                + OPENING_FENCE.len();
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_once(&mut view, &document);

        let content_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| {
                line.source_projection.as_ref().is_some_and(|projection| {
                    projection.boundaries.iter().any(|anchor| anchor.byte == content_start)
                })
            })
            .expect("empty fenced code block must retain an editable content projection");
        let hit = view.engine().hit_test_byte(
            content_line.rect.x,
            content_line.rect.y + content_line.rect.h * 0.5,
            0.0,
            0.0,
        );

        assert_eq!(hit, Some(content_start));

        view.handle_message(ui::plugin::PluginMessage::SetCursorByte(content_start), &mut document);
        render_editor_once(&mut view, &document);
        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("clicking the empty code content must reveal a cursor");

        assert_eq!(
            view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0,),
            Some(content_start),
        );
    }

    #[test]
    fn internal_soft_break_trailing_whitespace_advances_cursor_x() {
        for source in ["hello \nworld", "hello\t\nworld"] {
            let mut view = MarkdownEditorView::new();
            let mut doc = StubDoc::new(source);
            view.set_source(source.into(), 1);
            render_editor_once(&mut view, &doc);

            let before_space = "hello".len();
            let after_space = before_space + 1;
            let x_before = view
                .engine()
                .cursor_screen_pos_for_byte(before_space)
                .expect("before soft break space must have cursor pos")
                .0;
            let x_after_downstream = view
                .engine()
                .cursor_screen_pos_for_byte(after_space)
                .expect("after soft break trailing whitespace must have cursor pos")
                .0;
            view.handle_message(ui::plugin::PluginMessage::SetCursorByte(after_space), &mut doc);
            let x_after_cursor = view
                .engine()
                .cursor_screen_pos()
                .expect("caret at trailing whitespace before SoftBreak must render")
                .0;
            assert!(
                x_after_downstream > x_before && x_after_cursor > x_before,
                "cursor after trailing whitespace before SoftBreak must advance visually for {source:?}: before={x_before}, downstream={x_after_downstream}, cursor={x_after_cursor}"
            );
        }

        let multi_ws = "hello \t\nworld";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(multi_ws);
        view.set_source(multi_ws.into(), 1);
        render_editor_once(&mut view, &doc);
        let x5 = view.engine().cursor_screen_pos_for_byte(5).expect("byte 5").0;
        let x6 = view.engine().cursor_screen_pos_for_byte(6).expect("byte 6").0;
        let x7 = view.engine().cursor_screen_pos_for_byte(7).expect("byte 7").0;
        assert!(
            x6 > x5 && x7 > x6,
            "multi-ws before SoftBreak must advance: x5={x5}, x6={x6}, x7={x7}"
        );

        let bold_ws = "**hello** \t\nworld";
        let mut bold_view = MarkdownEditorView::new();
        let bold_doc = StubDoc::new(bold_ws);
        bold_view.set_source(bold_ws.into(), 1);
        render_editor_once(&mut bold_view, &bold_doc);
        let bx9 = bold_view.engine().cursor_screen_pos_for_byte(9).expect("byte 9").0;
        let bx10 = bold_view.engine().cursor_screen_pos_for_byte(10).expect("byte 10").0;
        let bx11 = bold_view.engine().cursor_screen_pos_for_byte(11).expect("byte 11").0;
        assert!(
            bx10 > bx9 && bx11 > bx10,
            "trailing ws after bold marker must advance: bx9={bx9}, bx10={bx10}, bx11={bx11}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::plugin::{
        EditAugmentation, EditIntent, EditPlan, EditPolicy, EditRequest, EditSelection,
        EditTransaction, KeyIntentMapper, PastePreference, TextReplacement, ViewPlugin,
    };

    #[test]
    fn markdown_editor_is_the_only_markdown_view_requesting_semantic_paste() {
        assert_eq!(MarkdownEditorView::new().paste_preference(), PastePreference::SemanticMarkdown);
        assert_eq!(MarkdownView::new().paste_preference(), PastePreference::PlainText);
    }

    fn test_enter_context(source: &str, byte: usize) -> EnterContext {
        classify_enter_context(source, byte)
    }

    #[test]
    fn table_cell_empty_insertion() {
        let src = "| A |\n|---|";
        let ctx = test_enter_context(src, 3);
        if let EnterContext::TableCell { next_cell_start, .. } = ctx {
            assert_eq!(next_cell_start, None);
        } else {
            panic!("Expected TableCell");
        }
    }

    #[test]
    fn markdown_edit_policy_maps_cursor_only_augmentation_to_move_cursor() {
        let mut view = MarkdownEditorView::new();
        let source = "| a |\n|---|\n| b |";
        view.set_source(source.into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: source.find('a').expect("first cell"),
            selection: None,
            intent: EditIntent::InsertParagraphBreak,
        };

        assert!(matches!(view.plan_edit(&request), EditPlan::MoveCursor(_)));
    }

    #[test]
    fn markdown_key_mapper_distinguishes_shift_enter_from_plain_enter() {
        let view = MarkdownEditorView::new();
        let shift = ui::core::Modifiers { shift: true, ..ui::core::Modifiers::NONE };

        assert_eq!(view.map_key(&ui::KeyCode::Enter, &shift), Some(EditIntent::InsertLineBreak));
        assert_eq!(view.map_key(&ui::KeyCode::Enter, &ui::core::Modifiers::NONE), None);
    }

    #[test]
    fn markdown_edit_policy_applies_line_break_after_deleting_selection() {
        let mut view = MarkdownEditorView::new();
        let source = "left selected right";
        view.set_source(source.into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: "left selected".len(),
            selection: Some("left ".len().."left selected".len()),
            intent: EditIntent::InsertLineBreak,
        };

        let (edited_source, _) = apply_single_replacement(source, &view.plan_edit(&request));
        assert_eq!(edited_source, "left \\\n right");
    }

    #[cfg(feature = "rich-markdown")]
    #[test]
    fn markdown_edit_policy_deletes_complete_inline_html_break_forward() {
        let mut view = MarkdownEditorView::new();
        let source = "# left<br>right";
        view.set_source(source.into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: "# left".len(),
            selection: None,
            intent: EditIntent::DeleteForward,
        };

        let (edited_source, cursor) = apply_single_replacement(source, &view.plan_edit(&request));
        assert_eq!(edited_source, "# leftright");
        assert_eq!(cursor, "# left".len());
    }

    #[test]
    fn augmentation_edit_plan_preserves_transaction_protocol_fields() {
        let request = EditRequest {
            source_generation: 42,
            cursor_byte: 9,
            selection: None,
            intent: EditIntent::InsertText("replacement".into()),
        };
        let augmentation = EditAugmentation {
            replace_range: Some(4..9),
            insert_text: Some("-".into()),
            cursor_byte_after: 5,
        };

        assert_eq!(
            augmentation_edit_plan(&request, augmentation),
            EditPlan::Apply(EditTransaction {
                source_generation: 42,
                replacements: vec![TextReplacement { range: 4..9, text: "-".into() }],
                selection_after: EditSelection::Caret(5),
            })
        );
    }

    #[test]
    fn blockquote_marker_byte_selection_highlight_uses_bounded_source_line_lookups() {
        let text = "> quoted text\n\nparagraph";
        let mut engine = PreviewEngine::<crate::builder::MarkdownDoc>::new();
        engine.set_edit_source(Some(text.to_string()));
        let style = crate::test_utils::default_style();
        let parsed = crate::parser::parse_markdown(text);
        let theme = ui::theme::test_theme();
        engine.rebuild_layout(
            crate::builder::MarkdownDoc::build(&parsed, &style),
            &style,
            800.0,
            600.0,
            None,
            &crate::view::AppCodeHighlighter { theme: &theme },
            &core::document::StringDocView::new(text),
            true,
            true,
        );

        reset_source_line_at_byte_call_count();
        // Select 'q' at byte index 2 (between > and q)
        engine.set_sel_anchor_byte(Some(2));
        engine.set_sel_cursor_byte(Some(2));
        engine.selection_highlights([1.0; 4]);

        assert!(
            source_line_at_byte_call_count() <= 2,
            "Bounded binary searches should strictly limit source_line_at_byte fallback calls (was {})",
            source_line_at_byte_call_count()
        );
    }

    fn selection_enter_plan(source: &str, selection: std::ops::Range<usize>) -> EditPlan {
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: selection.end,
            selection: Some(selection),
            intent: EditIntent::InsertParagraphBreak,
        };
        view.plan_edit(&request)
    }

    /// 应用单条替换计划到源码，返回（最终文本, 光标落点）。
    pub(super) fn apply_single_replacement(source: &str, plan: &EditPlan) -> (String, usize) {
        let EditPlan::Apply(transaction) = plan else {
            panic!("expected a single Apply plan, got {plan:?}");
        };
        assert_eq!(transaction.replacements.len(), 1, "selection Enter must stay atomic");
        let replacement = &transaction.replacements[0];
        let mut edited = source.to_owned();
        edited.replace_range(replacement.range.clone(), &replacement.text);
        let EditSelection::Caret(cursor) = transaction.selection_after else {
            panic!("selection Enter must end in a caret");
        };
        (edited, cursor)
    }

    #[test]
    fn selection_enter_in_paragraph_splits_block_at_deletion_point() {
        let source = "hello world";
        let plan = selection_enter_plan(source, 2..8);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "he\n\nrld");
        assert_eq!(cursor, 4, "光标应落在新块开头");
    }

    #[test]
    fn selection_enter_across_paragraphs_uses_deletion_point_context() {
        let source = "foo\n\nbar";
        let plan = selection_enter_plan(source, 2..5);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "fo\n\nbar");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn selection_enter_in_list_item_continues_marker() {
        let source = "- hello world";
        let plan = selection_enter_plan(source, 4..9);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "- he\n- orld");
        assert_eq!(cursor, 7, "光标应落在续行 marker 之后");
    }

    #[test]
    fn selection_enter_in_heading_middle_splits_heading() {
        let source = "# hello world";
        let plan = selection_enter_plan(source, 4..9);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "# he\norld");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn selection_enter_in_setext_heading_preserves_heading_and_creates_block_after_underline() {
        let source = "Title\n===\nafter";
        let plan = selection_enter_plan(source, 1..4);
        let EditPlan::Apply(transaction) = plan else {
            panic!("Setext selection Enter must produce one atomic transaction");
        };

        assert_eq!(transaction.replacements.len(), 2);
        let mut edited = source.to_owned();
        let mut replacements = transaction.replacements.clone();
        replacements.sort_by_key(|replacement| replacement.range.start);
        for replacement in replacements.iter().rev() {
            edited.replace_range(replacement.range.clone(), &replacement.text);
        }

        assert_eq!(edited, "Te\n===\n\n\nafter");
        assert_eq!(transaction.selection_after, EditSelection::Caret("Te\n===\n\n".len()));
    }

    #[test]
    fn selection_enter_leaving_empty_list_item_exits_list() {
        let source = "- a";
        let plan = selection_enter_plan(source, 2..3);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn selection_enter_in_code_block_falls_back_to_default() {
        let source = "```\ncode\n```";
        let plan = selection_enter_plan(source, 4..8);

        assert_eq!(plan, EditPlan::UseDefault);
    }

    #[test]
    fn zero_width_selection_is_treated_as_no_selection() {
        let source = "hello world";
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        let zero_width = EditRequest {
            source_generation: 1,
            cursor_byte: 4,
            selection: Some(4..4),
            intent: EditIntent::InsertParagraphBreak,
        };
        let no_selection = EditRequest { selection: None, ..zero_width.clone() };

        assert_eq!(view.plan_edit(&zero_width), view.plan_edit(&no_selection));
    }

    #[test]
    fn selection_backspace_still_falls_back_to_default() {
        let mut view = MarkdownEditorView::new();
        view.set_source("hello world".into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: 5,
            selection: Some(2..5),
            intent: EditIntent::DeleteBackward,
        };

        assert_eq!(view.plan_edit(&request), EditPlan::UseDefault);
    }

    #[test]
    fn table_vertical_navigation_preserves_column_and_traverses_wrapped_cell_lines() {
        use ui::plugin::{MoveDirection, ViewPlugin};

        let source = concat!(
            "before\n\n",
            "| alpha wrap one two | head2 |\n",
            "| --- | --- |\n",
            "| bravo | r1c1 top<br>r1c1 bottom |\n",
            "| charl | r2c1 |\n\n",
            "after"
        );
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        let document = core::document::StringDocView::new(source);
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let narrow_bounds = ui::core::geom::Rect::new(0.0, 0.0, 260.0, 600.0);
        let _ = <MarkdownEditorView as ViewPlugin>::render(
            &mut view,
            &document,
            narrow_bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        let alpha_line_lines: Vec<usize> = view
            .engine()
            .flat_lines()
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                let owner = line.source_projection.as_ref()?.owner;
                matches!(
                    owner,
                    crate::projection::ProjectionOwnerId::TableCell { row: 0, column: 0, .. }
                )
                .then_some(idx)
            })
            .collect();
        assert!(
            alpha_line_lines.len() >= 2,
            "narrow table fixture must wrap cell (0, 0) into >= 2 visual lines"
        );

        // Column 0: Down traverses wrapped lines inside (0,0), then enters (1,0), (2,0), then exits to `after`.
        let alpha_start = source.find("alpha").expect("alpha") + 2;
        let step1 = view
            .engine()
            .visual_move(alpha_start, MoveDirection::Down, None)
            .expect("Down within wrapped cell (0, 0)");
        let step1_owner = view.engine().flat_lines().iter().find_map(|line| {
            let proj = line.source_projection.as_ref()?;
            proj.boundaries.iter().any(|b| b.byte == step1).then_some(proj.owner)
        });
        assert!(
            matches!(
                step1_owner,
                Some(crate::projection::ProjectionOwnerId::TableCell { row: 0, column: 0, .. })
            ),
            "first Down inside wrapped (0,0) must stay in (0,0): byte={step1}, owner={step1_owner:?}"
        );

        let last_wrap_line =
            &view.engine().flat_lines()[*alpha_line_lines.last().expect("nonempty")];
        let last_wrap_start = last_wrap_line
            .source_projection
            .as_ref()
            .and_then(|p| p.boundaries.first())
            .expect("wrapped line start")
            .byte;
        let step_to_r1c0 = view
            .engine()
            .visual_move(last_wrap_start, MoveDirection::Down, None)
            .expect("Down from last wrap of (0,0) to (1,0)");
        let bravo_start = source.find("bravo").expect("bravo");
        let bravo_end = bravo_start + "bravo".len();
        assert!(
            (bravo_start..=bravo_end).contains(&step_to_r1c0),
            "expected Down from (0,0) to land in (1,0) bravo, got {step_to_r1c0} ({:?})",
            source.get(step_to_r1c0.saturating_sub(2)..(step_to_r1c0 + 4).min(source.len()))
        );

        let step_to_r2c0 = view
            .engine()
            .visual_move(step_to_r1c0, MoveDirection::Down, None)
            .expect("Down from (1,0) to (2,0)");
        let charl_start = source.find("charl").expect("charl");
        let charl_end = charl_start + "charl".len();
        assert!(
            (charl_start..=charl_end).contains(&step_to_r2c0),
            "expected Down from (1,0) to land in (2,0) charl, got {step_to_r2c0}"
        );

        let exit_down = view
            .engine()
            .visual_move(step_to_r2c0, MoveDirection::Down, None)
            .expect("Down from last table row exits to after");
        let after_start = source.find("after").expect("after");
        assert!(exit_down >= after_start, "expected exit to after, got {exit_down}");

        // Column 1: Up from header exits to `before`; Down enters r1c1 top -> bottom -> r2c1; Up reverses.
        let head2_mid = source.find("head2").expect("head2") + 2;
        let exit_up = view
            .engine()
            .visual_move(head2_mid, MoveDirection::Up, None)
            .expect("Up from header col 1 exits to before");
        assert!(
            exit_up <= "before".len(),
            "expected Up from header to reach before, got {exit_up}"
        );

        let r1c1_top_start = source.find("r1c1 top").expect("r1c1 top");
        let r1c1_bottom_start = source.find("r1c1 bottom").expect("r1c1 bottom");
        let r2c1_mid = source.find("r2c1").expect("r2c1") + 2;

        let down_into_r1c1 = view
            .engine()
            .visual_move(head2_mid, MoveDirection::Down, None)
            .expect("Down from head2 to r1c1 top");
        assert!(
            (r1c1_top_start..r1c1_bottom_start).contains(&down_into_r1c1),
            "expected landing in r1c1 top, got {down_into_r1c1}"
        );

        let down_into_r1c1_bottom = view
            .engine()
            .visual_move(r1c1_top_start + 2, MoveDirection::Down, None)
            .expect("Down from r1c1 top to r1c1 bottom");
        assert!(
            (r1c1_bottom_start..r1c1_bottom_start + "r1c1 bottom".len())
                .contains(&down_into_r1c1_bottom),
            "expected landing in r1c1 bottom, got {down_into_r1c1_bottom}"
        );

        let up_from_r2c1 = view
            .engine()
            .visual_move(r2c1_mid, MoveDirection::Up, None)
            .expect("Up from r2c1 enters bottom line of r1c1");
        assert!(
            (r1c1_bottom_start..=r1c1_bottom_start + "r1c1 bottom".len()).contains(&up_from_r2c1),
            "expected Up from r2c1 to land in r1c1 bottom, got {up_from_r2c1}"
        );
    }
}

#[cfg(test)]
#[path = "view_font_tests.rs"]
mod font_tests;
