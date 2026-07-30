//! Reshape pipeline: invalidation, zoom, drain, submit, post-shape update.
//! Methods on `impl App`, extracted from app.rs.

use crate::app::App;
use crate::reshape_worker::ReshapeRequest;

impl App {
    pub(crate) fn invalidate_reshape(&mut self) {
        self.editor_runtime.invalidate_reshape();
    }

    pub(crate) fn apply_zoom(&mut self, logical_font_size: f32) -> crate::app_effect::AppEffect {
        let logical_font_size = logical_font_size.clamp(6.0, 72.0);
        self.settings.set_font_size(logical_font_size);
        self.editor_runtime.update_settings(self.settings.clone());
        let metrics = self.ui_metrics();
        let mut render_resources = self.editor_runtime.take_render_resources();
        if let Some(text) = render_resources.text.as_mut() {
            text.shaper.set_font_size(metrics.font_size);
        }
        self.editor_runtime.restore_render_resources(render_resources);
        let tab_ids = self.editor_tab_ids_in_order();
        for tab_id in tab_ids.iter().copied() {
            if let Some(mut tab) = self.tab_session_mut(tab_id) {
                tab.invalidate_render_cache_all();
            }
        }
        let screen_height = self.screen_height();
        let visible_rows = self.visible_rows(screen_height);
        let viewport_height = self.visible_height_lines(screen_height);
        for tab_id in tab_ids {
            if let Some(mut tab) = self.tab_session_mut(tab_id) {
                tab.resize_presentation(visible_rows, viewport_height);
                tab.clamp_scroll_anchor(metrics.line_height);
                tab.derive_scroll_top(metrics.line_height);
            }
        }
        crate::app_effect::AppEffect::RESHAPE.merge(crate::app_effect::AppEffect::PERSIST_SETTINGS)
    }

    /// Drain reshape worker results and merge into active dv's DisplayLineMap
    pub(crate) fn drain_reshape_results(&mut self) {
        if !self.editor_runtime.has_reshape_worker() {
            return;
        }
        let _t0 = std::time::Instant::now();
        let Some(active_index) = self.active_editor_index() else {
            return;
        };
        let lh = self.ui_metrics().line_height;
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let results = self
            .editor_runtime
            .drain_reshape_results(32)
            .into_iter()
            .filter(|result| self.editor_runtime.accepts_reshape_result(result, active_index))
            .collect::<Vec<_>>();
        let result_count = results.len();
        let mut cleared_lines = Vec::new();
        let Some((tree_dirty, updated)) = self.tab_session_mut(tab_id).map(|mut tab| {
            let mut tree_dirty = false;
            let mut updated = false;
            for r in results {
                // Check if reshape result is materially identical to the current entry.
                // content_hash covers the line identity (byte_offset + byte_length)
                // and rendering params (viewport_width + font_size). If both hash and
                // visual_line_count are unchanged, skip the update and cache invalidation
                // to prevent unnecessary re-shaping and tree rebuilds that cause cursor drift.
                let is_unchanged = tab
                    .display_map_entry(r.doc_line)
                    .map(|old| {
                        old.content_hash == r.entry.content_hash
                            && old.visual_line_count == r.entry.visual_line_count
                    })
                    .unwrap_or(false);

                if is_unchanged {
                    cleared_lines.push(r.doc_line);
                    updated = true;
                    continue;
                }

                let height_changed = tab
                    .display_map_entry(r.doc_line)
                    .map(|old| old.visual_line_count != r.entry.visual_line_count)
                    .unwrap_or(true);

                tab.update_display_map_entry(r.doc_line, r.entry);
                cleared_lines.push(r.doc_line);
                if height_changed {
                    tree_dirty = true;
                }
                tab.invalidate_render_cache_line(r.doc_line);
                updated = true;
            }
            if tree_dirty {
                tab.rebuild_display_map();
                // Clamp anchor then derive scroll_top from stable anchor.
                // anchor is SOT — tree mapping changed but anchor.doc_line stays.
                tab.clamp_scroll_anchor(lh);
                tab.derive_scroll_top(lh);
            }
            (tree_dirty, updated)
        }) else {
            return;
        };
        for line in cleared_lines {
            self.editor_runtime.clear_reshape_pending(line);
        }
        self.needs_redraw |= tree_dirty || updated;
        let _elapsed = _t0.elapsed().as_micros();
        if result_count > 0 || tree_dirty {
            eprintln!(
                "[perf:drain] results={} tree_dirty={} elapsed={}us",
                result_count, tree_dirty, _elapsed
            );
        }
    }

    /// Submit reshape requests for lines just beyond the visible viewport.
    pub(crate) fn submit_reshape_ahead(&mut self) {
        if self.editor_runtime.take_skip_next_reshape_submit() {
            return;
        }
        if !self.editor_runtime.has_reshape_worker() {
            return;
        }
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let Some(active_index) = self.active_editor_index() else {
            return;
        };
        let _st0 = std::time::Instant::now();
        let mut _submitted = 0usize;
        let mut _skipped = 0usize;
        let viewport_width = self
            .active_tab_session()
            .map(|tab| self.viewport_content_width(tab.document))
            .unwrap_or(1.0);
        let font_size = self.ui_metrics().font_size;
        let (range, anchor_doc, document_line_count) = {
            let Some(tab) = self.tab_session(tab_id) else {
                return;
            };
            let document = tab.document;
            let range = tab.visible_doc_range_from_anchor(self.ui_metrics().line_height);
            // Always include anchor.doc_line so it gets accurate VL on first reshape,
            // even when scroll_top is derived from placeholder estimates.
            let anchor_doc =
                tab.scroll_anchor_doc_line().min(document.line_count().saturating_sub(1));
            (range, anchor_doc, document.line_count())
        };

        // Debounce: if anchor jumped more than the ahead buffer, the previously
        // submitted range has no overlap with the new viewport. Skip to avoid
        // flooding the reshape worker during rapid scrollbar drag.
        if !self.editor_runtime.should_submit_reshape_anchor(anchor_doc, std::time::Instant::now())
        {
            return;
        }
        self.editor_runtime.mark_reshape_anchor_submitted(anchor_doc);

        let start_doc = range
            .start
            .saturating_sub(appkit_shell::editor_runtime::RESHAPE_AHEAD_LINES)
            .min(anchor_doc.saturating_sub(appkit_shell::editor_runtime::RESHAPE_AHEAD_LINES));
        let ahead_end = (range.end + appkit_shell::editor_runtime::RESHAPE_AHEAD_LINES)
            .max(anchor_doc + appkit_shell::editor_runtime::RESHAPE_AHEAD_LINES)
            .min(document_line_count);
        let generation = self.editor_runtime.reshape_generation();
        let max_line_bytes = self.settings.max_line_bytes_for_shaping;
        let requests: Vec<(usize, ReshapeRequest)> = {
            let Some(tab) = self.tab_session(tab_id) else {
                return;
            };
            let document = tab.document;
            (start_doc..ahead_end)
                .filter_map(|dl| {
                    // Skip lines that are already shaped and up-to-date
                    let is_up_to_date = if let Some(entry) = tab.display_map_entry(dl) {
                        let off = document.line_byte_offset(dl).unwrap_or(0);
                        let len = document.line_byte_length(dl).unwrap_or(0);
                        let current_hash = crate::content_hash::content_hash(
                            off,
                            len as u32,
                            viewport_width,
                            font_size,
                        );
                        entry.content_hash != 0 && entry.content_hash == current_hash
                    } else {
                        false
                    };

                    if is_up_to_date || self.editor_runtime.reshape_pending(dl) {
                        _skipped += 1;
                        return None;
                    }

                    let line_bytes = document.doc_line_bytes(dl)?;
                    let off = document.line_byte_offset(dl).unwrap_or(0);
                    let len = document.line_byte_length(dl).unwrap_or(0);
                    Some((
                        dl,
                        ReshapeRequest {
                            generation,
                            doc_line: dl,
                            byte_offset: off,
                            byte_length: len as u32,
                            line_bytes: std::sync::Arc::from(line_bytes.as_ref()),
                            viewport_width,
                            font_size,
                            max_line_bytes,
                            dv_idx: active_index,
                        },
                    ))
                })
                .collect()
        };
        for (doc_line, request) in requests {
            if !self.editor_runtime.mark_reshape_pending(doc_line) {
                _skipped += 1;
                continue;
            }
            if self.editor_runtime.submit_reshape(request) {
                _submitted += 1;
            } else {
                self.editor_runtime.clear_reshape_pending(doc_line);
            }
        }
        let _selapsed = _st0.elapsed().as_micros();
        if _selapsed > 500 || _submitted > 0 {
            eprintln!(
                "[perf:submit] submitted={} skipped={} elapsed={}us",
                _submitted, _skipped, _selapsed
            );
        }
    }

    pub(crate) fn post_shape_update(&mut self) {
        let lh = self.ui_metrics().line_height;
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        let mut needs_redraw = false;

        // Autoscroll: ensure cursor is visible using visual-line precision.
        {
            let cursor_doc_line = tab.document.cursor_line_cached();
            let cursor_offset_now = tab.document.cursor().offset;
            let cursor_moved = cursor_offset_now != tab.last_cursor_offset();
            tab.set_last_cursor_offset(cursor_offset_now);

            if cursor_moved {
                let visible_range = tab.visible_doc_range_from_anchor(lh);
                let anchor = tab.scroll_anchor_doc_line();

                if cursor_doc_line < anchor {
                    // Cursor above viewport: scroll up
                    tab.set_scroll_anchor(cursor_doc_line, 0.0);
                    tab.clamp_scroll_anchor(lh);
                    tab.derive_scroll_top(lh);
                    needs_redraw = true;
                } else if cursor_doc_line >= visible_range.end {
                    // Cursor below viewport: scroll down
                    let visible_count = visible_range.len().max(1);
                    let new_anchor =
                        cursor_doc_line.saturating_sub(visible_count.saturating_sub(1));
                    tab.set_scroll_anchor(new_anchor, 0.0);
                    tab.clamp_scroll_anchor(lh);
                    tab.derive_scroll_top(lh);
                    needs_redraw = true;
                }
            }
        }
        self.needs_redraw |= needs_redraw;
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;
    use crate::plugins::editor::EditorPlugin;
    use crate::render_cache::{CachedLine, GlyphInstance};

    use crate::ui_shell::ShellInputs;
    use ui::core::{NoopMeasure, Screen};

    // ── invalidate_reshape tests ───────────────────────────────────

    #[test]
    fn invalidate_reshape_bumps_generation() {
        let mut app = App::new(None);
        let gen_before = app.editor_runtime.reshape_generation();
        app.invalidate_reshape();
        assert_eq!(app.editor_runtime.reshape_generation(), gen_before + 1);
    }

    #[test]
    fn invalidate_reshape_clears_pending() {
        let mut app = App::new(None);
        assert!(app.editor_runtime.mark_reshape_pending(42));
        assert!(app.editor_runtime.mark_reshape_pending(100));
        app.invalidate_reshape();
        assert!(!app.editor_runtime.reshape_pending(42));
        assert!(!app.editor_runtime.reshape_pending(100));
    }

    #[test]
    fn invalidate_reshape_idempotent_on_empty_pending() {
        let mut app = App::new(None);
        app.invalidate_reshape();
        app.invalidate_reshape();
        assert_eq!(app.editor_runtime.reshape_generation(), 2);
    }

    // ── Zoom helpers ───────────────────────────────────────────────

    fn sim_zoom_in(app: &mut App) -> crate::app_effect::AppEffect {
        app.apply_zoom(app.settings.font_size + 1.0)
    }

    fn sim_zoom_out(app: &mut App) -> crate::app_effect::AppEffect {
        app.apply_zoom((app.settings.font_size - 1.0).max(6.0))
    }

    fn sim_zoom_reset(app: &mut App) -> crate::app_effect::AppEffect {
        app.apply_zoom(15.0)
    }

    // ── apply_zoom tests ───────────────────────────────────────────

    #[test]
    fn apply_zoom_sets_font_size() {
        let mut app = App::new(None);
        app.apply_zoom(20.0);
        assert_eq!(app.settings.font_size, 20.0);
        assert_eq!(app.editor_runtime.settings_snapshot().font_size, 20.0);
    }

    #[test]
    fn apply_zoom_returns_reshape_without_applying_it() {
        let mut app = App::new(None);
        let generation = app.editor_runtime.reshape_generation();
        app.needs_redraw = false;

        let effect = app.apply_zoom(20.0);

        assert!(effect.reshape);
        assert!(effect.redraw);
        assert_eq!(app.settings.font_size, 20.0);
        assert_eq!(app.editor_runtime.reshape_generation(), generation);
        assert!(!app.needs_redraw);
    }

    #[test]
    fn apply_zoom_returns_persist_settings() {
        let mut app = App::new(None);
        let effect = app.apply_zoom(22.0);
        assert!(effect.persist_settings);
    }

    #[test]
    fn apply_zoom_clamped_does_not_panic() {
        let mut app = App::new(None);
        app.apply_zoom(6.0);
        app.apply_zoom(72.0);
        app.apply_zoom(1000.0);
    }

    // ── screen metrics tests ───────────────────────────────────────

    #[test]
    fn screen_width_returns_fallback_without_gpu() {
        let app = App::new(None);
        assert_eq!(app.screen_width(), 800.0);
    }

    #[test]
    fn screen_height_returns_fallback_without_gpu() {
        let app = App::new(None);
        assert_eq!(app.screen_height(), 600.0);
    }

    #[test]
    fn viewport_content_width_accounts_for_margins() {
        let mut app = App::new(None);
        app.settings.word_wrap = true;
        let dv = crate::document_view::DocumentView::new(vec!["line".to_string()], 40, 40.0);
        let vw = app.viewport_content_width(&dv);
        let left_margin = app.editor_left_margin(dv.line_count());
        let editor_rect = app.ui_shell.editor_rect();
        let expected = (editor_rect.x + editor_rect.w - left_margin).max(1.0);
        assert_eq!(vw, expected);
    }

    #[test]
    fn viewport_content_width_matches_render_width_with_left_chrome() {
        let mut app = App::new(None);
        app.settings.word_wrap = true;
        app.ui_shell.mark_layout_initialized_for_test();

        let screen = Screen::new(2084.0, 900.0);
        let metrics = app.ui_metrics();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: true,
            sidebar_thickness: 468.0,
            scrollbar_thickness: metrics.scrollbar_reserve,
            toc_visible: false,
            toc_thickness: 0.0,
            metrics,
            sidebar_settings: Default::default(),
        };
        let mut measure = NoopMeasure;
        app.ui_shell.update_frame(screen, &app.current_theme, &mut measure, &inputs);

        let dv = crate::document_view::DocumentView::new(vec!["line".to_string()], 40, 40.0);
        let left_margin = app.editor_left_margin(dv.line_count());
        let expected = crate::render_pipeline::render_viewport_width(
            screen.w,
            left_margin,
            &app.ui_metrics(),
            app.settings.word_wrap,
        );

        assert_eq!(app.viewport_content_width(&dv), expected);
    }

    fn make_glyph() -> GlyphInstance {
        GlyphInstance {
            x: 0.0,
            y: 0.0,
            bearing_x: 0.0,
            bearing_y: 0.0,
            width: 8.0,
            height: 14.0,
            uv: [0.0, 0.0, 0.1, 0.1],
            atlas_page: 0,
            highlight_kind: 0,
        }
    }

    #[test]
    fn zoom_in_increases_font_size() {
        let mut app = App::new(None);
        let orig = app.settings.font_size;
        sim_zoom_in(&mut app);
        assert_eq!(app.settings.font_size, orig + 1.0);
    }

    #[test]
    fn zoom_in_updates_line_height() {
        let mut app = App::new(None);
        sim_zoom_in(&mut app);
        // Zoom in makes font size 15.0 -> line height 15 * 1.618 = 24.27
        assert_eq!(app.settings.line_height, app.settings.font_size * 1.618);
    }

    #[test]
    fn zoom_out_decreases_font_size() {
        let mut app = App::new(None);
        let orig = app.settings.font_size;
        sim_zoom_out(&mut app);
        assert_eq!(app.settings.font_size, (orig - 1.0).max(6.0));
    }

    #[test]
    fn zoom_out_clamped_to_min_6() {
        let mut app = App::new(None);
        app.settings.font_size = 6.5;
        sim_zoom_out(&mut app);
        assert_eq!(app.settings.font_size, 6.0);
        sim_zoom_out(&mut app);
        assert_eq!(app.settings.font_size, 6.0);
    }

    #[test]
    fn zoom_reset_restores_default() {
        let mut app = App::new(None);
        sim_zoom_in(&mut app);
        sim_zoom_reset(&mut app);
        assert_eq!(app.settings.font_size, 15.0);
        assert_eq!(app.settings.line_height, 24.27);
    }

    #[test]
    fn zoom_returns_reshape_effect() {
        let mut app = App::new(None);
        app.needs_redraw = false;
        let effect = sim_zoom_in(&mut app);
        assert!(effect.reshape);
        assert!(effect.redraw);
        assert!(!app.needs_redraw);
        let effect = sim_zoom_out(&mut app);
        assert!(effect.reshape);
        let effect = sim_zoom_reset(&mut app);
        assert!(effect.reshape);
    }

    #[test]
    fn zoom_invalidates_render_cache() {
        let mut app = App::new(None);
        // Add at least one document so views is non-empty
        app.push_entry_for_test(
            crate::document_view::DocumentView::new(vec![String::new()], 40, 40.0),
            Box::new(EditorPlugin::new()),
        );
        let g = make_glyph();
        {
            let mut tab = app.active_tab_session_mut().unwrap();
            tab.display_mut().render_cache.insert(
                0,
                CachedLine {
                    instances: vec![g.clone()],
                    line_number_glyphs: vec![g.clone()],
                    atlas_generation: 1,
                    visual_line_count: 1,
                    content_hash: 42,
                    visual_lines: vec![(0, 1, 8.0)],
                    visual_line_instance_starts: vec![0],
                    cluster_data: vec![(0, 1, 8.0)],
                    subset_start: 0,
                },
            );
            assert!(!tab.display().render_cache.is_empty());
        }
        sim_zoom_in(&mut app);
        assert!(
            app.active_tab_session().unwrap().display().render_cache.is_empty(),
            "zoom_in should invalidate cache"
        );
    }

    #[test]
    fn zoom_increments_settings_version() {
        let mut app = App::new(None);
        let v1 = app.settings.version;
        sim_zoom_in(&mut app);
        assert_eq!(app.settings.version, v1 + 1);
        sim_zoom_out(&mut app);
        assert_eq!(app.settings.version, v1 + 2);
        sim_zoom_reset(&mut app);
        assert_eq!(app.settings.version, v1 + 3);
    }

    // ── Retina zoom tests ────────────────────────────────────────────

    #[test]
    fn zoom_uses_logical_points_at_retina_scale() {
        let mut app = App::new(None);
        app.update_scale_factor(2.0);

        app.apply_zoom(16.0);
        assert_eq!(app.settings.font_size, 16.0);
        assert_eq!(app.ui_metrics().font_size, 32.0);

        app.apply_zoom(15.0);
        assert_eq!(app.settings.font_size, 15.0);
        assert_eq!(app.ui_metrics().font_size, 30.0);
    }

    #[test]
    fn zoom_out_clamps_logical_size_at_six() {
        let mut app = App::new(None);
        app.update_scale_factor(2.0);
        app.apply_zoom(6.0);
        assert_eq!(app.settings.font_size, 6.0);
        assert_eq!(app.ui_metrics().font_size, 12.0);
    }

    // ── Compat facade zoom test ────────────────────────────────────────

    #[test]
    fn logical_zoom_step_is_one_at_two_x_dpi() {
        let mut app = App::new(None);
        app.update_scale_factor(2.0);
        let before = app.persisted_font_size();

        app.apply_zoom(before + 1.0);

        assert_eq!(app.persisted_font_size(), before + 1.0);
    }

    // ── Viewport update tests ──────────────────────────────────────────

    /// After zoom in, visible_rows should decrease (fewer lines fit on screen).
    #[test]
    fn zoom_in_reduces_visible_rows() {
        let mut app = App::new(None);
        let screen_h = 800.0;
        let before = app.visible_rows(screen_h);
        sim_zoom_in(&mut app);
        let after = app.visible_rows(screen_h);
        assert!(after < before, "zoom in should reduce visible_rows: {} -> {}", before, after);
    }

    /// After zoom out, visible_rows should increase (more lines fit on screen).
    #[test]
    fn zoom_out_increases_visible_rows() {
        let mut app = App::new(None);
        // Start at a very large font so there's room to zoom out and definitely change the number of rows
        app.settings.set_font_size(30.0);
        let screen_h = 800.0;
        let before = app.visible_rows(screen_h);
        sim_zoom_out(&mut app);
        sim_zoom_out(&mut app);
        sim_zoom_out(&mut app);
        let after = app.visible_rows(screen_h);
        assert!(after > before, "zoom out should increase visible_rows: {} -> {}", before, after);
    }

    /// After zoom reset, visible_rows should return to the value for font_size=15.
    #[test]
    fn zoom_reset_restores_visible_rows() {
        let mut app = App::new(None);
        let screen_h = 800.0;
        // Change font size significantly
        app.settings.set_font_size(30.0);
        sim_zoom_reset(&mut app);
        // After reset: font_size=15, line_height=15*1.618=24.27
        let after = app.visible_rows(screen_h);
        let expected = ((screen_h
            - if app.settings.show_status_bar { app.settings.status_bar_height } else { 0.0 }
            - app.content_top_offset())
            / app.settings.line_height)
            .max(1.0) as f64;
        let expected_rows = expected.floor() as usize;
        assert_eq!(
            after, expected_rows,
            "zoom reset should restore visible_rows for font_size=14: expected {}, got {}",
            expected_rows, after
        );
    }

    /// Zoom in should reduce viewport viewport_height for the active document view.
    #[test]
    fn zoom_in_updates_document_viewport() {
        let screen_h = 800.0;
        let mut app = App::new(None);
        // Create a document view directly (no GPU needed, test the viewport update path)
        let visible_rows = app.visible_rows(screen_h);
        let viewport_height = app.visible_height_lines(screen_h);
        let mut dv = crate::document_view::DocumentView::new(
            vec!["line1".to_string(), "line2".to_string()],
            visible_rows,
            viewport_height,
        );
        let old_visible = dv.presentation.display.viewport.visible_rows;
        let old_height = dv.presentation.display.viewport.viewport_height;
        // Simulate zoom in font size change and viewport update
        let new_size = app.settings.font_size + 1.0;
        app.settings.set_font_size(new_size);
        let new_visible = app.visible_rows(screen_h);
        let new_height = app.visible_height_lines(screen_h);
        dv.resize(new_visible, new_height);
        assert!(
            dv.presentation.display.viewport.visible_rows < old_visible,
            "zoom in should reduce viewport visible_rows: {} -> {}",
            old_visible,
            dv.presentation.display.viewport.visible_rows
        );
        assert!(
            dv.presentation.display.viewport.viewport_height < old_height,
            "zoom in should reduce viewport_height: {} -> {}",
            old_height,
            dv.presentation.display.viewport.viewport_height
        );
    }

    /// Zoom out should increase viewport viewport_height for the active document view.
    #[test]
    fn zoom_out_updates_document_viewport() {
        let screen_h = 800.0;
        let mut app = App::new(None);
        // Start large
        app.settings.set_font_size(30.0);
        let visible_rows = app.visible_rows(screen_h);
        let viewport_height = app.visible_height_lines(screen_h);
        let mut dv = crate::document_view::DocumentView::new(
            vec!["line1".to_string(), "line2".to_string()],
            visible_rows,
            viewport_height,
        );
        let old_visible = dv.presentation.display.viewport.visible_rows;
        let old_height = dv.presentation.display.viewport.viewport_height;
        // Simulate zoom out
        let new_size = (app.settings.font_size - 2.0).max(6.0);
        app.settings.set_font_size(new_size);
        let new_visible = app.visible_rows(screen_h);
        let new_height = app.visible_height_lines(screen_h);
        dv.resize(new_visible, new_height);
        assert!(
            dv.presentation.display.viewport.visible_rows > old_visible,
            "zoom out should increase viewport visible_rows: {} -> {}",
            old_visible,
            dv.presentation.display.viewport.visible_rows
        );
        assert!(
            dv.presentation.display.viewport.viewport_height > old_height,
            "zoom out should increase viewport_height: {} -> {}",
            old_height,
            dv.presentation.display.viewport.viewport_height
        );
    }

    /// visible_height_lines correctly accounts for status bar and tab bar.
    #[test]
    fn visible_height_lines_accounts_for_bars() {
        let mut app = App::new(None);
        let screen_h = 800.0;
        let metrics = app.ui_metrics();
        // show_status_bar defaults to false, so status bar height is 0
        let status_h = if app.settings.show_status_bar { metrics.status_bar_height } else { 0.0 };
        let top_h = app.content_top_offset();
        let available_h = screen_h - status_h - top_h;
        let expected = (available_h / metrics.line_height).max(1.0) as f64;
        assert_eq!(app.visible_height_lines(screen_h), expected);

        // With status bar enabled, it should account for the height
        app.settings.show_status_bar = true;
        let metrics = app.ui_metrics();
        let status_h = metrics.status_bar_height;
        let available_h2 = screen_h - status_h - top_h;
        let expected2 = (available_h2 / metrics.line_height).max(1.0) as f64;
        assert_eq!(app.visible_height_lines(screen_h), expected2);
    }
}
