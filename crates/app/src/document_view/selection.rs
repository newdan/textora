//! Selection operations and clipboard integration.

use core::buffer::text_buffer::CursorMovement;
use core::types::{ByteIndex, LogicalPoint};

use super::{DocumentView, normalize_paste_text};

impl DocumentView {
    /// Select the word at the given byte offset.
    /// Returns (start, end) byte offsets of the word.
    pub fn word_select_at(&self, offset: usize) -> (usize, usize) {
        use core::buffer::word_select;

        let total = self.tb.text_length();
        if total == 0 || offset >= total {
            return (offset, offset);
        }
        let range = word_select(&self.tb, offset);
        (range.start, range.end)
    }

    // ── Clipboard helpers ────────────────────────────────────────────

    /// Extract the currently selected text as bytes.
    /// Returns None if no selection.
    pub fn extract_selected_text(&self) -> Option<Vec<u8>> {
        let (start, end) = self.selection_range()?;
        if start >= end {
            return None;
        }
        let mut out = Vec::with_capacity(end - start);
        let mut off = start;
        while off < end {
            let chunk = self.tb.read_forward(off);
            if chunk.is_empty() {
                break;
            }
            let take = (end - off).min(chunk.len());
            out.extend_from_slice(&chunk[..take]);
            off += take;
        }
        Some(out)
    }

    /// Count UTF-8 characters in the current selection without allocating.
    /// Returns None if no selection or empty selection.
    pub fn count_selection_chars(&self) -> Option<usize> {
        let (start, end) = self.selection_range()?;
        if start >= end {
            return None;
        }
        let mut char_count = 0usize;
        let mut off = start;
        while off < end {
            let chunk = self.tb.read_forward(off);
            if chunk.is_empty() {
                break;
            }
            let take = (end - off).min(chunk.len());
            char_count +=
                std::str::from_utf8(&chunk[..take]).map(|s| s.chars().count()).unwrap_or(take);
            off += take;
        }
        Some(char_count)
    }

    /// Insert text at the cursor position, replacing any selection.
    /// Normalizes CRLF→LF and strips BOM before insertion.
    pub fn paste_text(&mut self, raw_text: &[u8]) {
        let normalized = normalize_paste_text(raw_text);
        if normalized.is_empty() {
            return;
        }
        // Delete selection first if present
        if self.has_selection() {
            self.delete_selection();
        }
        self.insert_at_cursor(&normalized);
    }

    /// Copy the current selection to the system clipboard.
    /// Returns true if text was copied.
    pub fn copy_selection_to_clipboard(&self) -> bool {
        let Some(text) = self.extract_selected_text() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        let s = String::from_utf8_lossy(&text);
        crate::clipboard::copy_to_clipboard(&s)
    }

    /// Cut the current selection to the system clipboard.
    /// Returns true if text was cut.
    pub fn cut_selection_to_clipboard(&mut self) -> bool {
        let Some(text) = self.extract_selected_text() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        let s = String::from_utf8_lossy(&text);
        if !crate::clipboard::copy_to_clipboard(&s) {
            return false;
        }
        self.delete_selection();
        true
    }

    /// Paste from the system clipboard at the cursor position.
    /// Returns true if text was pasted.
    pub fn paste_from_clipboard(&mut self) -> bool {
        let Some(text) = crate::clipboard::paste_from_clipboard() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        self.paste_text(text.as_bytes());
        true
    }

    // ── Selection extension (Shift+Arrow) ───────────────────────────

    /// Ensure selection_anchor is set (start selection if not already active).
    pub(crate) fn ensure_selection_active(&mut self) {
        if self.cursor.selection_anchor.is_none() {
            self.cursor.selection_anchor = Some(self.cursor.offset.to_usize());
        }
    }

    /// Extend selection left by one grapheme.
    pub fn extend_selection_left(&mut self) {
        self.ensure_selection_active();
        if self.cursor.offset.to_usize() > 0 {
            let cursor_offset = self.cursor.offset;
            self.tb.cursor_move_to_byte(cursor_offset);
            self.tb.cursor_move_delta(CursorMovement::Grapheme, -1);
            self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
        }
    }

    /// Extend selection right by one grapheme.
    pub fn extend_selection_right(&mut self) {
        self.ensure_selection_active();
        if self.cursor.offset.to_usize() < self.buffer_len() {
            let cursor_offset = self.cursor.offset;
            self.tb.cursor_move_to_byte(cursor_offset);
            self.tb.cursor_move_delta(CursorMovement::Grapheme, 1);
            self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
        }
    }

    /// Extend selection up by one line.
    pub fn extend_selection_up(&mut self) {
        self.ensure_selection_active();
        let pos = self.tb.cursor_logical_pos();
        if pos.line > 0 {
            let cursor_offset = self.cursor.offset;
            self.tb.cursor_move_to_byte(cursor_offset);
            // TODO: 后续引入 LineMap 后，应改为基于 VisualPoint 移动，以修复折行时的上下漂移问题
            self.tb
                .cursor_move_to_logical(LogicalPoint { unichar: pos.unichar, line: pos.line - 1 });
            self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
        }
    }

    /// Extend selection down by one line.
    pub fn extend_selection_down(&mut self) {
        self.ensure_selection_active();
        let pos = self.tb.cursor_logical_pos();
        let cursor_offset = self.cursor.offset;
        self.tb.cursor_move_to_byte(cursor_offset);
        // TODO: 后续引入 LineMap 后，应改为基于 VisualPoint 移动，以修复折行时的上下漂移问题
        self.tb.cursor_move_to_logical(LogicalPoint { unichar: pos.unichar, line: pos.line + 1 });
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    /// Extend selection left by one word.
    pub fn extend_selection_word_left(&mut self) {
        self.ensure_selection_active();
        let cursor_offset = self.cursor.offset;
        self.tb.cursor_move_to_byte(cursor_offset);
        self.tb.cursor_move_delta(CursorMovement::Word, -1);
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    /// Extend selection right by one word.
    pub fn extend_selection_word_right(&mut self) {
        self.ensure_selection_active();
        let cursor_offset = self.cursor.offset;
        self.tb.cursor_move_to_byte(cursor_offset);
        self.tb.cursor_move_delta(CursorMovement::Word, 1);
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    /// Extend selection to line start.
    pub fn extend_selection_to_line_start(&mut self) {
        self.ensure_selection_active();
        let line = self.cursor_line();
        if line < self.line_index.offsets.len() {
            self.set_cursor_offset_synced(self.line_index.offsets[line]);
        }
    }

    /// Extend selection to line end.
    pub fn extend_selection_to_line_end(&mut self) {
        self.ensure_selection_active();
        let line = self.cursor_line();
        if line < self.line_index.offsets.len() {
            self.set_cursor_offset_synced(
                self.line_index.offsets[line] + self.line_index.lengths[line],
            );
        }
    }

    /// Extend selection to document start.
    pub fn extend_selection_to_doc_start(&mut self) {
        self.ensure_selection_active();
        self.set_cursor_offset_synced(0);
    }

    /// Extend selection to document end.
    pub fn extend_selection_to_doc_end(&mut self) {
        self.ensure_selection_active();
        self.set_cursor_offset_synced(self.buffer_len());
    }

    /// Whether there is an active selection.
    pub fn has_selection(&self) -> bool {
        self.cursor.selection_anchor.is_some()
    }

    /// Returns (start, end) byte offsets of the selection, or None if no selection.
    /// start <= end always.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let cursor_usize = self.cursor.offset.to_usize();
        self.cursor.selection_anchor.map(|anchor| {
            if anchor <= cursor_usize { (anchor, cursor_usize) } else { (cursor_usize, anchor) }
        })
    }

    /// Clear the current selection without moving the cursor.
    pub fn clear_selection(&mut self) {
        self.cursor.selection_anchor = None;
    }

    /// Select all content (anchor at start, cursor at end).
    pub fn select_all(&mut self) {
        self.cursor.selection_anchor = Some(0);
        self.set_cursor_offset_synced(self.buffer_len());
    }

    /// Delete the selected content and clear selection.
    /// Returns true if content was deleted.
    pub fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection_range() {
            if start < end {
                // Move cursor to start, then set TextBuffer selection from start to end
                self.tb.cursor_move_to_byte(ByteIndex(start));
                self.tb.selection_update_offset(end);
                // Delete uses TextBuffer's selection range (ignores granularity/delta when selection exists)
                self.tb.extract_user_selection(true);
                self.cursor.selection_anchor = None;
                // Incremental: rescan from the line containing selection start.
                // INVARIANT: `start` is the byte offset of the selection start in the OLD text.
                // After extract_user_selection, the text has changed, but `start` still falls
                // within (or at the boundary of) the same line in the NEW text because the
                // deleted region is [start, end) — bytes before `start` are untouched.
                // Therefore `self.line_index.offsets[line]` (the line's start byte) is valid in both
                // old and new text, making it a safe rescan anchor.
                self.sync_cursor_offset_from_tb();
                self.dirty = self.tb.is_dirty();
                let rescan_from = if self.line_index.offsets.is_empty() {
                    0
                } else {
                    let line = match self.line_index.offsets.binary_search(&start) {
                        Ok(i) => i,
                        Err(i) => i.saturating_sub(1),
                    };
                    self.line_index.offsets[line]
                };
                let (tb, line_index) = (&self.model.tb, &mut self.model.line_index);
                line_index.rescan_from(tb, rescan_from);

                return true;
            } else {
                self.cursor.selection_anchor = None;
            }
        }
        false
    }
}
