use super::*;

const SOURCE_TEXT: &str = "Skill agent loop";
const LOGICAL_FONT_SIZE: f32 = 16.0;

fn set_engine_font(engine: &mut PreviewEngine, font_family: &str) {
    engine.handle_message_common(&PluginMessage::SetRenderSettings {
        font_family: font_family.into(),
        font_size: LOGICAL_FONT_SIZE,
        line_height: 24.0,
        toc_max_depth: 3,
        markdown_first_line_indent: false,
    });
}

fn assert_plugin_matches_source_font(plugin: &mut dyn ViewPlugin, font_family: &str, dpi: f32) {
    let theme = ui::theme::test_theme();
    let doc = core::document::StringDocView::new(SOURCE_TEXT);
    let mut shaper = shaping::Shaper::new().expect("font consistency requires system fonts");
    shaper.set_font_family(Some(font_family));
    shaper.set_font_size(LOGICAL_FONT_SIZE * dpi);
    let source_shape = shaper.shape(SOURCE_TEXT).expect("source text should shape");
    let draw_list =
        plugin.render(&doc, ui::Rect::new(0.0, 0.0, 800.0, 600.0), &theme, &mut shaper, dpi);
    let rendered = draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            ui::DrawCmd::TextLayout { layout, .. } if layout.text == SOURCE_TEXT => Some(layout),
            _ => None,
        })
        .expect("body text must be emitted");
    assert_eq!(rendered.font_family.as_deref(), Some(font_family));
    assert_eq!(
        rendered.shaped, source_shape,
        "source and preview must use the same actual glyphs and advances"
    );
}

#[test]
fn preview_and_wysiwyg_follow_font_changes_including_cached_layouts() {
    let mut preview = MarkdownView::new();
    preview.set_source(SOURCE_TEXT.into(), 1);
    let mut editor = MarkdownEditorView::new();
    editor.set_source(SOURCE_TEXT.into(), 1);
    for font_family in ["Menlo", "Helvetica", "Menlo"] {
        set_engine_font(&mut preview.engine, font_family);
        set_engine_font(&mut editor.engine, font_family);
        for dpi in [1.0, 2.0] {
            assert_plugin_matches_source_font(&mut preview, font_family, dpi);
            assert_plugin_matches_source_font(&mut editor, font_family, dpi);
        }
    }
}

#[test]
fn replacement_preedit_keeps_the_configured_body_font() {
    let mut editor = MarkdownEditorView::new();
    editor.set_source(SOURCE_TEXT.into(), 1);
    editor.engine.handle_set_cursor_byte(SOURCE_TEXT.len());
    editor.engine.set_sel_anchor_byte(Some(0));
    editor.engine.set_sel_cursor_byte(Some(SOURCE_TEXT.len()));
    editor.engine.handle_message_common(&PluginMessage::SetPreedit {
        text: SOURCE_TEXT.into(),
        cursor: Some((SOURCE_TEXT.len(), SOURCE_TEXT.len())),
    });
    for font_family in ["Helvetica", "Menlo"] {
        set_engine_font(&mut editor.engine, font_family);
        assert_plugin_matches_source_font(&mut editor, font_family, 2.0);
        let composition = editor
            .engine
            .replacement_preedit
            .as_ref()
            .expect("selection replacement must exercise the IME preview engine");
        assert_eq!(composition.engine.base_font_family, font_family);
    }
}

#[test]
fn markdown_body_uses_configured_editor_font_at_every_dpi() {
    let theme = ui::theme::test_theme();
    for font_family in ["Menlo", "Helvetica"] {
        let mut settings = ui::settings::Settings::new();
        settings.set_font_family(font_family.into());
        for dpi in [1.0, 2.0] {
            let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
            let style =
                MarkdownRenderSettings::from_metrics(&settings, &metrics).style_at_dpi(&theme, dpi);
            assert_eq!(style.body_font_family, vec![font_family.to_string()]);
            assert_eq!(style.body_font_size, metrics.font_size);
            assert_eq!(style.code_font_family.as_deref(), Some("monospace"));
        }
    }
}

#[test]
fn editor_font_change_invalidates_markdown_style_identity() {
    let theme = ui::theme::test_theme();
    let mut settings = ui::settings::Settings::new();
    let metrics = ui::settings::UiMetrics::from_settings(&settings, 1.0);
    settings.set_font_family("Menlo".into());
    let before = MarkdownRenderSettings::from_metrics(&settings, &metrics).style(&theme);
    settings.set_font_family("Helvetica".into());
    let after = MarkdownRenderSettings::from_metrics(&settings, &metrics).style(&theme);
    assert_ne!(style_hash_quick(&before), style_hash_quick(&after));
}
