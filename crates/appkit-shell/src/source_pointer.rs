//! Source pointer routing for a viewport that supplies its own chrome offsets.

use std::time::Instant;

use appkit_core::document::DocumentModel;
use appkit_core::line_index::LineIndex;
use textora_core::types::UniCharOffset;
use ui::render_geom::AdvanceCacheEntry;
use ui::settings::UiMetrics;
use winit::event::ElementState;
use winit::keyboard::ModifiersState;

use crate::cursor_motion::{CursorRenderState, hit_test_row_grapheme, hit_test_visual_row};
use crate::editor_runtime::EditorRuntime;
use crate::mouse_state::MouseState;

pub fn hit_test_with_sub_line_offset(
    px: f32,
    py: f32,
    advance_cache: &[AdvanceCacheEntry],
    metrics: &UiMetrics,
    left_margin: f32,
    content_top: f32,
    sub_line_offset: f32,
    line_index: &LineIndex,
) -> Option<(UniCharOffset, usize, usize)> {
    let (vis_line, entry) =
        hit_test_visual_row(py, content_top, sub_line_offset, metrics.line_height, advance_cache)?;
    let line_grapheme = hit_test_row_grapheme(px, left_margin, entry);
    let doc_unichar = line_index.unichar_of_line(entry.doc_line) + line_grapheme;
    Some((doc_unichar, entry.doc_line, vis_line))
}

pub fn handle_cursor_moved(
    mouse: &mut MouseState,
    dv: &mut DocumentModel,
    hit: Option<(UniCharOffset, usize, usize)>,
) -> bool {
    if !mouse.captures_pointer() {
        return false;
    }
    let Some((offset, doc_line, _vis_line)) = hit else {
        return false;
    };
    if mouse.click_count < 2 {
        if let Some(anchor_byte) = mouse.down_byte_offset.take() {
            dv.cursor_mut().selection_anchor = Some(anchor_byte);
        }
        dv.set_cursor_unichar_synced_on_line(offset, doc_line);
        return true;
    }

    let byte_offset = dv.unichar_to_byte_offset(offset);
    let (selection_start, selection_end) = if mouse.click_count >= 3 {
        let range = dv.source_line_selection_range(doc_line).unwrap_or_else(|| 0..dv.buffer_len());
        (range.start, range.end)
    } else {
        dv.word_select_at(byte_offset)
    };
    if let Some(initial_anchor) = mouse.down_byte_offset.take() {
        let anchor = if byte_offset >= initial_anchor {
            initial_anchor
        } else {
            dv.cursor().offset.to_usize()
        };
        dv.cursor_mut().selection_anchor = Some(anchor);
    }
    let anchor = dv.cursor().selection_anchor.unwrap_or(selection_start);
    let cursor_offset = if byte_offset >= anchor { selection_end } else { selection_start };
    dv.set_cursor_offset_synced(cursor_offset);
    true
}

pub fn handle_mouse_input_with_cursor_state(
    state: ElementState,
    px: f32,
    py: f32,
    mouse: &mut MouseState,
    dv: &mut DocumentModel,
    cursor_render_state: &mut CursorRenderState,
    modifiers: ModifiersState,
    hit: Option<(UniCharOffset, usize, usize)>,
) -> bool {
    if state.is_pressed() {
        mouse.is_down = true;

        // A pointer press is user-driven caret movement: split any ongoing
        // undo coalescing run even if the click lands back on the byte where
        // the last edit ended.
        dv.break_edit_merge();

        mouse.record_editor_press((px, py));

        if let Some((offset, doc_line, vis_line)) = hit {
            let shift = modifiers.shift_key();

            if mouse.click_count == 3 {
                // Triple-click: select line
                let range =
                    dv.source_line_selection_range(doc_line).unwrap_or_else(|| 0..dv.buffer_len());
                dv.cursor_mut().selection_anchor = Some(range.start);
                dv.set_cursor_offset_synced(range.end);
                mouse.down_byte_offset = Some(range.start);
            } else if mouse.click_count == 2 {
                // Double-click: word select
                let byte_offset = dv.unichar_to_byte_offset(offset);
                let (start, end) = dv.word_select_at(byte_offset);
                dv.cursor_mut().selection_anchor = Some(start);
                dv.set_cursor_offset_synced(end);
                mouse.down_byte_offset = Some(start);
            } else if shift {
                // Shift+click: extend selection
                dv.ensure_selection_active();
                dv.set_cursor_unichar_synced_on_line(offset, doc_line);
                cursor_render_state.click_hint = Some((offset, vis_line));
            } else {
                // Single click: set cursor, clear any existing selection
                // Record down_byte_offset for potential drag-to-select
                dv.cursor_mut().selection_anchor = None;
                dv.cursor_move_to_unichar_on_line(offset, doc_line);
                mouse.down_byte_offset = Some(dv.cursor().offset.to_usize());
                cursor_render_state.click_hint = Some((offset, vis_line));
            }
            cursor_render_state.sticky_x = px;
            cursor_render_state.cursor_blink_instant = Instant::now();
            return true;
        } else {
            // 点击空白区域（hit 为 None，即 advance_cache 之外的空白）：
            // 取消选区并将光标移至文档末尾，同时隐藏选区高亮。
            dv.cursor_mut().selection_anchor = None;
            dv.cursor_move_to_offset(dv.buffer_len()); // byte offset OK for EOF
            mouse.down_byte_offset = None;
            cursor_render_state.sticky_x = px;
            cursor_render_state.cursor_blink_instant = Instant::now();
            return true;
        }
    } else {
        mouse.is_down = false;
        mouse.down_byte_offset = None;
        if dv.cursor().selection_anchor == Some(dv.cursor().offset.to_usize()) {
            dv.cursor_mut().selection_anchor = None;
        }
    }
    false
}

impl EditorRuntime {
    /// Applies a source pointer button after the product has resolved the hit and focus.
    pub fn handle_source_pointer_button(
        &mut self,
        state: ElementState,
        position: (f32, f32),
        mouse: &mut MouseState,
        modifiers: ModifiersState,
        hit: Option<(UniCharOffset, usize, usize)>,
        line_height: f32,
    ) -> bool {
        let Some(mut tab) = self.active_tab_id().and_then(|id| self.tab_session_mut(id)) else {
            return false;
        };
        let handled = handle_mouse_input_with_cursor_state(
            state,
            position.0,
            position.1,
            mouse,
            tab.document,
            &mut tab.runtime.presentation.cursor_render_state,
            modifiers,
            hit,
        );
        if handled {
            tab.ensure_cursor_visible(line_height);
        }
        handled
    }

    /// Updates the selection of a captured source pointer.
    pub fn move_source_pointer_selection(
        &mut self,
        mouse: &mut MouseState,
        hit: Option<(UniCharOffset, usize, usize)>,
    ) -> bool {
        let Some(tab) = self.active_tab_id().and_then(|id| self.tab_session_mut(id)) else {
            return false;
        };
        handle_cursor_moved(mouse, tab.document, hit)
    }
}
