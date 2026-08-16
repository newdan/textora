//! A text buffer for a text editor.
//!
//! Implements a Unicode-aware, layout-aware text buffer for terminals.
//! It's based on a gap buffer. It has no line cache and instead relies
//! on the performance of the ucd module for fast text navigation.
//!
//! ---
//!
//! If the project ever outgrows a basic gap buffer (e.g. to add time travel)
//! an ideal, alternative architecture would be a piece table with immutable trees.
//! The tree nodes can be allocated on the same arena allocator as the added chunks,
//! making lifetime management fairly easy. The algorithm is described here:
//! * <https://cdacamar.github.io/data%20structures/algorithms/benchmarking/text%20editors/c++/editor-data-structures/>
//! * <https://github.com/cdacamar/fredbuf>
//!
//! The downside is that text navigation & search takes a performance hit due to small chunks.
//! The solution to the former is to keep line caches, which further complicates the architecture.
//! There's no solution for the latter. However, there's a chance that the performance will still be sufficient.

pub use super::gap_buffer::GapBuffer;
use super::history::*;
use super::search::*;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::io;
use std::mem;
use std::rc::Rc;

use crate::buffer::edit::ActiveEditLineInfo;
use crate::cell::SemiRefCell;
use crate::document::ReadableDocument;
use crate::helpers::*;
use crate::highlight::HighlighterCache;
use crate::highlight::Language;
use crate::types::{ByteIndex, LogicalPoint, UniCharOffset, UnicharLineLookup, VisualPoint};
use crate::unicode::Cursor;
use crate::{icu, simd};

pub enum IoError {
    Io(io::Error),
    Icu(icu::Error),
}

pub type IoResult<T> = std::result::Result<T, IoError>;

impl From<io::Error> for IoError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<icu::Error> for IoError {
    fn from(err: icu::Error) -> Self {
        Self::Icu(err)
    }
}

/// Char- or word-wise navigation? Your choice.
pub enum CursorMovement {
    Grapheme,
    Word,
}

/// See [`TextBuffer::move_selected_lines`].
pub enum MoveLineDirection {
    Up,
    Down,
}

/// The result of a call to `TextBuffer::render()`
pub struct RenderResult {
    /// The maximum visual X position we encountered during rendering.
    pub visual_pos_x_max: CoordType,
}

/// A [`TextBuffer`] with inner mutability.
pub type TextBufferCell = SemiRefCell<TextBuffer>;

/// A [`TextBuffer`] inside an [`Rc`].
///
/// We need this because the TUI system needs to borrow
/// the given text buffer(s) until after the layout process.
pub type RcTextBuffer = Rc<TextBufferCell>;

/// A text buffer for a text editor.
pub struct TextBuffer {
    pub(crate) buffer: GapBuffer,

    pub(crate) undo_stack: VecDeque<SemiRefCell<HistoryEntry>>,
    pub(crate) redo_stack: VecDeque<SemiRefCell<HistoryEntry>>,
    pub(crate) last_history_type: HistoryType,
    pub(crate) edit_merge_anchor: Option<EditMergeAnchor>,
    pub(crate) last_save_generation: u32,

    pub(crate) active_edit_group: Option<ActiveEditGroupInfo>,
    pub(crate) active_edit_line_info: Option<ActiveEditLineInfo>,
    pub(crate) active_edit_depth: i32,
    pub(crate) active_edit_off: usize,

    pub(crate) stats: TextBufferStatistics,
    pub(crate) cursor: Cursor,
    pub(crate) cursor_for_rendering: Option<Cursor>,
    pub(crate) selection: Option<TextBufferSelection>,
    pub(crate) selection_generation: u32,
    pub(crate) search: Option<UnsafeCell<ActiveSearch>>,
    pub(crate) highlighter_cache: HighlighterCache,

    pub(crate) margin_width: CoordType,
    pub(crate) margin_enabled: bool,
    pub(crate) tab_size: CoordType,
    pub(crate) indent_with_tabs: bool,
    pub(crate) line_highlight_enabled: bool,
    pub(crate) language: Option<&'static Language>,
    pub(crate) ruler: CoordType,
    pub(crate) encoding: &'static str,
    pub(crate) newlines_are_crlf: bool,
    pub(crate) insert_final_newline: bool,
    pub(crate) overtype: bool,

    pub(crate) wants_cursor_visibility: bool,
}

impl TextBuffer {
    /// Creates a new text buffer inside an [`Rc`].
    /// See [`TextBuffer::new()`].
    pub fn new_rc(small: bool) -> io::Result<RcTextBuffer> {
        let buffer = Self::new(small)?;
        Ok(Rc::new(SemiRefCell::new(buffer)))
    }

    /// Creates a new text buffer. With `small` you can control
    /// if the buffer is optimized for <1MiB contents.
    pub fn new(small: bool) -> io::Result<Self> {
        Ok(Self {
            buffer: GapBuffer::new(small)?,

            undo_stack: Default::default(),
            redo_stack: Default::default(),
            last_history_type: HistoryType::Other,
            edit_merge_anchor: None,
            last_save_generation: 0,

            active_edit_group: None,
            active_edit_line_info: None,
            active_edit_depth: 0,
            active_edit_off: 0,

            stats: TextBufferStatistics { logical_lines: 1, visual_lines: 1 },
            cursor: Default::default(),
            cursor_for_rendering: None,
            selection: None,
            selection_generation: 0,
            search: None,
            highlighter_cache: HighlighterCache::new(),

            margin_width: 0,
            margin_enabled: false,

            tab_size: 4,
            indent_with_tabs: false,
            line_highlight_enabled: false,
            language: None,
            ruler: 0,
            encoding: "UTF-8",
            newlines_are_crlf: cfg!(windows), // Windows users want CRLF
            insert_final_newline: false, // NOTE: Even with POSIX, single-line buffers need this to be false
            overtype: false,

            wants_cursor_visibility: false,
        })
    }

    /// Length of the document in bytes.
    pub fn text_length(&self) -> usize {
        self.buffer.len()
    }

    /// Access the underlying GapBuffer for direct read operations (e.g. save).
    pub fn gap_buffer(&self) -> &GapBuffer {
        &self.buffer
    }

    /// Create a TextBuffer that takes ownership of an existing GapBuffer.
    /// Used by DocumentView to avoid a double copy when loading files
    /// (file::load_file already built the GapBuffer).
    pub fn from_gap_buffer(gap_buffer: GapBuffer) -> io::Result<Self> {
        let total = gap_buffer.len();
        let logical_lines = if total > 0 {
            let chunk = gap_buffer.read_forward(0);
            let _end = chunk.len().min(total);
            let (_, lines) = simd::lines_fwd(chunk, 0, 0, CoordType::MAX);
            lines + 1
        } else {
            1
        };
        Ok(Self {
            buffer: gap_buffer,
            undo_stack: Default::default(),
            redo_stack: Default::default(),
            last_history_type: HistoryType::Other,
            edit_merge_anchor: None,
            last_save_generation: 0,
            active_edit_group: None,
            active_edit_line_info: None,
            active_edit_depth: 0,
            active_edit_off: 0,
            stats: TextBufferStatistics {
                logical_lines: logical_lines.min(CoordType::MAX),
                visual_lines: logical_lines.min(CoordType::MAX),
            },
            cursor: Default::default(),
            cursor_for_rendering: None,
            selection: None,
            selection_generation: 0,
            search: None,
            highlighter_cache: HighlighterCache::new(),
            margin_width: 0,
            margin_enabled: false,
            tab_size: 4,
            indent_with_tabs: false,
            line_highlight_enabled: false,
            language: None,
            ruler: 0,
            encoding: "UTF-8",
            newlines_are_crlf: cfg!(windows),
            insert_final_newline: false,
            overtype: false,
            wants_cursor_visibility: false,
        })
    }

    /// Number of logical lines in the document,
    /// that is, lines separated by newlines.
    pub fn logical_line_count(&self) -> CoordType {
        self.stats.logical_lines
    }

    /// Number of visual lines in the document,
    /// that is, the number of lines after layout.
    pub fn visual_line_count(&self) -> CoordType {
        self.stats.visual_lines
    }

    /// Does the buffer need to be saved?
    pub fn is_dirty(&self) -> bool {
        self.last_save_generation != self.buffer.generation()
    }

    /// The buffer generation changes on every edit.
    /// With this you can check if it has changed since
    /// the last time you called this function.
    pub fn generation(&self) -> u32 {
        self.buffer.generation()
    }

    /// Force the buffer to be dirty (needs to be saved to disk).
    pub fn mark_as_dirty(&mut self) {
        self.last_save_generation = self.buffer.generation().wrapping_sub(1);
    }

    /// Force the buffer to be clean (has been saved to disk).
    /// Use this with caution. It's called automatically on write().
    pub fn mark_as_clean(&mut self) {
        self.last_save_generation = self.buffer.generation();
    }

    /// The encoding used during reading/writing. "UTF-8" is the default.
    pub fn encoding(&self) -> &'static str {
        self.encoding
    }

    /// Set the encoding used during reading/writing.
    pub fn set_encoding(&mut self, encoding: &'static str) {
        if self.encoding != encoding {
            self.encoding = encoding;
            self.mark_as_dirty();
        }
    }

    /// The newline type used in the document. LF or CRLF.
    pub fn is_crlf(&self) -> bool {
        self.newlines_are_crlf
    }

    /// Changes the newline type without normalizing the document.
    pub fn set_crlf(&mut self, crlf: bool) {
        self.newlines_are_crlf = crlf;
    }

    /// Changes the newline type used in the document.
    ///
    /// NOTE: Cannot be undone.
    pub fn normalize_newlines(&mut self, crlf: bool) {
        let newline: &[u8] = if crlf { b"\r\n" } else { b"\n" };
        let mut off = 0;

        let mut cursor_offset = self.cursor.offset;
        let mut cursor_for_rendering_offset =
            self.cursor_for_rendering.map_or(cursor_offset, |c| c.offset);

        #[cfg(debug_assertions)]
        let mut adjusted_newlines = 0;

        'outer: loop {
            // Seek to the offset of the next line start.
            loop {
                let chunk = self.read_forward(off);
                if chunk.is_empty() {
                    break 'outer;
                }

                let (delta, line) = simd::lines_fwd(chunk, 0, 0, 1);
                off += delta;
                if line == 1 {
                    break;
                }
            }

            // Get the preceding newline.
            let chunk = self.read_backward(off);
            let chunk_newline_len = if chunk.ends_with(b"\r\n") { 2 } else { 1 };
            let chunk_newline = &chunk[chunk.len() - chunk_newline_len..];

            if chunk_newline != newline {
                // If this newline is still before our cursor position, then it still has an effect on its offset.
                // Any newline adjustments past that cursor position are irrelevant.
                let delta = newline.len() as isize - chunk_newline_len as isize;
                if off <= cursor_offset.to_usize() {
                    cursor_offset = cursor_offset.saturating_add_signed(delta);
                    #[cfg(debug_assertions)]
                    {
                        adjusted_newlines += 1;
                    }
                }
                if off <= cursor_for_rendering_offset.to_usize() {
                    cursor_for_rendering_offset =
                        cursor_for_rendering_offset.saturating_add_signed(delta);
                }

                // Replace the newline.
                off -= chunk_newline_len;
                self.buffer.replace(off..off + chunk_newline_len, newline);
                off += newline.len();
            }
        }

        // If this fails, the cursor offset calculation above is wrong.
        #[cfg(debug_assertions)]
        debug_assert_eq!(adjusted_newlines, self.cursor.logical_pos.line);

        self.cursor.offset = cursor_offset;
        if let Some(cursor) = &mut self.cursor_for_rendering {
            cursor.offset = cursor_for_rendering_offset;
        }

        self.newlines_are_crlf = crlf;
    }

    /// If enabled, automatically insert a final newline
    /// when typing at the end of the file.
    pub fn set_insert_final_newline(&mut self, enabled: bool) {
        self.insert_final_newline = enabled;
    }

    /// Whether to insert or overtype text when writing.
    pub fn is_overtype(&self) -> bool {
        self.overtype
    }

    /// Set the overtype mode.
    pub fn set_overtype(&mut self, overtype: bool) {
        self.overtype = overtype;
    }

    /// Gets the logical cursor position, that is,
    /// the position in lines and unichars per line.
    pub fn cursor_logical_pos(&self) -> LogicalPoint {
        self.cursor.logical_pos
    }

    /// Gets the visual cursor position, that is,
    /// the position in laid out rows and columns.
    pub fn cursor_visual_pos(&self) -> VisualPoint {
        self.cursor.visual_pos
    }

    /// Gets the byte offset of the cursor in the buffer.
    pub fn cursor_offset(&self) -> ByteIndex {
        self.cursor.offset
    }

    /// Gets the width of the left margin.
    pub fn margin_width(&self) -> CoordType {
        self.margin_width
    }

    /// Is the left margin enabled?
    pub fn set_margin_enabled(&mut self, enabled: bool) -> bool {
        if self.margin_enabled == enabled {
            false
        } else {
            self.margin_enabled = enabled;
            self.reflow();
            true
        }
    }

    /// Ask the TUI system to scroll the buffer and make the cursor visible.
    ///
    /// TODO: This function shows that [`TextBuffer`] is poorly abstracted
    /// away from the TUI system. The only reason this exists is so that
    /// if someone outside the TUI code enables word-wrap, the TUI code
    /// recognizes this and scrolls the cursor into view. But outside of this
    /// scrolling, views, etc., are all UI concerns = this should not be here.
    pub fn make_cursor_visible(&mut self) {
        self.wants_cursor_visibility = true;
    }

    /// For the TUI code to retrieve a prior [`TextBuffer::make_cursor_visible()`] request.
    pub fn take_cursor_visibility_request(&mut self) -> bool {
        mem::take(&mut self.wants_cursor_visibility)
    }

    /// Set the tab width. Could be anything, but is expected to be 1-8.
    pub fn tab_size(&self) -> CoordType {
        self.tab_size
    }

    /// Set the tab size. Clamped to 1-8.
    pub fn set_tab_size(&mut self, width: CoordType) -> bool {
        let width = width.clamp(1, 8);
        if width == self.tab_size {
            false
        } else {
            self.tab_size = width;
            self.reflow();
            true
        }
    }

    /// Calculates the amount of spaces a tab key press would insert at the given column.
    /// This also equals the visual width of an actual tab character.
    ///
    /// This exists because Rust doesn't have range constraints yet, and without
    /// them assembly blows up in size by 7x. It's a recurring issue with Rust.
    #[inline]
    pub(crate) fn tab_size_eval(&self, column: CoordType) -> CoordType {
        // SAFETY: `set_tab_size` clamps `self.tab_size` to 1-8.
        unsafe { std::hint::assert_unchecked(self.tab_size >= 1 && self.tab_size <= 8) };
        self.tab_size - (column % self.tab_size)
    }

    /// If the cursor is at an indentation of `column`, this returns
    /// the column to which a backspace key press would delete to.
    #[inline]
    pub(crate) fn tab_size_prev_column(&self, column: CoordType) -> CoordType {
        // SAFETY: `set_tab_size` clamps `self.tab_size` to 1-8.
        unsafe { std::hint::assert_unchecked(self.tab_size >= 1 && self.tab_size <= 8) };
        (column - 1).max(0) / self.tab_size * self.tab_size
    }

    /// Returns whether tabs are used for indentation.
    pub fn indent_with_tabs(&self) -> bool {
        self.indent_with_tabs
    }

    /// Sets whether tabs or spaces are used for indentation.
    pub fn set_indent_with_tabs(&mut self, indent_with_tabs: bool) {
        self.indent_with_tabs = indent_with_tabs;
    }

    /// Sets whether the line the cursor is on should be highlighted.
    pub fn set_line_highlight_enabled(&mut self, enabled: bool) {
        self.line_highlight_enabled = enabled;
    }

    pub fn language(&self) -> Option<&'static Language> {
        self.language
    }

    pub fn set_language(&mut self, language: Option<&'static Language>) {
        self.language = language;
        self.highlighter_cache.invalidate_from(0);
    }

    /// Sets a ruler column, e.g. 80.
    pub fn set_ruler(&mut self, column: CoordType) {
        self.ruler = column;
    }

    pub fn reflow(&mut self) {
        self.reflow_internal(true);
    }

    pub(crate) fn recalc_after_content_changed(&mut self) {
        self.reflow_internal(false);
    }

    fn reflow_internal(&mut self, force: bool) {
        // +1 onto logical_lines, because line numbers are 1-based.
        // +1 onto log10, because we want the digit width and not the actual log10.
        // +3 onto log10, because we append " | " to the line numbers to form the margin.
        self.margin_width = if self.margin_enabled {
            self.stats.logical_lines.ilog10() as CoordType + 4
        } else {
            0
        };

        self.cursor_for_rendering = None;

        if force {
            // Recalculate the cursor position.
            self.cursor = self.cursor_move_to_logical_internal(
                self.goto_line_start(self.cursor, self.cursor.logical_pos.line),
                self.cursor.logical_pos,
            );

            // Without word wrap, visual lines == logical lines.
            self.stats.visual_lines = self.stats.logical_lines;
        }
    }

    /// Replaces the entire buffer contents with the given `text`.
    /// Assumes that the line count doesn't change.
    pub fn copy_from_str(&mut self, text: &dyn ReadableDocument) {
        if self.buffer.copy_from(text) {
            self.recalc_after_content_swap();
            self.cursor_move_to_logical(LogicalPoint { unichar: usize::MAX, line: 0 });

            let delete = self.buffer.len() - self.cursor.offset.to_usize();
            if delete != 0 {
                self.buffer.allocate_gap(self.cursor.offset.to_usize(), 0, delete);
            }
        }
    }

    pub(crate) fn recalc_after_content_swap(&mut self) {
        // If the buffer was changed, nothing we previously saved can be relied upon.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_history_type = HistoryType::Other;
        self.edit_merge_anchor = None;
        self.cursor = Default::default();
        self.set_selection(None);
        self.mark_as_clean();
        self.reflow();
        self.highlighter_cache.invalidate_from(0);
    }

    /// Returns the current selection.
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    pub(crate) fn set_selection(&mut self, selection: Option<TextBufferSelection>) -> u32 {
        self.selection = selection.filter(|s| s.beg != s.end);
        self.selection_generation = self.selection_generation.wrapping_add(1);
        self.selection_generation
    }

    /// Moves the cursor to the given byte offset.
    ///
    /// Internal — prefer `cursor_move_to_unichar` for user-facing cursor paths.
    pub fn cursor_move_to_byte(&mut self, offset: ByteIndex) {
        unsafe { self.set_cursor(self.cursor_move_to_byte_internal(self.cursor, offset)) }
    }

    /// Moves the cursor to the given document-level unichar offset.
    ///
    /// Converts the offset to a (line, local_unichar) pair via the provided
    /// `UnicharLineLookup` implementation, then delegates to `cursor_move_to_logical`.
    pub fn cursor_move_to_unichar(
        &mut self,
        offset: UniCharOffset,
        lookup: &dyn UnicharLineLookup,
    ) {
        let (line, local) = lookup.line_at_unichar(offset);
        self.cursor_move_to_logical(LogicalPoint { line, unichar: local });
    }

    /// Moves the cursor to the given logical position.
    pub fn cursor_move_to_logical(&mut self, pos: LogicalPoint) {
        unsafe { self.set_cursor(self.cursor_move_to_logical_internal(self.cursor, pos)) }
    }

    /// Moves the cursor to the given visual position.
    pub fn cursor_move_to_visual(&mut self, pos: VisualPoint) {
        unsafe { self.set_cursor(self.cursor_move_to_visual_internal(self.cursor, pos)) }
    }

    /// Moves the cursor by the given delta.
    pub fn cursor_move_delta(&mut self, granularity: CursorMovement, delta: isize) {
        unsafe { self.set_cursor(self.cursor_move_delta_internal(self.cursor, granularity, delta)) }
    }

    pub fn grapheme_boundary_delta(&self, offset: ByteIndex, delta: isize) -> ByteIndex {
        let cursor = self.cursor_move_to_byte_internal(self.cursor, offset);
        self.cursor_move_delta_internal(cursor, CursorMovement::Grapheme, delta).offset
    }

    pub fn is_grapheme_boundary(&self, offset: ByteIndex) -> bool {
        self.cursor_move_to_byte_internal(self.cursor, offset).offset == offset
    }

    /// Sets the cursor to the given position, and clears the selection.
    ///
    /// # Safety
    ///
    /// This function performs no checks that the cursor is valid. "Valid" in this case means
    /// that the TextBuffer has not been modified since you received the cursor from this class.
    pub unsafe fn set_cursor(&mut self, cursor: Cursor) {
        self.set_cursor_internal(cursor);
        self.last_history_type = HistoryType::Other;
        self.set_selection(None);
    }

    pub(crate) fn set_cursor_for_selection(&mut self, cursor: Cursor) {
        let beg = match self.selection {
            Some(TextBufferSelection { beg, .. }) => beg,
            None => self.cursor.logical_pos,
        };

        self.set_cursor_internal(cursor);
        self.last_history_type = HistoryType::Other;

        let end = self.cursor.logical_pos;
        self.set_selection(if beg == end { None } else { Some(TextBufferSelection { beg, end }) });
    }

    pub(crate) fn set_cursor_internal(&mut self, cursor: Cursor) {
        debug_assert!(cursor.offset.to_usize() <= self.text_length());
        debug_assert!(cursor.logical_pos.line as CoordType <= self.stats.logical_lines);
        debug_assert!(cursor.visual_pos.column >= 0);
        debug_assert!(cursor.visual_pos.row as CoordType <= self.stats.visual_lines);
        self.cursor = cursor;
    }

    /// For interfacing with ICU.
    pub fn read_backward(&self, off: usize) -> &[u8] {
        self.buffer.read_backward(off)
    }

    /// For interfacing with ICU.
    pub fn read_forward(&self, off: usize) -> &[u8] {
        self.buffer.read_forward(off)
    }
}

impl ReadableDocument for TextBuffer {
    fn read_forward(&self, off: usize) -> &[u8] {
        self.buffer.read_forward(off)
    }
    fn read_backward(&self, off: usize) -> &[u8] {
        self.buffer.read_backward(off)
    }
}

#[cfg(test)]
#[path = "text_buffer_tests.rs"]
mod tests;
