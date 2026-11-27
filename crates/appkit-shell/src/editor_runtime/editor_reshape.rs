//! Viewport-driven asynchronous source layout.

use std::sync::Arc;
use std::time::Instant;

use super::EditorRuntime;
use super::reshape_session::RESHAPE_AHEAD_LINES;
use crate::render_pipeline::{
    document_uses_verbatim_source_spacing, protected_ranges_for_line, spacing_mode_for_line,
};
use crate::reshape_worker::{ReshapeRequest, ReshapeResult, content_hash_for_layout};
use crate::tab_session::{TabSession, TabSessionMut};

const RESHAPE_RESULTS_PER_FRAME: usize = 32;
const SLOW_RESHAPE_SUBMIT_MICROS: u128 = 500;

impl EditorRuntime {
    /// Applies completed background layouts for the active tab.
    pub fn apply_active_reshape_results(&mut self) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let Some(active_index) = self.tab_index(tab_id) else {
            return false;
        };
        let started_at = Instant::now();
        let line_height = self.editor_line_height();
        let (model, reshape) = (&mut self.model_session, &mut self.reshape_session);
        let Some(mut tab) = model.tab_session_mut(tab_id) else {
            return false;
        };
        reshape.observe_source(tab_id, tab.document.content_revision());
        let mut applied = 0;
        let mut tree_dirty = false;
        for result in reshape.drain_completed(RESHAPE_RESULTS_PER_FRAME) {
            if !reshape.accepts(&result, active_index) {
                continue;
            }
            reshape.clear_pending(result.doc_line);
            tree_dirty |= apply_reshape_result(&mut tab, result);
            applied += 1;
        }
        if tree_dirty {
            tab.rebuild_display_map();
            tab.clamp_scroll_anchor(line_height);
            tab.derive_scroll_top(line_height);
        }
        if applied > 0 {
            eprintln!(
                "[perf:drain] results={applied} tree_dirty={tree_dirty} elapsed={}us",
                started_at.elapsed().as_micros()
            );
        }
        applied > 0
    }

    /// Schedules source layout near the active viewport for the supplied content width.
    pub fn submit_active_reshape_ahead(&mut self, viewport_width: f32) {
        if self.reshape_session.take_skip_next_submit() || !self.reshape_session.has_worker() {
            return;
        }
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let Some(active_index) = self.tab_index(tab_id) else {
            return;
        };
        let line_height = self.editor_line_height();
        let metrics =
            ui::settings::UiMetrics::from_settings(&self.settings, self.scale_factor() as f32);
        let Some(tab) = self.model_session.tab_session(tab_id) else {
            return;
        };
        let viewport_lines = tab.visible_doc_range_from_anchor(line_height);
        let document_lines = tab.document.line_count();
        let anchor = tab.scroll_anchor_doc_line().min(document_lines.saturating_sub(1));
        let started_at = Instant::now();
        let reshape = &mut self.reshape_session;
        reshape.observe_source(tab_id, tab.document.content_revision());
        if !reshape.should_submit_ahead(anchor, started_at) {
            return;
        }
        reshape.mark_submitted(anchor);

        let first_line = viewport_lines.start.min(anchor).saturating_sub(RESHAPE_AHEAD_LINES);
        let past_last_line =
            viewport_lines.end.max(anchor).saturating_add(RESHAPE_AHEAD_LINES).min(document_lines);
        let layout = ReshapeLayout {
            font_size: metrics.font_size,
            font_family: self.settings.font_family.clone().into(),
            viewport_width,
            max_line_bytes: self.settings.max_line_bytes_for_shaping,
            spacing_mode: self.settings.text_spacing_mode,
            active_index,
            generation: reshape.generation(),
        };
        let mut submitted = 0;
        for line in first_line..past_last_line {
            if reshape.is_pending(line) {
                continue;
            }
            let Some(request) = layout.request(&tab, line) else {
                continue;
            };
            if reshape.mark_pending(line) {
                if reshape.submit(request) {
                    submitted += 1;
                } else {
                    reshape.clear_pending(line);
                }
            }
        }
        let elapsed_micros = started_at.elapsed().as_micros();
        if submitted > 0 || elapsed_micros > SLOW_RESHAPE_SUBMIT_MICROS {
            let skipped = past_last_line.saturating_sub(first_line + submitted);
            eprintln!(
                "[perf:submit] submitted={submitted} skipped={skipped} elapsed={elapsed_micros}us"
            );
        }
    }

    /// Reveals a changed source caret after layout, keeping the viewport anchor stable.
    pub fn reveal_active_caret_after_shape(&mut self) -> bool {
        let line_height = self.editor_line_height();
        let Some(mut tab) = self.active_tab_id().and_then(|tab_id| self.tab_session_mut(tab_id))
        else {
            return false;
        };
        let caret = tab.document.cursor().offset;
        if caret == tab.last_cursor_offset() {
            return false;
        }
        tab.set_last_cursor_offset(caret);
        let previous_anchor = tab.display().viewport.scroll_anchor;
        tab.ensure_cursor_visible(line_height);
        let current_anchor = tab.display().viewport.scroll_anchor;
        current_anchor.doc_line != previous_anchor.doc_line
            || current_anchor.pixel_offset != previous_anchor.pixel_offset
    }
}

fn apply_reshape_result(tab: &mut TabSessionMut<'_>, result: ReshapeResult) -> bool {
    let previous = tab.display_map_entry(result.doc_line);
    if previous.is_some_and(|entry| {
        entry.content_hash == result.entry.content_hash
            && entry.visual_line_count == result.entry.visual_line_count
    }) {
        return false;
    }
    let height_changed =
        previous.is_none_or(|entry| entry.visual_line_count != result.entry.visual_line_count);
    tab.update_display_map_entry(result.doc_line, result.entry);
    tab.invalidate_render_cache_line(result.doc_line);
    height_changed
}

struct ReshapeLayout {
    font_size: f32,
    font_family: Arc<str>,
    viewport_width: f32,
    max_line_bytes: usize,
    spacing_mode: ui::typography::TextSpacingMode,
    active_index: usize,
    generation: u64,
}

impl ReshapeLayout {
    fn request(&self, tab: &TabSession<'_>, line: usize) -> Option<ReshapeRequest> {
        let document = tab.document;
        let line_bytes = document.doc_line_bytes(line)?;
        let byte_offset = document.line_byte_offset(line)?;
        let byte_length = document.line_byte_length(line)?;
        let spacing = &tab.runtime.presentation.source_spacing;
        let protected_ranges =
            protected_ranges_for_line(&spacing.protected_ranges, byte_offset, line_bytes.len());
        let spacing_mode = spacing_mode_for_line(
            self.spacing_mode,
            document_uses_verbatim_source_spacing(document),
            &line_bytes,
        );
        let highlight_anchor = document.language.map(|_| byte_offset);
        let content_hash = content_hash_for_layout(
            &line_bytes,
            highlight_anchor,
            self.viewport_width,
            self.font_size,
            spacing_mode,
            &protected_ranges,
            spacing.semantic_version,
            &self.font_family,
        );
        if tab
            .display_map_entry(line)
            .is_some_and(|entry| entry.content_hash != 0 && entry.content_hash == content_hash)
        {
            return None;
        }
        Some(ReshapeRequest {
            font_family: self.font_family.clone(),
            generation: self.generation,
            doc_line: line,
            byte_offset,
            highlight_depends_on_position: highlight_anchor.is_some(),
            byte_length: byte_length as u32,
            line_bytes: Arc::from(line_bytes.as_ref()),
            viewport_width: self.viewport_width,
            font_size: self.font_size,
            max_line_bytes: self.max_line_bytes,
            spacing_mode,
            protected_ranges: Arc::from(protected_ranges),
            semantic_version: spacing.semantic_version,
            dv_idx: self.active_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_lines_always_use_verbatim_spacing() {
        use ui::typography::TextSpacingMode;

        let settings = ui::settings::Settings::new();
        assert_eq!(
            spacing_mode_for_line(settings.text_spacing_mode, false, "中文\tvalue".as_bytes()),
            TextSpacingMode::Verbatim
        );
    }
}
