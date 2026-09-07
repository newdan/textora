//! Selection replacement is a separate layout; document coordinates remain authoritative.

use super::*;
use std::ops::Range;

pub(super) struct ReplacementPreedit<S: BlockSource> {
    pub(super) engine: Box<PreviewEngine<S>>,
    selection: Range<usize>,
    inserted_len: usize,
}

impl<S: BlockSource> ReplacementPreedit<S> {
    pub(super) fn preview_byte(&self, source_byte: usize) -> usize {
        if source_byte <= self.selection.start {
            return source_byte;
        }
        if source_byte < self.selection.end {
            return self.selection.start;
        }
        source_byte - self.selection.len() + self.inserted_len
    }

    pub(super) fn source_byte(&self, preview_byte: usize) -> usize {
        if preview_byte <= self.selection.start {
            return preview_byte;
        }
        if preview_byte <= self.selection.start + self.inserted_len {
            return self.selection.start;
        }
        preview_byte - self.inserted_len + self.selection.len()
    }
}

impl<S: BlockSource> PreviewEngine<S> {
    pub(super) fn update_replacement_preedit(&mut self) {
        let Some(context) = self.edit_ctx.as_ref().filter(|context| context.preedit_text.is_some())
        else {
            self.clear_replacement_preedit();
            return;
        };
        let selection = self.byte_selection_range().map(|(start, end)| start..end);
        let Some(selection) = selection else {
            self.clear_replacement_preedit();
            return;
        };
        let Some(source) = self.edit_source.as_deref() else { return };
        if source.get(selection.clone()).is_none() {
            return;
        }
        let preedit_text = context.preedit_text.as_deref().expect("active composition has text");
        let requested_caret =
            selection.start + preedit_cursor_offset(preedit_text, context.preedit_cursor);
        let inserted_len = preedit_text.len();
        let mut preview_source = source.to_owned();
        preview_source.replace_range(selection.clone(), preedit_text);
        let caret_byte = preview_source
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain(std::iter::once(preview_source.len()))
            .take_while(|byte| *byte <= requested_caret)
            .last()
            .unwrap_or_default();
        let composition = self.replacement_preedit.get_or_insert_with(|| ReplacementPreedit {
            engine: Box::new(PreviewEngine::new()),
            selection: selection.clone(),
            inserted_len,
        });
        composition.selection = selection;
        composition.inserted_len = inserted_len;
        let engine = &mut composition.engine;
        if engine.edit_source.as_deref() != Some(preview_source.as_str()) {
            engine.set_edit_source(Some(preview_source));
            engine.mark_source_dirty();
        }
        engine.set_source_generation(self.source_generation);
        engine.handle_set_cursor_byte(caret_byte);
    }

    fn clear_replacement_preedit(&mut self) {
        if self.replacement_preedit.take().is_some() {
            self.dirty = EngineDirty::SourceChanged;
        }
    }
}

pub(super) fn render_editor(
    view: &mut MarkdownEditorView,
    bounds: ui::Rect,
    theme: &Theme,
    shaper: &mut shaping::Shaper,
    dpi_scale: f32,
) -> DrawList {
    let original = &mut view.engine;
    let Some(composition) = &mut original.replacement_preedit else {
        return render_engine(original, &view.source, None, bounds, theme, shaper, dpi_scale);
    };
    let preview = &mut composition.engine;
    preview.base_font_size = original.base_font_size;
    preview.base_line_height = original.base_line_height;
    preview.toc_max_depth = original.toc_max_depth;
    preview.markdown_first_line_indent = original.markdown_first_line_indent;
    preview.scroll_y = original.scroll_y;
    preview.cursor_visible = original.cursor_visible;
    let preview_source = preview.edit_source.clone().expect("replacement preview owns its source");
    let mut commands = render_engine(
        preview,
        &preview_source,
        Some(composition.selection.start..composition.selection.start + composition.inserted_len),
        bounds,
        theme,
        shaper,
        dpi_scale,
    );
    draw_replacement_underline(&mut commands, composition, bounds, theme, dpi_scale);
    original.content_height = composition.engine.content_height;
    original.scroll_y = composition.engine.scroll_y;
    commands
}

fn draw_replacement_underline(
    commands: &mut DrawList,
    composition: &ReplacementPreedit<MarkdownDoc>,
    bounds: ui::Rect,
    theme: &Theme,
    dpi_scale: f32,
) {
    let composing_range =
        composition.selection.start..composition.selection.start + composition.inserted_len;
    let thickness = dpi_scale;
    for line in composition.engine.flat_lines() {
        let Some(projection) = &line.source_projection else { continue };
        let covered = projection
            .boundaries
            .windows(2)
            .enumerate()
            .filter_map(|(grapheme, anchors)| {
                (anchors[0].byte < composing_range.end && anchors[1].byte > composing_range.start)
                    .then_some(grapheme)
            })
            .collect::<Vec<_>>();
        let (Some(first), Some(last)) = (covered.first(), covered.last()) else { continue };
        let left = crate::layout::grapheme_x(line, *first);
        let right = crate::layout::grapheme_x(line, last + 1);
        commands.fill(
            ui::Rect::new(
                bounds.x + line.rect.x + left,
                bounds.y + line.rect.y + line.font_size - composition.engine.scroll_y,
                right - left,
                thickness,
            ),
            theme.editor.foreground,
        );
    }
}

fn render_engine(
    engine: &mut PreviewEngine,
    source: &str,
    composing_range: Option<Range<usize>>,
    bounds: ui::Rect,
    theme: &Theme,
    shaper: &mut shaping::Shaper,
    dpi_scale: f32,
) -> DrawList {
    let render_started_at = std::time::Instant::now();
    let settings = MarkdownRenderSettings {
        font_size: engine.base_font_size * dpi_scale,
        line_height: engine.base_line_height * dpi_scale,
        toc_max_depth: engine.toc_max_depth,
        markdown_first_line_indent: engine.markdown_first_line_indent,
    };
    let style = settings.style(theme);
    engine.toc_max_depth = settings.toc_max_depth;
    let string_doc = core::document::StringDocView::new(source);
    let (mut dl, _) = engine.render(
        theme,
        bounds.w,
        bounds.h,
        bounds.x,
        bounds.y,
        &style,
        |s| build_composition_document(source, s, composing_range.as_ref()),
        Some(shaper),
        &string_doc,
        true,  // editing keeps whole-document flat lines for selection/navigation.
        false, // precise shaping/highlighting stays viewport-driven for responsiveness.
    );
    let render_duration_us = render_started_at.elapsed().as_micros();
    #[cfg(debug_assertions)]
    {
        let _ =
            std::fs::OpenOptions::new().create(true).append(true).open("/tmp/perf.log").and_then(
                |mut f| {
                    use std::io::Write;
                    writeln!(f, "[md render] {} us", render_duration_us)
                },
            );
        println!("[md render] {} us", render_duration_us);
    }
    draw_standalone_preedit(engine, &mut dl, bounds, theme, shaper);
    // Draw cursor at the WYSIWYG position (only when blink phase is visible)
    if engine.cursor_visible
        && let Some((cx, cy, cw, ch)) =
            engine.visual_cursor_screen_pos().or_else(|| engine.cursor_screen_pos())
    {
        let cursor_x = bounds.x + cx;
        let cursor_y = bounds.y + cy;
        let visual_cw = cw * dpi_scale;
        let cursor_rect =
            ui::core::geom::Rect::new(cursor_x - visual_cw * 0.5, cursor_y, visual_cw, ch);
        dl.fill(cursor_rect, theme.editor.cursor);
    }
    dl
}

fn draw_standalone_preedit(
    engine: &mut PreviewEngine,
    dl: &mut DrawList,
    bounds: ui::Rect,
    theme: &Theme,
    shaper: &mut shaping::Shaper,
) {
    if let Some(preedit) = engine.standalone_preedit_render_data() {
        let preedit_text = preedit.text.to_owned();
        let preedit_cursor = preedit.cursor;
        let preedit_x = preedit.x;
        let preedit_baseline_y = preedit.baseline_y;
        let preedit_font_size = preedit.font_size;
        let cursor_offset = preedit_cursor_offset(&preedit_text, preedit_cursor);
        let cursor_advance = ui::core::text_layout::UiTextLayout::new(
            &preedit_text[..cursor_offset],
            preedit_font_size,
            None,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            false,
            shaper,
        )
        .map_or(0.0, |layout| layout.shaped.width);
        engine.set_standalone_preedit_cursor_advance(cursor_advance);
        dl.text_shaped(
            bounds.x + preedit_x,
            bounds.y + preedit_baseline_y,
            preedit_font_size,
            theme.editor.foreground,
            &preedit_text,
            shaper,
        );
    }
}

fn build_composition_document(
    source: &str,
    style: &MarkdownStyle,
    composing_range: Option<&Range<usize>>,
) -> MarkdownDoc {
    let mut parsed = crate::parser::parse_markdown(source);
    if let Some(composing_range) = composing_range {
        for (event, range) in parsed.events.iter_mut().zip(&parsed.event_ranges) {
            if matches!(event, crate::parser::MarkdownEvent::SoftBreak)
                && composing_range.contains(&range.start)
            {
                *event = crate::parser::MarkdownEvent::HardBreak;
            }
        }
    }
    crate::builder::MarkdownDoc::build_for_editing(&parsed, style, source)
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn render(view: &mut MarkdownEditorView) -> DrawList {
        let mut shaper = shaping::Shaper::new().expect("test font shaper must initialize");
        let source = view.source.clone();
        ViewPlugin::render(
            view,
            &core::document::StringDocView::new(&source),
            ui::Rect::new(0.0, 0.0, 800.0, 600.0),
            &ui::theme::test_theme(),
            &mut shaper,
            1.0,
        )
    }

    fn text(view: &MarkdownEditorView) -> String {
        view.engine()
            .flat_lines()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn composing_selection() -> MarkdownEditorView {
        let mut view = MarkdownEditorView::new();
        view.set_source("old tail".into(), 1);
        for message in [
            PluginMessage::SetCursorByte(3),
            PluginMessage::SetSelAnchorByte(Some(0)),
            PluginMessage::SetSelCursorByte(Some(3)),
            PluginMessage::SetPreedit { text: "新".into(), cursor: Some((3, 3)) },
        ] {
            assert_eq!(view.engine.handle_message_common(&message), Some(true));
        }
        render(&mut view);
        assert_eq!(text(&view), "新 tail");
        view
    }

    fn assert_preedit_matches_commit(view: &mut MarkdownEditorView, expected: &str) {
        render(view);
        assert_eq!(
            text(view),
            expected,
            "preview must follow current selection even before the next IME event"
        );
        view.engine.handle_message_common(&PluginMessage::SetPreedit {
            text: "新".into(),
            cursor: Some((3, 3)),
        });
        render(view);
        assert_eq!(text(view), expected, "identical IME events must not retain a stale selection");
        let cursor_byte = view.engine.edit_ctx.as_ref().expect("active caret").cursor_byte;
        let range = view
            .engine
            .selection_source_range()
            .map_or(cursor_byte..cursor_byte, |(start, end)| start..end);
        let mut committed_source = view.source.clone();
        committed_source.replace_range(range, "新");
        assert_eq!(committed_source, expected, "host commit uses authoritative document selection");
        view.set_source(committed_source, 2);
        view.engine.clear_selection();
        render(view);
        assert_eq!(text(view), expected, "committed document must match the composition preview");
    }

    #[test]
    fn replacement_preedit_navigation_reanchors_before_repeated_ime_event() {
        for (direction, expected_cursor, expected) in [
            (ui::plugin::MoveDirection::Right, 4, "old 新tail"),
            (ui::plugin::MoveDirection::Left, 0, "新old tail"),
        ] {
            for cursor_first in [false, true] {
                let mut view = composing_selection();
                let next = view
                    .engine
                    .visual_move(3, direction, None)
                    .expect("visual navigation resolves");
                assert_eq!(next, expected_cursor);
                if cursor_first {
                    view.engine.handle_message_common(&PluginMessage::SetCursorByte(next));
                    view.engine.handle_message_common(&PluginMessage::SetSelAnchorByte(None));
                    view.engine.handle_message_common(&PluginMessage::SetSelCursorByte(None));
                } else {
                    view.engine.handle_message_common(&PluginMessage::ClearSelection);
                    view.engine.handle_message_common(&PluginMessage::SetCursorByte(next));
                }
                assert_preedit_matches_commit(&mut view, expected);
            }
        }
    }

    #[test]
    fn replacement_preedit_expanded_and_collapsed_selection_matches_commit() {
        for (anchor, cursor, expected) in [(0, 4, "新tail"), (0, 0, "新old tail"), (8, 4, "old 新")]
        {
            let mut view = composing_selection();
            // The host publishes cursor and selection as separate messages.
            view.engine.handle_message_common(&PluginMessage::SetCursorByte(cursor));
            view.engine.handle_message_common(&PluginMessage::SetSelAnchorByte(Some(anchor)));
            view.engine.handle_message_common(&PluginMessage::SetSelCursorByte(Some(cursor)));
            assert_preedit_matches_commit(&mut view, expected);
        }
    }

    #[test]
    fn replacement_preedit_select_all_uses_authoritative_source_extent() {
        let mut view = composing_selection();
        view.engine.handle_message_common(&PluginMessage::SelectAll);
        assert_eq!(view.engine.selection_source_range(), Some((0, 8)));
        assert_preedit_matches_commit(&mut view, "新");
    }

    #[test]
    fn replacement_preedit_merges_selected_source_and_preserves_truth() {
        for (source, range, replacement, expected) in [
            ("one two one", 0..3, "新", "新 two one"),
            ("old", 0..3, "中\n文", "中\n文"),
            ("abc\ndef", 1..5, "中😀", "a中😀ef"),
            ("abc\n\ndef", 1..6, "新", "a新ef"),
            ("中😀文 tail", 0..10, "替换", "替换 tail"),
            ("**ab\n\ncd**", 4..6, "新", "**ab新cd**"),
        ] {
            for (anchor, cursor) in [(range.start, range.end), (range.end, range.start)] {
                let mut view = MarkdownEditorView::new();
                view.set_source(source.to_owned(), 1);
                view.engine.handle_set_cursor_byte(cursor);
                render(&mut view);
                view.engine.set_sel_anchor_byte(Some(anchor));
                view.engine.set_sel_cursor_byte(Some(cursor));
                view.engine.set_preedit_text(
                    replacement.to_owned(),
                    Some((replacement.len(), replacement.len())),
                );
                render(&mut view);
                assert_eq!(text(&view), expected, "source={source:?}, cursor={cursor}");
                assert_eq!(view.source, source);
                assert_eq!(
                    view.engine.edit_ctx.as_ref().expect("cursor was set").cursor_byte,
                    cursor
                );
                assert_eq!(view.engine.selection_source_range(), Some((range.start, range.end)));
            }
        }
    }

    #[test]
    fn replacement_preedit_caret_snaps_inside_combining_grapheme() {
        let mut view = MarkdownEditorView::new();
        view.set_source("old tail".into(), 1);
        view.engine.handle_set_cursor_byte(3);
        view.engine.set_sel_anchor_byte(Some(0));
        view.engine.set_sel_cursor_byte(Some(3));
        view.engine.set_preedit_text("a\u{301}b".into(), Some((1, 1)));
        render(&mut view);
        let actual = view
            .engine
            .visual_cursor_screen_pos()
            .expect("IME caret must resolve at grapheme start");
        let first = &view.engine.flat_lines()[0];
        assert!((actual.0 - first.rect.x).abs() < 0.01);
    }

    #[test]
    fn replacement_preedit_candidate_and_hits_use_composition_layout() {
        let mut view = MarkdownEditorView::new();
        view.set_source("abc\n\ndef tail".into(), 1);
        view.engine.handle_set_cursor_byte(6);
        view.engine.set_sel_anchor_byte(Some(1));
        view.engine.set_sel_cursor_byte(Some(6));
        view.engine.set_preedit_text("中\n😀".into(), Some((8, 8)));
        let commands = render(&mut view);
        let caret = view.engine.visual_cursor_screen_pos().expect("composition caret exists");
        let lines = view.engine.flat_lines();
        assert_eq!(lines.len(), 2);
        assert!(caret.1 > lines[0].rect.y + lines[0].font_size);
        let query = view.engine.query_common(&PluginQuery::CursorScreenPos(6));
        assert!(
            matches!(query, Some(PluginResponse::CursorScreenRect(Some(rect))) if rect == caret)
        );
        let line = &lines[1];
        let hit = view.engine.hit_test_byte(
            line.rect.x + crate::layout::grapheme_x(line, 3),
            line.rect.y + line.rect.h * 0.5,
            0.0,
            0.0,
        );
        assert_eq!(hit, Some(8), "suffix coordinates map back after deleted source");
        let underline_count = commands.cmds.iter().filter(|command| matches!(command,
            ui::core::paint::DrawCmd::FillRect { rect, .. } if (rect.h - 1.0).abs() < 0.01 && rect.w > 0.0
        )).count();
        assert!(underline_count >= 2, "each composition line has an underline");
        assert!(view.engine.selection_highlights([1.0; 4]).cmds.is_empty());
    }

    #[test]
    fn replacement_preedit_cancel_restores_selected_document() {
        let source = "abc\n\ndef";
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        view.engine.handle_set_cursor_byte(6);
        view.engine.set_sel_anchor_byte(Some(1));
        view.engine.set_sel_cursor_byte(Some(6));
        render(&mut view);
        let original = text(&view);
        view.engine.set_preedit_text("新".into(), Some((3, 3)));
        render(&mut view);
        assert_eq!(text(&view), "a新ef");
        view.engine.set_preedit_text(String::new(), None);
        render(&mut view);
        assert_eq!(text(&view), original);
        assert_eq!(view.engine.selection_source_range(), Some((1, 6)));
        assert_eq!(view.source, source);
    }

    #[test]
    fn replacement_preedit_same_text_commit_ends_composition() {
        let mut view = MarkdownEditorView::new();
        view.set_source("one two".into(), 1);
        view.engine.handle_set_cursor_byte(3);
        view.engine.set_sel_anchor_byte(Some(0));
        view.engine.set_sel_cursor_byte(Some(3));
        view.engine.set_preedit_text("one".into(), Some((3, 3)));
        render(&mut view);
        view.set_source("one two".into(), 2);
        assert!(view.engine.replacement_preedit.is_none());
        assert!(view.engine.edit_ctx.as_ref().expect("cursor remains").preedit_text.is_none());
    }

    #[test]
    fn replacement_preedit_commit_source_update_does_not_duplicate_composition() {
        let mut view = MarkdownEditorView::new();
        view.set_source("one two".into(), 1);
        view.engine.handle_set_cursor_byte(3);
        view.engine.set_sel_anchor_byte(Some(0));
        view.engine.set_sel_cursor_byte(Some(3));
        view.engine.set_preedit_text("新".into(), Some((3, 3)));
        render(&mut view);
        assert_eq!(text(&view), "新 two");
        view.set_source("新 two".into(), 2);
        view.engine.clear_selection();
        view.engine.handle_set_cursor_byte(3);
        render(&mut view);
        assert_eq!(text(&view), "新 two");
    }
}
