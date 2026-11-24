//! Source caret placement that retains both the logical line and the visual row.

use std::ops::Range;
use std::time::Instant;

use appkit_core::document::DocumentModel;
use appkit_core::line_index::LineIndex;
use appkit_core::tab_set::types::TabId;
use textora_core::types::GraphemeOffset;
use ui::core::widget::PointerClickKind;
use ui::render_geom::AdvanceCacheEntry;

use super::EditorRuntime;
use super::editor_pointer::SelectionMotion;
use crate::cursor_motion::{CursorRenderState, hit_test_row_grapheme, hit_test_visual_row};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SourcePointerPolicy {
    #[default]
    GroupedBytes,
    VisualRows,
}

#[derive(Clone, Copy)]
struct SourcePointerHit {
    offset: GraphemeOffset,
    document_line: usize,
    visual_row: usize,
}

impl EditorRuntime {
    pub fn configure_source_pointer(&mut self, policy: SourcePointerPolicy) {
        self.source_pointer_policy = policy;
    }

    pub(super) fn place_source_row_selection(
        &mut self,
        tab_id: TabId,
        editor_rect: ui::Rect,
        position: (f32, f32),
        motion: SelectionMotion,
        click_kind: PointerClickKind,
    ) -> bool {
        let metrics = ui::UiMetrics::from_settings(&self.settings, self.scale_factor() as f32);
        let Some(tab) = self.tab_session(tab_id) else {
            return false;
        };
        let text_left = self.source_input_left().unwrap_or_else(|| {
            let gutter = self.settings.gutter_width(tab.document.line_count()) * metrics.dpi;
            editor_rect.x + metrics.content_left_margin.max(gutter)
        });
        let hit = hit_test_source_row(
            position,
            tab.advance_cache(),
            metrics.line_height,
            text_left,
            editor_rect.y,
            tab.sub_line_pixel_offset(metrics.line_height),
            &tab.document.line_index,
        );

        match motion {
            SelectionMotion::Press { extend } => {
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                let pending_anchor = tab.with_presentation(|document, presentation| {
                    press_source_row(
                        document,
                        &mut presentation.cursor_render_state,
                        position.0,
                        hit,
                        click_kind,
                        extend,
                    )
                });
                tab.ensure_cursor_visible(metrics.line_height);
                self.input_session.set_source_row_anchor(pending_anchor);
            }
            SelectionMotion::Drag => {
                let Some(hit) = hit else {
                    return false;
                };
                let pending_anchor = self.input_session.take_source_row_anchor();
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                drag_source_row(tab.document_mut(), hit, click_kind, pending_anchor);
            }
        }
        true
    }
}

fn hit_test_source_row(
    position: (f32, f32),
    advance_cache: &[AdvanceCacheEntry],
    line_height: f32,
    text_left: f32,
    text_top: f32,
    sub_line_offset: f32,
    line_index: &LineIndex,
) -> Option<SourcePointerHit> {
    let (visual_row, entry) =
        hit_test_visual_row(position.1, text_top, sub_line_offset, line_height, advance_cache)?;
    let grapheme_on_line = hit_test_row_grapheme(position.0, text_left, entry);
    Some(SourcePointerHit {
        offset: line_index.grapheme_offset_of_line(entry.doc_line) + grapheme_on_line,
        document_line: entry.doc_line,
        visual_row,
    })
}

fn press_source_row(
    document: &mut DocumentModel,
    caret: &mut CursorRenderState,
    pointer_x: f32,
    hit: Option<SourcePointerHit>,
    click_kind: PointerClickKind,
    extend: bool,
) -> Option<usize> {
    caret.sticky_x = pointer_x;
    caret.cursor_blink_instant = Instant::now();

    let Some(hit) = hit else {
        document.cursor_mut().selection_anchor = None;
        document.cursor_move_to_offset(document.buffer_len());
        return None;
    };
    if let Some(range) = source_selection_range(document, hit, click_kind) {
        document.cursor_mut().selection_anchor = Some(range.start);
        document.set_cursor_offset_synced(range.end);
        return Some(range.start);
    }

    caret.click_hint = Some((hit.offset, hit.visual_row));
    if extend {
        document.ensure_selection_active();
        document.set_cursor_grapheme_offset_synced_on_line(hit.offset, hit.document_line);
        return None;
    }
    document.cursor_mut().selection_anchor = None;
    document.cursor_move_to_grapheme_offset_on_line(hit.offset, hit.document_line);
    Some(document.cursor_offset().to_usize())
}

fn drag_source_row(
    document: &mut DocumentModel,
    hit: SourcePointerHit,
    click_kind: PointerClickKind,
    pending_anchor: Option<usize>,
) {
    let Some(range) = source_selection_range(document, hit, click_kind) else {
        if let Some(anchor) = pending_anchor {
            document.cursor_mut().selection_anchor = Some(anchor);
        }
        document.set_cursor_grapheme_offset_synced_on_line(hit.offset, hit.document_line);
        return;
    };

    let hit_byte = document.grapheme_offset_to_byte(hit.offset);
    if let Some(initial_anchor) = pending_anchor {
        let anchor = if hit_byte >= initial_anchor {
            initial_anchor
        } else {
            document.cursor_offset().to_usize()
        };
        document.cursor_mut().selection_anchor = Some(anchor);
    }
    let anchor = document.cursor().selection_anchor.unwrap_or(range.start);
    let cursor_byte = if hit_byte >= anchor { range.end } else { range.start };
    document.set_cursor_offset_synced(cursor_byte);
}

fn source_selection_range(
    document: &DocumentModel,
    hit: SourcePointerHit,
    click_kind: PointerClickKind,
) -> Option<Range<usize>> {
    match click_kind {
        PointerClickKind::Single => None,
        PointerClickKind::Double => {
            let (start, end) =
                document.word_select_at(document.grapheme_offset_to_byte(hit.offset));
            Some(start..end)
        }
        PointerClickKind::Triple => Some(
            document
                .source_line_selection_range(hit.document_line)
                .unwrap_or_else(|| 0..document.buffer_len()),
        ),
    }
}

#[cfg(test)]
mod tests;
