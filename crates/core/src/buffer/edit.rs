use crate::buffer::history::{
    ActiveEditGroupInfo, EditHistoryKind, EditMergeAnchor, HistoryEntry, HistoryType,
    TextBufferSelection,
};
use crate::buffer::text_buffer::{CursorMovement, MoveLineDirection, TextBuffer};
use crate::cell::SemiRefCell;
use crate::helpers::CoordType;
use crate::simd::{self, memchr2};
use crate::types::{ByteIndex, LogicalPoint};
use crate::unicode::{self, Cursor};
use std::mem;
use stdext::arena::scratch_arena;
use stdext::collections::BString;
use stdext::{ReplaceRange, minmax, slice_copy_safe};

pub(crate) const MARGIN_TEMPLATE: &str = "                                                                                                    ";
pub(crate) const TAB_WHITESPACE: &str = MARGIN_TEMPLATE;

pub(crate) struct ActiveEditLineInfo {
    pub(crate) safe_start: Cursor,
    pub(crate) line_height_in_rows: CoordType,
    pub(crate) distance_next_line_start: usize,
}
impl TextBuffer {
    /// Replaces tabs with whitespace if needed, etc.
    pub fn write_canon(&mut self, text: &[u8]) {
        self.write(text, self.cursor, false);
    }

    /// Inserts `text` as-is at the current cursor position.
    /// The only transformation applied is that newlines are normalized.
    pub fn write_raw(&mut self, text: &[u8]) {
        self.write(text, self.cursor, true);
    }

    fn write(&mut self, text: &[u8], at: Cursor, raw: bool) {
        let history_type = if raw { HistoryType::Other } else { HistoryType::Write };
        let mut edit_begun = false;

        // If we have an active selection, writing an empty `text`
        // will still delete the selection. As such, we check this first.
        if let Some((beg, end)) = self.selection_range_internal(false) {
            self.edit_begin(history_type, beg);
            self.edit_delete(end);
            self.set_selection(None);
            edit_begun = true;
        }

        // If the text is empty the remaining code won't do anything,
        // allowing us to exit early.
        if text.is_empty() {
            // ...we still need to end any active edit session though.
            if edit_begun {
                self.edit_end();
            }
            return;
        }

        if !edit_begun {
            self.edit_begin(history_type, at);
        }

        let mut offset = 0;
        let scratch = scratch_arena(None);
        let mut newline_buffer = BString::empty();

        loop {
            // Can't use `unicode::newlines_forward` because bracketed paste uses CR instead of LF/CRLF.
            let offset_next = memchr2(b'\r', b'\n', text, offset);
            let line = &text[offset..offset_next];
            let column_before = self.cursor.logical_pos.unichar;

            // Write the contents of the line into the buffer.
            let mut line_off = 0;
            while line_off < line.len() {
                // Split the line into chunks of non-tabs and tabs.
                let mut plain = line;
                if !raw && !self.indent_with_tabs {
                    let end = memchr2(b'\t', b'\t', line, line_off);
                    plain = &line[line_off..end];
                }

                // Non-tabs are written as-is, because the outer loop already handles newline translation.
                self.edit_write(plain);
                line_off += plain.len();

                // Now replace tabs with spaces.
                while line_off < line.len() && line[line_off] == b'\t' {
                    let spaces = self.tab_size_eval(self.cursor.column);
                    let spaces = &TAB_WHITESPACE.as_bytes()[..spaces as usize];
                    self.edit_write(spaces);
                    line_off += 1;
                }
            }

            if !raw && self.overtype {
                let delete = self.cursor.logical_pos.unichar - column_before;
                let end = self.cursor_move_to_logical_internal(
                    self.cursor,
                    LogicalPoint {
                        unichar: self.cursor.logical_pos.unichar + delete,
                        line: self.cursor.logical_pos.line,
                    },
                );
                self.edit_delete(end);
            }

            offset += line.len();
            if offset >= text.len() {
                break;
            }

            // First, write the newline.
            newline_buffer.clear();
            newline_buffer.push_str(&*scratch, if self.newlines_are_crlf { "\r\n" } else { "\n" });

            if !raw {
                // We'll give the next line the same indentation as the previous one.
                // This block figures out how much that is. We can't reuse that value,
                // because "  a\n  a\n" should give the 3rd line a total indentation of 4.
                // Assuming your terminal has bracketed paste, this won't be a concern though.
                // (If it doesn't, use a different terminal.)
                let line_beg = self.goto_line_start(self.cursor, self.cursor.logical_pos.line);
                let limit = self.cursor.offset.to_usize();
                let mut off = line_beg.offset.to_usize();
                let mut newline_indentation = 0;

                'outer: while off < limit {
                    let chunk = self.read_forward(off);
                    let chunk = &chunk[..chunk.len().min(limit - off)];

                    for &c in chunk {
                        if c == b' ' {
                            newline_indentation += 1;
                        } else if c == b'\t' {
                            newline_indentation += self.tab_size_eval(newline_indentation);
                        } else {
                            break 'outer;
                        }
                    }

                    off += chunk.len();
                }

                // If tabs are enabled, add as many tabs as we can.
                if self.indent_with_tabs {
                    let tab_count = newline_indentation / self.tab_size;
                    newline_buffer.push_repeat(&*scratch, '\t', tab_count as usize);
                    newline_indentation -= tab_count * self.tab_size;
                }

                // If tabs are disabled, or if the indentation wasn't a multiple of the tab size,
                // add spaces to make up the difference.
                newline_buffer.push_repeat(&*scratch, ' ', newline_indentation as usize);
            }

            self.edit_write(newline_buffer.as_bytes());

            // Skip one CR/LF/CRLF.
            if offset >= text.len() {
                break;
            }
            if text[offset] == b'\r' {
                offset += 1;
            }
            if offset >= text.len() {
                break;
            }
            if text[offset] == b'\n' {
                offset += 1;
            }
            if offset >= text.len() {
                break;
            }
        }

        // POSIX mandates that all valid lines end in a newline.
        // This isn't all that common on Windows and so we have
        // `self.final_newline` to control this.
        //
        // In order to not annoy people with this, we only add a
        // newline if you just edited the very end of the buffer.
        if self.insert_final_newline
            && self.cursor.offset.to_usize() > 0
            && self.cursor.offset.to_usize() == self.text_length()
            && self.cursor.logical_pos.unichar > 0
        {
            let cursor = self.cursor;
            self.edit_write(if self.newlines_are_crlf { b"\r\n" } else { b"\n" });
            // Can't use `set_cursor_internal` here, because we haven't updated the line stats yet.
            self.cursor = cursor;
        }

        self.edit_end();
    }

    /// Deletes 1 grapheme cluster from the buffer.
    /// `cursor_movements` is expected to be -1 for backspace and 1 for delete.
    /// If there's a current selection, it will be deleted and `cursor_movements` ignored.
    /// The selection is cleared after the call.
    /// Deletes characters from the buffer based on a delta from the cursor.
    pub fn delete(&mut self, granularity: CursorMovement, delta: CoordType) {
        if delta == 0 {
            return;
        }

        let mut beg;
        let mut end;

        if let Some(r) = self.selection_range_internal(false) {
            (beg, end) = r;
        } else {
            if (delta < 0 && self.cursor.offset == ByteIndex::ZERO)
                || (delta > 0 && self.cursor.offset.to_usize() >= self.text_length())
            {
                // Nothing to delete.
                return;
            }

            beg = self.cursor;
            end = self.cursor_move_delta_internal(beg, granularity, delta);
            if beg.offset == end.offset {
                return;
            }
            if beg.offset > end.offset {
                mem::swap(&mut beg, &mut end);
            }
        }

        self.edit_begin(HistoryType::Delete, beg);
        self.edit_delete(end);
        self.edit_end();

        self.set_selection(None);
    }

    /// Returns the logical position of the first character on this line.
    /// Return `.unichar == 0` if there are no non-whitespace characters.
    pub fn indent_end_logical_pos(&self) -> LogicalPoint {
        let cursor = self.goto_line_start(self.cursor, self.cursor.logical_pos.line);
        let (chars, _) = self.measure_indent_internal(cursor.offset.to_usize(), CoordType::MAX);
        LogicalPoint { unichar: chars.max(0) as usize, line: cursor.logical_pos.line }
    }

    /// Indents/unindents the current selection or line.
    pub fn indent_change(&mut self, direction: CoordType) {
        let selection = self.selection;
        let mut selection_beg = self.cursor.logical_pos;
        let mut selection_end = selection_beg;

        if let Some(TextBufferSelection { beg, end }) = &selection {
            selection_beg = *beg;
            selection_end = *end;
        }

        if direction >= 0 && self.selection.is_none_or(|sel| sel.beg.line == sel.end.line) {
            self.write_canon(b"\t");
            return;
        }

        self.edit_begin_grouping();

        for y in
            selection_beg.line.min(selection_end.line)..=selection_beg.line.max(selection_end.line)
        {
            self.cursor_move_to_logical(LogicalPoint { unichar: 0, line: y });

            let line_start_offset = self.cursor.offset.to_usize();
            let (curr_chars, curr_columns) =
                self.measure_indent_internal(line_start_offset, CoordType::MAX);

            self.cursor_move_to_logical(LogicalPoint {
                unichar: curr_chars.max(0) as usize,
                line: self.cursor.logical_pos.line,
            });

            let delta;

            if direction < 0 {
                // Unindent the line. If there's no indentation, skip.
                if curr_columns <= 0 {
                    continue;
                }

                let (prev_chars, _) = self.measure_indent_internal(
                    line_start_offset,
                    self.tab_size_prev_column(curr_columns),
                );

                delta = prev_chars - curr_chars;
                self.delete(CursorMovement::Grapheme, delta);
            } else {
                // Indent the line. `self.cursor` is already at the level of indentation.
                delta = self.tab_size_eval(curr_columns);
                self.write_canon(b"\t");
            }

            // As the lines get unindented, the selection should shift with them.
            if y == selection_beg.line {
                selection_beg.unichar = (selection_beg.unichar as isize + delta) as usize;
            }
            if y == selection_end.line {
                selection_end.unichar = (selection_end.unichar as isize + delta) as usize;
            }
        }
        self.edit_end_grouping();

        // Move the cursor to the new end of the selection.
        self.set_cursor_internal(self.cursor_move_to_logical_internal(self.cursor, selection_end));

        // NOTE: If the selection was previously `None`,
        // it should continue to be `None` after this.
        self.set_selection(
            selection.map(|_| TextBufferSelection { beg: selection_beg, end: selection_end }),
        );
    }

    fn measure_indent_internal(
        &self,
        mut offset: usize,
        max_columns: CoordType,
    ) -> (CoordType, CoordType) {
        let mut chars = 0;
        let mut columns = 0;

        'outer: loop {
            let chunk = self.read_forward(offset);
            if chunk.is_empty() {
                break;
            }

            for &c in chunk {
                let next = match c {
                    b' ' => columns + 1,
                    b'\t' => columns + self.tab_size_eval(columns),
                    _ => break 'outer,
                };
                if next > max_columns {
                    break 'outer;
                }
                chars += 1;
                columns = next;
            }

            offset += chunk.len();

            // No need to do another round if we
            // already got the exact right amount.
            if columns >= max_columns {
                break;
            }
        }

        (chars, columns)
    }

    /// Displaces the current, cursor or the selection, line(s) in the given direction.
    pub fn move_selected_lines(&mut self, direction: MoveLineDirection) {
        let selection = self.selection;
        let cursor = self.cursor;

        // If there's no selection, we move the line the cursor is on instead.
        let [beg, end] = match self.selection {
            Some(s) => minmax(s.beg.line, s.end.line),
            None => [cursor.logical_pos.line, cursor.logical_pos.line],
        };

        // Check if this would be a no-op.
        if match direction {
            MoveLineDirection::Up => beg == 0,
            MoveLineDirection::Down => end >= self.stats.logical_lines as usize - 1,
        } {
            return;
        }

        let delta = match direction {
            MoveLineDirection::Up => -1isize,
            MoveLineDirection::Down => 1isize,
        };
        let (cut, paste) = match direction {
            MoveLineDirection::Up => (beg - 1, end),
            MoveLineDirection::Down => (end + 1, beg),
        };

        self.edit_begin_grouping();
        {
            // Let's say this is `MoveLineDirection::Up`.
            // In that case, we'll cut (remove) the line above the selection here...
            self.cursor_move_to_logical(LogicalPoint { unichar: 0, line: cut });
            let line = self.extract_selection(true);

            // ...and paste it below the selection. This will then
            // appear to the user as if the selection was moved up.
            self.cursor_move_to_logical(LogicalPoint { unichar: 0, line: paste });
            self.edit_begin(HistoryType::Write, self.cursor);
            // The `extract_selection` call can return an empty `Vec`),
            // if the `cut` line was at the end of the file. Since we want to
            // paste the line somewhere it needs a trailing newline at the minimum.
            //
            // Similarly, if the `paste` line is at the end of the file
            // and there's no trailing newline, we'll have failed to reach
            // that end in which case `logical_pos.line != paste`.
            if line.is_empty() || self.cursor.logical_pos.line != paste {
                self.write_canon(b"\n");
            }
            if !line.is_empty() {
                self.write_raw(&line);
            }
            self.edit_end();
        }
        self.edit_end_grouping();

        // Shift the cursor and selection together with the moved lines.
        self.cursor_move_to_logical(LogicalPoint {
            unichar: cursor.logical_pos.unichar,
            line: (cursor.logical_pos.line as isize + delta) as usize,
        });
        self.set_selection(selection.map(|mut s| {
            s.beg.line = (s.beg.line as isize + delta) as usize;
            s.end.line = (s.end.line as isize + delta) as usize;
            s
        }));
    }

    /// Extracts the contents of the current selection.
    /// May optionally delete it, if requested. This is meant to be used for Ctrl+X.
    fn extract_selection(&mut self, delete: bool) -> Vec<u8> {
        let line_copy = !self.has_selection();
        let Some((beg, end)) = self.selection_range_internal(true) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        self.buffer.extract_raw(beg.offset.to_usize()..end.offset.to_usize(), &mut out, 0);

        if delete && !out.is_empty() {
            self.edit_begin(HistoryType::Delete, beg);
            self.edit_delete(end);
            self.edit_end();
            self.set_selection(None);
        }

        // Line copies (= Ctrl+C when there's no selection) always end with a newline.
        if line_copy && !out.ends_with(b"\n") {
            out.replace_range(out.len().., if self.newlines_are_crlf { b"\r\n" } else { b"\n" });
        }

        out
    }

    /// Extracts the contents of the current selection the user made.
    /// This differs from `TextBuffer::extract_selection()` in that
    /// it does nothing if the selection was made by searching.
    pub fn extract_user_selection(&mut self, delete: bool) -> Option<Vec<u8>> {
        if !self.has_selection() {
            return None;
        }

        if !delete && let Some(search) = &self.search {
            let search = unsafe { &*search.get() };
            if search.selection_generation == self.selection_generation {
                return None;
            }
        }

        Some(self.extract_selection(delete))
    }

    /// Returns the current selection anchors, or `None` if there
    /// is no selection. The returned logical positions are sorted.
    pub fn selection_range(&self) -> Option<(Cursor, Cursor)> {
        self.selection_range_internal(false)
    }

    /// Returns the current selection anchors.
    ///
    /// If there's no selection and `line_fallback` is `true`,
    /// the start/end of the current line are returned.
    /// This is meant to be used for Ctrl+C / Ctrl+X.
    fn selection_range_internal(&self, line_fallback: bool) -> Option<(Cursor, Cursor)> {
        let [beg, end] = match self.selection {
            None if !line_fallback => return None,
            None => [
                LogicalPoint { unichar: 0, line: self.cursor.logical_pos.line },
                LogicalPoint { unichar: 0, line: self.cursor.logical_pos.line + 1 },
            ],
            Some(TextBufferSelection { beg, end }) => minmax(beg, end),
        };

        let beg = self.cursor_move_to_logical_internal(self.cursor, beg);
        let end = self.cursor_move_to_logical_internal(beg, end);

        if beg.offset < end.offset { Some((beg, end)) } else { None }
    }

    pub fn edit_begin_grouping(&mut self) {
        self.active_edit_group = Some(ActiveEditGroupInfo {
            cursor_before: self.cursor.logical_pos,
            selection_before: self.selection,
            stats_before: self.stats,
            generation_before: self.buffer.generation(),
        });
    }

    pub fn edit_end_grouping(&mut self) {
        self.active_edit_group = None;
    }

    /// Starts a new edit operation.
    /// This is used for tracking the undo/redo history.
    fn edit_begin(&mut self, history_type: HistoryType, cursor: Cursor) {
        self.active_edit_depth += 1;
        if self.active_edit_depth > 1 {
            return;
        }

        // Any edit outside `replace_range_with_history` invalidates the coalescing anchor.
        // (`replace_range_with_history` re-establishes it after `edit_end`.)
        self.edit_merge_anchor = None;

        let cursor_before = self.cursor;
        self.set_cursor_internal(cursor);

        // If both the last and this are a Write/Delete operation, we skip allocating a new undo history item.
        if history_type != self.last_history_type
            || !matches!(history_type, HistoryType::Write | HistoryType::Delete)
        {
            self.redo_stack.clear();
            while self.undo_stack.len() > 1000 {
                self.undo_stack.pop_front();
            }

            self.last_history_type = history_type;
            self.undo_stack.push_back(SemiRefCell::new(HistoryEntry {
                cursor_before: cursor_before.logical_pos,
                selection_before: self.selection,
                stats_before: self.stats,
                generation_before: self.buffer.generation(),
                cursor: cursor.logical_pos,
                deleted: Vec::new(),
                added: Vec::new(),
            }));

            if let Some(info) = &self.active_edit_group
                && let Some(entry) = self.undo_stack.back()
            {
                let mut entry = entry.borrow_mut();
                entry.cursor_before = info.cursor_before;
                entry.selection_before = info.selection_before;
                entry.stats_before = info.stats_before;
                entry.generation_before = info.generation_before;
            }
        }

        self.active_edit_off = cursor.offset.to_usize();
        self.highlighter_cache.invalidate_from(cursor.logical_pos.line as CoordType);

        // If word-wrap is enabled, the visual layout of all logical lines affected by the write
        // may have changed. This includes even text before the insertion point up to the line
        // start, because this write may have joined with a word before the initial cursor.
        // See other uses of `word_wrap_cursor_next_line` in this function.
    }

    /// Writes `text` into the buffer at the current cursor position.
    /// It records the change in the undo stack.
    fn edit_write(&mut self, text: &[u8]) {
        let logical_y_before = self.cursor.logical_pos.line;

        // Copy the written portion into the undo entry.
        {
            let mut undo = self
                .undo_stack
                .back_mut()
                .expect("an active edit always has an undo entry")
                .borrow_mut();
            undo.added.extend_from_slice(text);
        }

        // Write!
        self.buffer.replace(self.active_edit_off..self.active_edit_off, text);

        // Move self.cursor to the end of the newly written text. Can't use `self.set_cursor_internal`,
        // because we're still in the progress of recalculating the line stats.
        self.active_edit_off += text.len();
        self.cursor =
            self.cursor_move_to_byte_internal(self.cursor, ByteIndex(self.active_edit_off));
        self.stats.logical_lines += (self.cursor.logical_pos.line - logical_y_before) as CoordType;
    }

    /// Deletes the text between the current cursor position and `to`.
    /// It records the change in the undo stack.
    fn edit_delete(&mut self, to: Cursor) {
        debug_assert!(to.offset.to_usize() >= self.active_edit_off);

        let logical_y_before = self.cursor.logical_pos.line;
        let off = self.active_edit_off;
        let mut out_off = usize::MAX;

        let mut undo = self.undo_stack.back_mut().unwrap().borrow_mut();

        // If this is a continued backspace operation,
        // we need to prepend the deleted portion to the undo entry.
        if self.cursor.logical_pos < undo.cursor {
            out_off = 0;
            undo.cursor = self.cursor.logical_pos;
        }

        // Copy the deleted portion into the undo entry.
        let deleted = &mut undo.deleted;
        self.buffer.extract_raw(off..to.offset.to_usize(), deleted, out_off);

        // Delete the portion from the buffer by enlarging the gap.
        let count = to.offset.to_usize() - off;
        self.buffer.allocate_gap(off, 0, count);

        self.stats.logical_lines +=
            logical_y_before as CoordType - to.logical_pos.line as CoordType;
    }

    fn edit_replace(&mut self, start: Cursor, end: Cursor, replacement: &[u8]) {
        let start_offset = start.offset.to_usize();
        let end_offset = end.offset.to_usize();

        {
            let mut undo = self
                .undo_stack
                .back_mut()
                .expect("an active edit always has an undo entry")
                .borrow_mut();
            // If this edit joins a coalesced entry at an earlier position
            // (continued backspace), prepend the deleted portion, mirroring `edit_delete`.
            let out_off = if start.logical_pos < undo.cursor {
                undo.cursor = start.logical_pos;
                0
            } else {
                usize::MAX
            };
            self.buffer.extract_raw(start_offset..end_offset, &mut undo.deleted, out_off);
            undo.added.extend_from_slice(replacement);
        }

        self.buffer.replace(start_offset..end_offset, replacement);
        self.cursor = self
            .cursor_move_to_byte_internal(self.cursor, ByteIndex(start_offset + replacement.len()));
        self.stats.logical_lines +=
            self.cursor.logical_pos.line as CoordType - end.logical_pos.line as CoordType;
    }

    /// Finalizes the current edit operation
    /// and recalculates the line statistics.
    fn edit_end(&mut self) {
        self.active_edit_depth -= 1;
        debug_assert!(self.active_edit_depth >= 0);
        if self.active_edit_depth > 0 {
            return;
        }

        #[cfg(debug_assertions)]
        {
            let entry = self.undo_stack.back_mut().unwrap().borrow_mut();
            debug_assert!(!entry.deleted.is_empty() || !entry.added.is_empty());
        }

        if let Some(info) = self.active_edit_line_info.take() {
            let deleted_count = self.undo_stack.back_mut().unwrap().borrow_mut().deleted.len();
            let target = self.cursor.logical_pos;

            // From our safe position we can measure the actual visual position of the cursor.
            self.set_cursor_internal(self.cursor_move_to_logical_internal(info.safe_start, target));

            // If content is added at the insertion position, that's not a problem:
            // We can just remeasure the height of this one line and calculate the delta.
            // `deleted_count` is 0 in this case.
            //
            // The problem is when content is deleted, because it may affect lines
            // beyond the end of the `next_line`. In that case we have to measure
            // the entire buffer contents until the end to compute `self.stats.visual_lines`.
            if deleted_count < info.distance_next_line_start {
                // Now we can measure how many more visual rows this logical line spans.
                let next_line = self.cursor_move_to_logical_internal(
                    self.cursor,
                    LogicalPoint { unichar: 0, line: target.line + 1 },
                );
                let lines_before = info.line_height_in_rows;
                let lines_after = next_line.visual_pos.row as CoordType
                    - info.safe_start.visual_pos.row as CoordType;
                self.stats.visual_lines += lines_after - lines_before;
            } else {
                let end = self.cursor_move_to_logical_internal(self.cursor, LogicalPoint::MAX);
                self.stats.visual_lines = end.visual_pos.row as CoordType + 1;
            }
        } else {
            // If word-wrap is disabled the visual line count always matches the logical one.
            self.stats.visual_lines = self.stats.logical_lines;
        }

        self.recalc_after_content_changed();
    }

    /// Undo the last edit operation.
    pub fn undo(&mut self) {
        self.undo_redo(true);
    }

    /// Redo the last undo operation.
    pub fn redo(&mut self) {
        self.undo_redo(false);
    }

    /// Breaks any ongoing undo coalescing run, so the next `Insert`/`Delete`
    /// edit starts a fresh undo entry even when it is byte-adjacent to the
    /// last one.
    ///
    /// The buffer itself cannot tell a post-transaction caret sync apart from
    /// user-driven caret movement (both arrive as `cursor_move_to_byte`), so
    /// callers that know a caret move came from the user (keyboard
    /// navigation, mouse clicks, search jumps) must call this explicitly.
    pub fn break_edit_merge(&mut self) {
        self.edit_merge_anchor = None;
    }

    fn undo_redo(&mut self, undo: bool) {
        self.edit_merge_anchor = None;
        let buffer_generation = self.buffer.generation();
        let mut entry_buffer_generation = None;
        let mut damage_start = CoordType::MAX;

        loop {
            // Transfer the last entry from the undo stack to the redo stack or vice versa.
            {
                let (from, to) = if undo {
                    (&mut self.undo_stack, &mut self.redo_stack)
                } else {
                    (&mut self.redo_stack, &mut self.undo_stack)
                };

                // Only pop the entry if its buffer generation matches the previous one
                let Some(g) = from.pop_back_if(|c| {
                    entry_buffer_generation.is_none_or(|g| g == c.borrow().generation_before)
                }) else {
                    break;
                };

                to.push_back(g);
            }

            let change = {
                let to = if undo { &self.redo_stack } else { &self.undo_stack };
                to.back().unwrap()
            };

            // Remember the buffer generation of the change so we can stop popping undos/redos.
            // Also, move to the point where the modification took place.
            let cursor = {
                let change = change.borrow();
                entry_buffer_generation = Some(change.generation_before);
                self.cursor_move_to_logical_internal(self.cursor, change.cursor)
            };

            let safe_cursor = cursor;

            damage_start = damage_start.min(cursor.logical_pos.line as CoordType);

            {
                let mut change = change.borrow_mut();
                let change = &mut *change;

                // Undo: Whatever was deleted is now added and vice versa.
                mem::swap(&mut change.deleted, &mut change.added);

                // Delete the inserted portion.
                self.buffer.allocate_gap(cursor.offset.to_usize(), 0, change.deleted.len());

                // Reinsert the deleted portion.
                {
                    let added = &change.added[..];
                    let mut beg = 0;
                    let mut offset = cursor.offset.to_usize();

                    while beg < added.len() {
                        let (end, line) = simd::lines_fwd(added, beg, 0, 1);
                        let has_newline = line != 0;
                        let link = &added[beg..end];
                        let line = unicode::strip_newline(link);
                        let mut written;

                        {
                            let gap = self.buffer.allocate_gap(offset, line.len() + 2, 0);
                            written = slice_copy_safe(gap, line);

                            if has_newline {
                                if self.newlines_are_crlf && written < gap.len() {
                                    gap[written] = b'\r';
                                    written += 1;
                                }
                                if written < gap.len() {
                                    gap[written] = b'\n';
                                    written += 1;
                                }
                            }

                            self.buffer.commit_gap(written);
                        }

                        beg = end;
                        offset += written;
                    }
                }

                // Restore the previous line statistics.
                mem::swap(&mut self.stats, &mut change.stats_before);

                // Restore the previous selection.
                mem::swap(&mut self.selection, &mut change.selection_before);

                // Pretend as if the buffer was never modified.
                self.buffer.set_generation(change.generation_before);
                change.generation_before = buffer_generation;

                // Restore the previous cursor.
                let cursor_before =
                    self.cursor_move_to_logical_internal(safe_cursor, change.cursor_before);
                change.cursor_before = self.cursor.logical_pos;
                // Can't use `set_cursor_internal` here, because we haven't updated the line stats yet.
                self.cursor = cursor_before;

                if self.undo_stack.is_empty() {
                    self.last_history_type = HistoryType::Other;
                }
            }
        }

        if damage_start == CoordType::MAX {
            // There weren't any undo/redo entries.
            return;
        }

        self.highlighter_cache.invalidate_from(damage_start);

        if entry_buffer_generation.is_some() {
            self.recalc_after_content_changed();
        }
    }

    /// Replace a byte range with new content. Used by find-and-replace.
    pub fn replace_range(&mut self, range: std::ops::Range<usize>, replacement: &[u8]) {
        self.replace_range_with_history(range, replacement, EditHistoryKind::Standalone);
    }

    /// Replace a byte range with new content, recording undo history per `kind`.
    ///
    /// `Insert`/`Delete` edits coalesce with an immediately adjacent edit of the
    /// same kind (continuous typing / backspace runs), matching source-mode undo
    /// granularity. `Standalone` edits always form their own undo entry.
    pub fn replace_range_with_history(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &[u8],
        kind: EditHistoryKind,
    ) {
        if range.is_empty() && replacement.is_empty() {
            return;
        }
        // Move cursor to start of range
        let start = self.cursor_move_to_byte_internal(self.cursor, ByteIndex(range.start));
        // Move end cursor to end of range for deletion
        let end = self.cursor_move_to_byte_internal(start, ByteIndex(range.end));

        let coalesce = self
            .edit_merge_anchor
            .is_some_and(|anchor| anchor.continues(kind, &range, replacement));
        // Force `edit_begin` to either join the anchored entry or start a fresh one:
        // caret syncs between edits reset `last_history_type`, so the adjacency-checked
        // anchor is the source of truth for coalescing.
        self.last_history_type = if coalesce { kind.into() } else { HistoryType::Other };
        self.edit_begin(kind.into(), start);
        self.edit_replace(start, end, replacement);
        self.edit_end();

        self.edit_merge_anchor = EditMergeAnchor::after_edit(kind, &range, replacement);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_with_text(text: &str) -> TextBuffer {
        let mut buffer = TextBuffer::new(false).expect("test buffer must be created");
        buffer.set_crlf(false);
        buffer.set_insert_final_newline(false);
        buffer.write_raw(text.as_bytes());
        buffer.mark_as_clean();
        // Treat the initial content like a freshly loaded document: no undo history.
        buffer.undo_stack.clear();
        buffer
    }

    fn buffer_text(buffer: &mut TextBuffer) -> String {
        let mut text = String::new();
        buffer.save_as_string(&mut text);
        text
    }

    #[test]
    fn replace_range_with_nonempty_replacement_increments_generation_once_and_undoes_once() {
        let mut buffer = TextBuffer::new(false).expect("test buffer must be created");
        buffer.set_crlf(false);
        buffer.set_insert_final_newline(false);
        buffer.write_raw(b"hello world");
        let generation_before = buffer.generation();

        buffer.replace_range(5..11, b"next");

        assert_eq!(buffer.generation(), generation_before + 1);
        buffer.undo();
        let mut restored_text = String::new();
        buffer.save_as_string(&mut restored_text);
        assert_eq!(restored_text, "hello world");
    }

    #[test]
    fn insert_history_coalesces_adjacent_typing_into_one_undo_entry() {
        let mut buffer = buffer_with_text("");

        for (index, ch) in b"abc".iter().enumerate() {
            // The WYSIWYG caret sync between transactions resets `last_history_type`;
            // coalescing must still apply to adjacent typing.
            buffer.cursor_move_to_byte(ByteIndex(index));
            buffer.replace_range_with_history(index..index, &[*ch], EditHistoryKind::Insert);
        }

        assert_eq!(buffer_text(&mut buffer), "abc");
        assert_eq!(buffer.undo_stack.len(), 1, "adjacent typing must coalesce");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "");
    }

    #[test]
    fn delete_history_coalesces_adjacent_backspaces_into_one_undo_entry() {
        let mut buffer = buffer_with_text("abc");

        for start in (0..3).rev() {
            buffer.cursor_move_to_byte(ByteIndex(start + 1));
            buffer.replace_range_with_history(start..start + 1, b"", EditHistoryKind::Delete);
        }

        assert_eq!(buffer_text(&mut buffer), "");
        assert_eq!(buffer.undo_stack.len(), 1, "adjacent backspaces must coalesce");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "abc");
    }

    #[test]
    fn delete_history_coalesces_adjacent_forward_deletes_into_one_undo_entry() {
        let mut buffer = buffer_with_text("abc");

        for _ in 0..3 {
            buffer.cursor_move_to_byte(ByteIndex(0));
            buffer.replace_range_with_history(0..1, b"", EditHistoryKind::Delete);
        }

        assert_eq!(buffer_text(&mut buffer), "");
        assert_eq!(buffer.undo_stack.len(), 1, "adjacent forward deletes must coalesce");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "abc");
    }

    #[test]
    fn alternating_insert_and_delete_history_splits_undo_entries() {
        let mut buffer = buffer_with_text("");

        buffer.replace_range_with_history(0..0, b"a", EditHistoryKind::Insert);
        buffer.cursor_move_to_byte(ByteIndex(1));
        buffer.replace_range_with_history(0..1, b"", EditHistoryKind::Delete);
        buffer.cursor_move_to_byte(ByteIndex(0));
        buffer.replace_range_with_history(0..0, b"b", EditHistoryKind::Insert);

        assert_eq!(buffer_text(&mut buffer), "b");
        assert_eq!(buffer.undo_stack.len(), 3, "type changes must split undo entries");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "a");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "");
    }

    #[test]
    fn standalone_replacement_breaks_insert_coalescing() {
        let mut buffer = buffer_with_text("");

        buffer.replace_range_with_history(0..0, b"a", EditHistoryKind::Insert);
        buffer.cursor_move_to_byte(ByteIndex(1));
        buffer.replace_range(1..1, b"-");
        buffer.cursor_move_to_byte(ByteIndex(2));
        buffer.replace_range_with_history(2..2, b"b", EditHistoryKind::Insert);

        assert_eq!(buffer_text(&mut buffer), "a-b");
        assert_eq!(buffer.undo_stack.len(), 3, "standalone edits must never coalesce");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "a-");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "a");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "");
    }

    #[test]
    fn insert_history_does_not_coalesce_after_caret_moves_elsewhere() {
        let mut buffer = buffer_with_text("xy");

        buffer.replace_range_with_history(0..0, b"a", EditHistoryKind::Insert);
        buffer.cursor_move_to_byte(ByteIndex(3));
        buffer.replace_range_with_history(3..3, b"b", EditHistoryKind::Insert);

        assert_eq!(buffer_text(&mut buffer), "axyb");
        assert_eq!(buffer.undo_stack.len(), 2, "non-adjacent typing must not coalesce");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "axy");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "xy");
    }

    #[test]
    fn break_edit_merge_starts_a_new_undo_entry_at_the_same_byte() {
        let mut buffer = buffer_with_text("");

        buffer.replace_range_with_history(0..0, b"a", EditHistoryKind::Insert);
        // The post-transaction caret sync must keep the anchor alive ...
        buffer.cursor_move_to_byte(ByteIndex(1));
        // ... but explicit user navigation away and back splits the run, even
        // though the caret returns to the exact byte where the last edit ended.
        buffer.break_edit_merge();
        buffer.cursor_move_to_byte(ByteIndex(1));
        buffer.replace_range_with_history(1..1, b"b", EditHistoryKind::Insert);

        assert_eq!(buffer_text(&mut buffer), "ab");
        assert_eq!(buffer.undo_stack.len(), 2, "user navigation must split the typing run");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "a");
        buffer.undo();
        assert_eq!(buffer_text(&mut buffer), "");
    }
}
