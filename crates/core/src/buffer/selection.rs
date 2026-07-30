use crate::buffer::history::TextBufferSelection;
use crate::buffer::text_buffer::{CursorMovement, TextBuffer};
use crate::helpers::CoordType;
use crate::types::{ByteIndex, LogicalPoint, VisualPoint};
use std::ops::Range;

impl TextBuffer {
    /// Moves the cursor to `offset` (byte index) and updates the selection.
    ///
    /// The public API accepts `usize`; conversion to [`ByteIndex`] happens internally.
    pub fn selection_update_offset(&mut self, offset: usize) {
        self.set_cursor_for_selection(
            self.cursor_move_to_byte_internal(self.cursor, ByteIndex(offset)),
        );
    }

    /// Moves the cursor to `visual_pos` and updates the selection to contain it.
    pub fn selection_update_visual(&mut self, visual_pos: VisualPoint) {
        self.set_cursor_for_selection(self.cursor_move_to_visual_internal(self.cursor, visual_pos));
    }

    /// Moves the cursor to `logical_pos` and updates the selection to contain it.
    pub fn selection_update_logical(&mut self, logical_pos: LogicalPoint) {
        self.set_cursor_for_selection(
            self.cursor_move_to_logical_internal(self.cursor, logical_pos),
        );
    }

    /// Moves the cursor by `delta` and updates the selection to contain it.
    pub fn selection_update_delta(&mut self, granularity: CursorMovement, delta: CoordType) {
        self.set_cursor_for_selection(self.cursor_move_delta_internal(
            self.cursor,
            granularity,
            delta,
        ));
    }

    /// Select the current word.
    pub fn select_word(&mut self) {
        let Range { start, end } =
            super::navigation::word_select(&self.buffer, self.cursor.offset.to_usize());
        let beg = self.cursor_move_to_byte_internal(self.cursor, ByteIndex(start));
        let end = self.cursor_move_to_byte_internal(beg, ByteIndex(end));
        unsafe { self.set_cursor(end) };
        self.set_selection(Some(TextBufferSelection {
            beg: beg.logical_pos,
            end: end.logical_pos,
        }));
    }

    /// Select the current line.
    pub fn select_line(&mut self) {
        let beg = self.cursor_move_to_logical_internal(
            self.cursor,
            LogicalPoint { unichar: 0, line: self.cursor.logical_pos.line },
        );
        let end = self.cursor_move_to_logical_internal(
            beg,
            LogicalPoint { unichar: 0, line: self.cursor.logical_pos.line + 1 },
        );
        unsafe { self.set_cursor(end) };
        self.set_selection(Some(TextBufferSelection {
            beg: beg.logical_pos,
            end: end.logical_pos,
        }));
    }

    /// Select the entire document.
    pub fn select_all(&mut self) {
        let beg = Default::default();
        let end = self.cursor_move_to_logical_internal(beg, LogicalPoint::MAX);
        unsafe { self.set_cursor(end) };
        self.set_selection(Some(TextBufferSelection {
            beg: beg.logical_pos,
            end: end.logical_pos,
        }));
    }

    /// Starts a new selection, if there's none already.
    pub fn start_selection(&mut self) {
        if self.selection.is_none() {
            self.set_selection(Some(TextBufferSelection {
                beg: self.cursor.logical_pos,
                end: self.cursor.logical_pos,
            }));
        }
    }

    /// Destroy the current selection.
    pub fn clear_selection(&mut self) -> bool {
        let had_selection = self.selection.is_some();
        self.set_selection(None);
        had_selection
    }
}
