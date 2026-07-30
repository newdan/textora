//! Edit operations — insert, delete, undo/redo, and cursor sync helpers.

use crate::line_index::LineIndex;
use core::buffer::text_buffer::CursorMovement;
use core::types::ByteIndex;

use super::{DocumentView, replace_null_bytes};

impl DocumentView {
    /// Insert bytes at the cursor position.
    /// Delegates to TextBuffer::write_raw.
    pub fn insert_at_cursor(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let sanitized = replace_null_bytes(data);
        let has_newline = sanitized.contains(&b'\n') || sanitized.contains(&b'\r');
        let old_len = self.tb.text_length();
        let old_line_count = self.line_index.offsets.len();
        self.tb.write_raw(&sanitized);
        self.sync_after_edit_incremental(old_len, old_line_count, has_newline);
    }

    /// Delete one grapheme before the cursor (backspace).
    /// Delegates to TextBuffer::delete(Grapheme, -1).
    pub fn delete_backward(&mut self, count: usize) {
        if count == 0 || self.cursor.offset == ByteIndex::ZERO {
            return;
        }
        let old_len = self.tb.text_length();
        let old_line_count = self.line_index.offsets.len();
        self.tb.delete(CursorMovement::Grapheme, -(count as isize));
        self.sync_after_edit_incremental(old_len, old_line_count, true);
    }

    /// Delete one grapheme after the cursor (delete forward).
    /// Delegates to TextBuffer::delete(Grapheme, 1).
    pub fn delete_forward(&mut self, count: usize) {
        if count == 0 || self.cursor.offset.to_usize() >= self.tb.text_length() {
            return;
        }
        let old_len = self.tb.text_length();
        let old_line_count = self.line_index.offsets.len();
        self.tb.delete(CursorMovement::Grapheme, count as isize);
        self.sync_after_edit_incremental(old_len, old_line_count, true);
    }

    /// Undo the last edit operation.
    pub fn undo(&mut self) {
        self.tb.undo();
        self.sync_after_edit_incremental_undo_redo();
    }

    /// Redo the last undone operation.
    pub fn redo(&mut self) {
        self.tb.redo();
        self.sync_after_edit_incremental_undo_redo();
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Incremental sync for undo/redo: rescan from line 0 reusing Vec capacity,
    /// and update viewport in-place instead of recreating it.
    fn sync_after_edit_incremental_undo_redo(&mut self) {
        self.sync_cursor_offset_from_tb();
        self.mark_content_changed();
        self.dirty = self.tb.is_dirty();
        // Assign stable snapshot ID when tab becomes dirty (for hot exit).
        if self.dirty && self.dirty_snapshot_id.is_none() {
            self.dirty_snapshot_id = Some(if self.file_path.is_some() {
                crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::path_id(
                    self.file_path.as_ref().unwrap(),
                ))
            } else {
                crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id())
            });
        }
        let (tb, line_index) = (&self.model.tb, &mut self.model.line_index);
        line_index.rescan_from(tb, 0);
        self.invalidate_highlights_from(0);
    }

    /// Incremental line index update after an edit.
    ///
    /// When `has_newline` is false, only shifts offsets of lines after the edit
    /// position — O(1) for single-char inserts. Falls back to full rebuild if
    /// newlines were involved or line count changed.
    fn sync_after_edit_incremental(
        &mut self,
        old_len: usize,
        old_line_count: usize,
        may_have_newline: bool,
    ) {
        self.sync_cursor_offset_from_tb();
        self.mark_content_changed();
        self.dirty = self.tb.is_dirty();
        // Assign stable snapshot ID when tab becomes dirty (for hot exit).
        if self.dirty && self.dirty_snapshot_id.is_none() {
            self.dirty_snapshot_id = Some(if self.file_path.is_some() {
                crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::path_id(
                    self.file_path.as_ref().unwrap(),
                ))
            } else {
                crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id())
            });
        }

        let new_len = self.tb.text_length();
        let delta: isize = new_len as isize - old_len as isize;

        let new_line_count = self.line_index.offsets.len();

        // Fast path: no newlines involved, line count unchanged, and line index is non-empty.
        // This covers single-char inserts/deletes of non-newline characters.
        if !may_have_newline
            && new_line_count == old_line_count
            && delta != 0
            && !self.line_index.offsets.is_empty()
        {
            // edit_pos: for inserts, cursor is at end of inserted text (edit_pos + delta).
            //   edit_pos = cursor - delta
            // For deletes, cursor is at start of deleted range (edit_pos).
            //   edit_pos = cursor
            let cursor_usize = self.cursor.offset.to_usize();
            let edit_pos =
                if delta > 0 { (cursor_usize as isize - delta) as usize } else { cursor_usize };
            // Shift all line offsets after the edit position by delta
            let start = self.line_index.offsets.partition_point(|&off| off <= edit_pos);
            for off in &mut self.line_index.offsets[start..] {
                *off = (*off as isize + delta) as usize;
            }
            // Update current line length
            let line = self.cursor_line();
            if line < self.line_index.lengths.len() {
                self.line_index.lengths[line] =
                    (self.line_index.lengths[line] as isize + delta) as usize;
            }
            self.invalidate_highlights_from(line as isize);
            return;
        }

        // Slow path: newlines were added/removed, or line count changed.
        // Rescan from the edit position forward.
        if may_have_newline && old_line_count != 0 {
            let cursor_usize = self.cursor.offset.to_usize();
            let edit_pos =
                if delta > 0 { (cursor_usize as isize - delta) as usize } else { cursor_usize };
            // Find the line containing edit_pos using binary search
            let edit_line = match self.line_index.offsets.binary_search(&edit_pos) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            // Rescan from the start of that line
            let rescan_from = self.line_index.offsets[edit_line];
            let (tb, line_index) = (&self.model.tb, &mut self.model.line_index);
            line_index.rescan_from(tb, rescan_from);
            self.invalidate_highlights_from(edit_line as isize);
        } else {
            // Fallback: full rebuild
            self.line_index = LineIndex::rebuild_from(&self.tb);
            self.invalidate_highlights_from(0);
            let _total = self.line_index.line_count().max(1);
        }
    }

    /// Sync cursor_offset from TextBuffer (no edit, just cursor move).
    /// Clears selection on cursor movement.
    pub(crate) fn sync_cursor(&mut self) {
        self.cursor.selection_anchor = None;
        self.sync_cursor_offset_from_tb();
        self.cursor.cached_line = None;
    }

    /// Set cursor_offset to a caller-computed value and sync TextBuffer's
    /// internal cursor to match. Does NOT clear selection — callers that
    /// need to clear selection should do so before calling this.
    /// Invalidates cached_cursor_line.
    pub(crate) fn set_cursor_offset_synced(&mut self, offset: usize) {
        self.tb.cursor_move_to_byte(ByteIndex(offset));
        // 回读实际位置：cursor_move_to_byte 按 grapheme 边界 snap，
        // 当 offset 落在多字节 UTF-8 字符内部时最终位置会与请求不同。
        self.cursor.offset = self.tb.cursor_offset();
        self.cursor.cached_line = None;
    }

    /// Read cursor_offset from TextBuffer and update cache.
    /// Use after tb operations that leave the cursor in the correct position
    /// (e.g. delete, undo/redo, sync_after_edit_*).
    pub(crate) fn sync_cursor_offset_from_tb(&mut self) {
        self.cursor.offset = self.tb.cursor_offset();
        self.cursor.cached_line = None;
    }

    /// Debug assertion: cursor_offset must match tb.cursor_offset().
    /// Call at entry of read paths to catch desync early.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub(crate) fn assert_cursor_synced(&self) {
        debug_assert_eq!(
            self.tb.cursor_offset(),
            self.cursor.offset,
            "cursor_offset desynced from tb: tb={}, dv={}",
            self.tb.cursor_offset().to_usize(),
            self.cursor.offset.to_usize()
        );
    }
}
