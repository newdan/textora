//! Document view: combines loaded file content with viewport scrolling.
//!
//! Uses TextBuffer (from core) as the single source of truth for all content
//! and editing operations. Line offsets are cached for efficient visible-line
//! extraction; the cache is rebuilt after each edit.
//!
//! ## Selection state
//!
//! `selection_anchor: Option<usize>` is the single source of truth for selections.
//! `tb`'s internal selection is **not** synchronized — it is only filled briefly
//! inside `delete_selection` to reuse `extract_user_selection`.
//! After any operation that may touch tb's selection, callers must NOT read
//! tb's selection state — always go through `selection_anchor` / `selection_range()`.
//!
//! ## Cursor offset
//!
//! `cursor_offset` must only be set through `set_cursor_offset_synced()` (for
//! caller-computed offsets) or `sync_cursor_offset_from_tb()` (when tb already
//! holds the correct position). Direct field assignment is forbidden outside of
//! these methods to guarantee `tb.cursor_offset()` stays in sync.

use crate::line_index::{LineIndex, count_graphemes_before, grapheme_to_byte};
use std::borrow::Cow;

use crate::document_presentation::DocumentPresentation;
use appkit_core::document::{CursorState, DocumentModel};
pub(crate) mod edit;
pub(crate) mod selection;
pub(crate) mod visible;
use core::buffer::text_buffer::{CursorMovement, TextBuffer};
use core::document::ReadableDocument;
use core::document::{DocView, DocViewMut};
use core::file;
use core::highlight::{FILE_ASSOCIATIONS, Highlight, HighlightKind};
use core::types::{ByteIndex, UniCharOffset};
use std::path::Path;

pub use appkit_core::document::DocumentSaveError;

/// Holds the loaded document content and viewport state.
///
/// TextBuffer is the single source of truth for content, cursor, and undo/redo.
/// Line offsets are cached for efficient rendering and rebuilt after each edit.
pub struct DocumentView {
    /// Headless document state. Compatibility deref is temporary and removed
    /// in Task 16D.
    pub model: DocumentModel,
    /// Rebuildable viewport/render/search presentation state.
    pub(crate) presentation: DocumentPresentation,
}

impl std::ops::Deref for DocumentView {
    type Target = DocumentModel;

    fn deref(&self) -> &Self::Target {
        &self.model
    }
}

impl std::ops::DerefMut for DocumentView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.model
    }
}

impl DocumentView {
    pub(crate) fn into_parts(self) -> (DocumentModel, DocumentPresentation) {
        (self.model, self.presentation)
    }

    pub(crate) fn from_parts(model: DocumentModel, presentation: DocumentPresentation) -> Self {
        Self { model, presentation }
    }

    /// Read-only access to cursor state.
    pub fn cursor(&self) -> &CursorState {
        &self.model.cursor
    }
    /// Mutable access to cursor state.
    pub fn cursor_mut(&mut self) -> &mut CursorState {
        &mut self.model.cursor
    }

    pub(crate) fn take_presentation(&mut self) -> DocumentPresentation {
        let visible_rows = self.presentation.display.viewport.visible_rows;
        let viewport_height = self.presentation.display.viewport.viewport_height;
        std::mem::replace(
            &mut self.presentation,
            DocumentPresentation::new(visible_rows, viewport_height),
        )
    }

    pub(crate) fn restore_presentation(&mut self, presentation: DocumentPresentation) {
        self.presentation = presentation;
    }

    /// Create a new document view from pre-split lines (for testing).
    pub fn new(lines: Vec<String>, visible_rows: usize, viewport_height: f64) -> Self {
        let mut tb = TextBuffer::new(false).expect("TextBuffer creation failed");
        let content = lines.join("\n");
        if !content.is_empty() {
            let sanitized = replace_null_bytes(content.as_bytes());
            tb.write_raw(&sanitized);
        }
        tb.mark_as_clean();
        tb.cursor_move_to_byte(ByteIndex::ZERO);
        let model = DocumentModel::new(tb);
        let presentation = DocumentPresentation::new(visible_rows, viewport_height);

        Self { model, presentation }
    }

    pub(crate) fn from_external_content(
        path: &Path,
        content: &str,
        visible_rows: usize,
        viewport_height: f64,
    ) -> Self {
        let crlf = content.contains("\r\n");
        let normalized = content.replace("\r\n", "\n");
        let mut document = Self::new(
            normalized.split('\n').map(str::to_owned).collect(),
            visible_rows,
            viewport_height,
        );
        document.file_path = Some(path.to_owned());
        document.disk_revision = crate::file_safety::capture_revision(path).ok();
        document.crlf = crlf;
        document.set_language_from_path(path);
        document
    }

    pub(crate) fn restore_edit_position(
        &mut self,
        cursor_offset: usize,
        selection_anchor: Option<usize>,
        scroll_anchor: ui::viewport::ScrollAnchor,
    ) {
        let buffer_length = self.buffer_len();
        self.set_cursor_offset_synced(cursor_offset.min(buffer_length));
        self.cursor.selection_anchor = selection_anchor.map(|offset| offset.min(buffer_length));
        self.presentation.display.viewport.scroll_anchor.doc_line =
            scroll_anchor.doc_line.min(self.line_count().saturating_sub(1));
        self.presentation.display.viewport.scroll_anchor.pixel_offset =
            scroll_anchor.pixel_offset.max(0.0);
    }

    pub fn set_language_from_path(&mut self, path: &Path) {
        self.language = path.extension().and_then(|ext| ext.to_str()).and_then(|ext| {
            FILE_ASSOCIATIONS
                .iter()
                .find(|(pattern, _)| *pattern == ext || pattern.ends_with(&format!(".{ext}")))
                .map(|(_, lang)| *lang)
        });
        self.presentation.display.render_cache.invalidate_all();
    }

    /// Load a file from disk.
    pub fn from_file(
        path: &Path,
        visible_rows: usize,
        viewport_height: f64,
    ) -> Result<Self, String> {
        let _ff_t0 = std::time::Instant::now();
        let (buffer, _meta) =
            file::load_file(path).map_err(|e| format!("failed to load {}: {e}", path.display()))?;
        let disk_revision = crate::file_safety::capture_revision(path).ok();
        let _ff_load_us = _ff_t0.elapsed().as_micros();

        // Quick scan for null bytes (zero-copy)
        let has_null = {
            let total = buffer.len();
            let mut off = 0;
            let mut found = false;
            while off < total {
                let chunk = buffer.read_forward(off);
                let end = chunk.len().min(total - off);
                if chunk[..end].contains(&0) {
                    found = true;
                    break;
                }
                off += end;
            }
            found
        };

        let mut tb = if has_null {
            // Rare path: extract, replace null bytes, write through edit pipeline
            let content_bytes: Vec<u8> = {
                let total = buffer.len();
                if total == 0 {
                    Vec::new()
                } else {
                    let chunk = buffer.read_forward(0);
                    if chunk.len() >= total { chunk[..total].to_vec() } else { chunk.to_vec() }
                }
            };
            let mut tb = TextBuffer::new(false).expect("TextBuffer creation failed");
            if !content_bytes.is_empty() {
                let sanitized = replace_null_bytes(&content_bytes);
                tb.write_raw(&sanitized);
            }
            tb
        } else {
            // Fast path: zero-copy from GapBuffer
            TextBuffer::from_gap_buffer(buffer).expect("TextBuffer creation failed")
        };
        let _ff_write_us = _ff_t0.elapsed().as_micros();

        if _meta.line_ending == core::file::LineEnding::Crlf {
            tb.set_crlf(true);
        }
        let had_bom = _meta.had_bom;
        tb.mark_as_clean();
        tb.cursor_move_to_byte(ByteIndex::ZERO);

        // Detect language from file extension for syntax highlighting.
        let language = path.extension().and_then(|ext| ext.to_str()).and_then(|ext| {
            FILE_ASSOCIATIONS
                .iter()
                .find(|(pattern, _)| *pattern == ext || pattern.ends_with(&format!(".{ext}")))
                .map(|(_, lang)| *lang)
        });

        let mut model = DocumentModel::new(tb);
        model.file_path = Some(path.to_path_buf());
        model.disk_revision = disk_revision;
        model.dirty = _meta.original_encoding.is_some();
        model.crlf = _meta.line_ending == core::file::LineEnding::Crlf;
        model.had_bom = had_bom;
        model.original_encoding = _meta.original_encoding;
        model.language = language;

        let _ff_total = _ff_t0.elapsed().as_micros();
        eprintln!(
            "[perf:from_file] load={}us scan_write={}us index={}us total={}us lines={} null={}",
            _ff_load_us,
            _ff_write_us - _ff_load_us,
            _ff_total - _ff_write_us,
            _ff_total,
            model.line_index.offsets.len(),
            has_null
        );
        let presentation = DocumentPresentation::new(visible_rows, viewport_height);

        Ok(Self { model, presentation })
    }

    /// Save the document to its current file path.
    ///
    /// Save the document to its current file path.
    pub fn save(&mut self) -> Result<(), DocumentSaveError> {
        self.model.save()
    }

    /// Save the document to the given path (atomic write).
    ///
    /// Preserves line endings and BOM based on the metadata tracked since load.
    /// On success, updates `file_path`, clears the dirty flag, and marks the
    /// TextBuffer as clean.
    pub fn save_as(&mut self, path: &std::path::Path) -> Result<(), DocumentSaveError> {
        self.model.save_as(path)
    }

    pub(crate) fn content_revision(&self) -> u64 {
        self.content_revision
    }

    pub(crate) fn mark_content_changed(&mut self) {
        self.content_revision = self.content_revision.saturating_add(1);
    }

    pub(crate) fn classify_external_change(
        &self,
        observed: Option<&crate::file_safety::DiskRevision>,
        explicit_rename: Option<&crate::file_safety::DiskRevision>,
        rename_candidates: &[crate::file_safety::DiskRevision],
    ) -> crate::external_document_change::ExternalDocumentChange {
        crate::external_document_change::classify_external_change(
            self.disk_revision.as_ref(),
            observed,
            self.dirty,
            explicit_rename,
            rename_candidates,
        )
    }

    // ── TextBuffer access ──────────────────────────────────────────────

    /// Borrow the underlying TextBuffer (for word_select, etc.).
    pub fn tb(&self) -> &TextBuffer {
        &self.tb
    }

    pub fn cursor_offset(&self) -> ByteIndex {
        self.tb.cursor_offset()
    }

    // ── Content reading ──────────────────────────────────────────────

    /// Get the byte offset of a document line.
    pub fn line_byte_offset(&self, doc_line: usize) -> Option<usize> {
        self.line_index.offsets.get(doc_line).copied()
    }

    /// Byte length of a document line (including trailing newline if any).
    pub fn line_byte_length(&self, doc_line: usize) -> Option<usize> {
        self.line_index.lengths.get(doc_line).copied()
    }

    pub fn doc_bytes_in_range(&self, range: std::ops::Range<usize>) -> Option<Cow<'_, [u8]>> {
        let length = range.len();
        if length == 0 {
            return Some(Cow::Borrowed(&[]));
        }
        let total = self.tb.text_length();
        if range.start >= total {
            return Some(Cow::Borrowed(&[]));
        }
        // Limit length to the end of the text buffer
        let length = length.min(total - range.start);

        // Fast path: the range doesn't span a gap boundary — return borrowed.
        let chunk = self.tb.read_forward(range.start);
        if chunk.len() >= length {
            return Some(Cow::Borrowed(&chunk[..length]));
        }
        // Slow path: range spans gap buffer boundary — collect into Vec.
        let mut result = Vec::with_capacity(length);
        let mut pos = range.start;
        while result.len() < length && pos < total {
            let chunk = self.tb.read_forward(pos);
            if chunk.is_empty() {
                break;
            }
            let need = length - result.len();
            let take = need.min(chunk.len());
            result.extend_from_slice(&chunk[..take]);
            pos += take;
        }
        Some(Cow::Owned(result))
    }

    /// Get the raw bytes of a document line by index.
    pub fn doc_line_bytes(&self, doc_line: usize) -> Option<Cow<'_, [u8]>> {
        let offset = self.line_index.offsets.get(doc_line).copied()?;
        let length = self.line_index.lengths.get(doc_line).copied()?;
        self.doc_bytes_in_range(offset..offset + length)
    }

    /// Total number of lines in the document.
    pub fn line_count(&self) -> usize {
        LineIndex::line_count(&self.line_index)
    }

    /// Whether the document is empty.
    pub fn is_empty(&self) -> bool {
        self.tb.text_length() == 0
    }

    /// Get the total buffer length in bytes.
    pub fn buffer_len(&self) -> usize {
        self.tb.text_length()
    }

    /// Return the entire buffer content as a UTF-8 `String`.
    ///
    /// Iterates over gap-buffer chunks via ReadableDocument::read_forward.
    pub fn full_text(&self) -> String {
        let total = self.tb.text_length();
        if total == 0 {
            return String::new();
        }
        // Fast path: the whole buffer is one contiguous chunk.
        let first = self.tb.read_forward(0);
        if first.len() >= total {
            return String::from_utf8_lossy(&first[..total]).into_owned();
        }
        // Slow path: collect across chunk boundaries.
        let mut bytes = Vec::with_capacity(total);
        let mut off = 0;
        while off < total {
            let chunk = self.tb.read_forward(off);
            if chunk.is_empty() {
                break;
            }
            let need = total - off;
            bytes.extend_from_slice(&chunk[..need.min(chunk.len())]);
            off += chunk.len();
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Buffer generation counter — increments on every edit.
    pub fn generation(&self) -> u32 {
        self.tb.generation()
    }

    // ── Viewport operations ──────────────────────────────────────────

    pub fn resize(&mut self, visible_rows: usize, viewport_height: f64) {
        self.presentation.display.viewport.resize(visible_rows, viewport_height);
    }
    /// Set CRLF mode. Syncs both the DocumentView flag and the underlying TextBuffer.
    pub fn set_crlf(&mut self, crlf: bool) {
        self.crlf = crlf;
        self.tb.set_crlf(crlf);
    }

    // ── Cursor movement ──────────────────────────────────────────────

    /// Move cursor left by one grapheme. At line start, wraps to previous line end.
    pub fn cursor_move_left(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Grapheme, -1);
        self.sync_cursor();
    }

    /// Move cursor right by one grapheme. At line end, wraps to next line start.
    pub fn cursor_move_right(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Grapheme, 1);
        self.sync_cursor();
    }

    /// Move cursor to a specific byte offset.
    pub fn cursor_move_to_offset(&mut self, offset: usize) {
        self.tb.cursor_move_to_byte(ByteIndex(offset));
        self.sync_cursor();
    }

    /// Move cursor to a document-level unichar offset.
    ///
    /// Delegates to TextBuffer's `cursor_move_to_unichar` via the `LineIndex` lookup.
    pub(crate) fn cursor_move_to_unichar(&mut self, offset: UniCharOffset) {
        let (tb, line_index) = (&mut self.model.tb, &self.model.line_index);
        tb.cursor_move_to_unichar(offset, line_index);
        self.sync_cursor();
    }

    /// Move cursor to a unichar offset using `hint_line` from a hit-test to
    /// disambiguate consecutive empty lines that share the same unichar start.
    pub(crate) fn cursor_move_to_unichar_on_line(
        &mut self,
        offset: UniCharOffset,
        hint_line: usize,
    ) {
        let line_start = self.line_index.unichar_of_line(hint_line);
        let local = offset.to_usize().saturating_sub(line_start.to_usize());
        self.tb
            .cursor_move_to_logical(core::types::LogicalPoint { line: hint_line, unichar: local });
        self.sync_cursor();
    }

    /// Like `set_cursor_offset_synced` but accepts `UniCharOffset`.
    ///
    /// Delegates to TextBuffer's `cursor_move_to_unichar` via the `LineIndex` lookup,
    /// then syncs the DocumentView cursor state.
    pub(crate) fn set_cursor_unichar_synced(&mut self, offset: UniCharOffset) {
        let (tb, line_index) = (&mut self.model.tb, &self.model.line_index);
        tb.cursor_move_to_unichar(offset, line_index);
        self.cursor.offset = self.tb.cursor_offset();
        self.cursor.cached_line = None;
    }

    /// Like `set_cursor_unichar_synced` but uses `hint_line` from hit-test
    /// to disambiguate consecutive empty lines.
    pub(crate) fn set_cursor_unichar_synced_on_line(
        &mut self,
        offset: UniCharOffset,
        hint_line: usize,
    ) {
        let line_start = self.line_index.unichar_of_line(hint_line);
        let local = offset.to_usize().saturating_sub(line_start.to_usize());
        self.tb
            .cursor_move_to_logical(core::types::LogicalPoint { line: hint_line, unichar: local });
        self.cursor.offset = self.tb.cursor_offset();
        self.cursor.cached_line = None;
    }

    /// Convert a document-level `UniCharOffset` to a byte offset.
    /// Uses `line_index.line_at_unichar()` to find the line, then
    /// walks UAX #29 grapheme clusters within the line to locate the exact byte position.
    pub(crate) fn unichar_to_byte_offset(&self, offset: UniCharOffset) -> usize {
        let (line, line_local_grapheme) = self.line_index.line_at_unichar(offset);
        let line_start = self.line_index.offsets[line];
        let line_end = if line + 1 < self.line_index.offsets.len() {
            self.line_index.offsets[line + 1]
        } else {
            self.tb.text_length()
        };
        grapheme_to_byte(&self.tb, line_start, line_end, line_local_grapheme)
    }

    /// Convert a byte offset to a document-level `UniCharOffset`.
    /// Uses `line_index` to find the line, then counts UAX #29 grapheme clusters
    /// up to the byte position within that line.
    pub(crate) fn byte_to_unichar_offset(&self, byte_offset: usize) -> UniCharOffset {
        // Find which line this byte offset belongs to.
        let line = match self.line_index.offsets.binary_search(&byte_offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        };
        let line_start = self.line_index.offsets[line];
        let line_end = if line + 1 < self.line_index.offsets.len() {
            self.line_index.offsets[line + 1]
        } else {
            self.tb.text_length()
        };
        let grapheme_count = count_graphemes_before(&self.tb, line_start, line_end, byte_offset);
        self.line_index.unichar_of_line(line) + grapheme_count
    }

    /// Move cursor left by one word (Option+Left).
    pub fn cursor_move_word_left(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Word, -1);
        self.sync_cursor();
    }

    /// Move cursor right by one word (Option+Right).
    pub fn cursor_move_word_right(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Word, 1);
        self.sync_cursor();
    }

    /// Move cursor to the start of the current line.
    pub fn cursor_move_to_line_start(&mut self) {
        let cursor_column = self.cursor_column();
        self.tb.cursor_move_delta(CursorMovement::Grapheme, -(cursor_column as isize));
        self.sync_cursor();
    }

    /// Move cursor to the first non-whitespace character of the current line (indent start).
    /// Returns the byte offset of the indent position, or line start if no indent.
    pub fn indent_column_offset(&self) -> usize {
        let line = self.cursor_line();
        if line >= self.line_index.offsets.len() {
            return self.cursor.offset.to_usize();
        }
        let line_start = self.line_index.offsets[line];
        let line_len = self.line_index.lengths[line];
        if line_len == 0 {
            return line_start;
        }
        // Read the line content from the buffer using read_forward
        let total = self.tb.text_length();
        let mut result = Vec::with_capacity(line_len);
        let mut i = line_start;
        while result.len() < line_len && i < total {
            let chunk = self.tb.read_forward(i);
            if chunk.is_empty() {
                break;
            }
            let need = line_len - result.len();
            let take = need.min(chunk.len());
            result.extend_from_slice(&chunk[..take]);
            i += take;
        }
        // Find first non-whitespace byte
        for (j, &b) in result.iter().enumerate() {
            if b != b' ' && b != b'\t' {
                return line_start + j;
            }
        }
        // All whitespace → go to line end (or line start for empty indent)
        line_start
    }

    /// Move cursor to the end of the current line (before newline).
    pub fn cursor_move_to_line_end(&mut self) {
        let line = self.cursor_line();
        if line < self.line_index.lengths.len() {
            let line_end_offset = self.line_index.offsets[line] + self.line_index.lengths[line];
            self.tb.cursor_move_to_byte(ByteIndex(line_end_offset));
            self.sync_cursor();
        }
    }

    /// Move cursor up one visual line, preserving column position.
    pub fn cursor_move_up(&mut self) {
        let pos = self.tb.cursor_logical_pos();
        if pos.line > 0 {
            // TODO: 后续引入 LineMap 后，应改为基于 VisualPoint 移动，以修复折行时的上下漂移问题
            self.tb.cursor_move_to_logical(core::types::LogicalPoint {
                unichar: pos.unichar,
                line: pos.line - 1,
            });
            self.sync_cursor();
        }
    }

    /// Move cursor down one visual line, preserving column position.
    pub fn cursor_move_down(&mut self) {
        let pos = self.tb.cursor_logical_pos();
        // TODO: 后续引入 LineMap 后，应改为基于 VisualPoint 移动，以修复折行时的上下漂移问题
        self.tb.cursor_move_to_logical(core::types::LogicalPoint {
            unichar: pos.unichar,
            line: pos.line + 1,
        });
        self.sync_cursor();
    }

    /// Ensure the cursor is roughly visible by scrolling if the document line is out of bounds.
    pub fn ensure_cursor_visible(&mut self, line_height: f32) {
        let cursor_line = self.cursor_line();
        let visible_range = self
            .presentation
            .display
            .viewport
            .visible_doc_range_from_anchor(&self.presentation.display.display_map, line_height);
        let anchor = self.scroll_anchor_doc_line();

        if cursor_line >= visible_range.start && cursor_line < visible_range.end {
            return;
        }

        if cursor_line < anchor {
            self.presentation.cursor_render_state.click_hint = None; // viewport scrolled
            self.set_scroll_anchor_and_refresh(
                ui::viewport::ScrollAnchor::new(cursor_line, 0.0),
                line_height,
            );
        } else if cursor_line >= visible_range.end {
            let visible_count = visible_range.len().max(1);
            let new_anchor = cursor_line.saturating_sub(visible_count.saturating_sub(1));
            self.presentation.cursor_render_state.click_hint = None; // viewport scrolled
            self.set_scroll_anchor_and_refresh(
                ui::viewport::ScrollAnchor::new(new_anchor, 0.0),
                line_height,
            );
        }
    }

    /// Page up: scroll viewport up by one screenful of pixels.
    pub fn page_up(&mut self, line_height: f32) {
        let page_pixels =
            (self.presentation.display.viewport.visible_rows.max(1) as f32) * line_height;
        self.scroll_viewport_by_pixels(-page_pixels, line_height);
        let first_doc = self
            .presentation
            .display
            .viewport
            .visible_doc_range_from_anchor(&self.presentation.display.display_map, line_height)
            .start;
        if let Some(offset) = self.line_byte_offset(first_doc) {
            self.set_cursor_offset_synced(offset);
        }
        self.presentation.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
    }

    /// Page down: scroll viewport down by one screenful of pixels.
    pub fn page_down(&mut self, line_height: f32) {
        let page_pixels =
            (self.presentation.display.viewport.visible_rows.max(1) as f32) * line_height;
        self.scroll_viewport_by_pixels(page_pixels, line_height);
        let first_doc = self
            .presentation
            .display
            .viewport
            .visible_doc_range_from_anchor(&self.presentation.display.display_map, line_height)
            .start;
        if let Some(offset) = self.line_byte_offset(first_doc) {
            self.set_cursor_offset_synced(offset);
        }
        self.presentation.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
    }

    pub fn viewport_anchor_doc_line(&self) -> usize {
        self.scroll_anchor_doc_line()
    }

    pub fn scroll_doc_lines_for_viewport(&mut self, doc_line_delta: isize, line_height: f32) {
        if self.line_count() == 0 {
            return;
        }

        if self.presentation.display.display_map.line_count() == self.line_count() {
            self.presentation
                .display
                .viewport
                .scroll_doc_lines(doc_line_delta, &self.presentation.display.display_map);
            self.refresh_scroll_metrics(line_height);
            return;
        }

        let max_doc_line = self.line_count().saturating_sub(1);
        let next_doc_line = if doc_line_delta.is_negative() {
            self.scroll_anchor_doc_line().saturating_sub(doc_line_delta.unsigned_abs())
        } else {
            self.scroll_anchor_doc_line().saturating_add(doc_line_delta as usize)
        }
        .min(max_doc_line);
        self.presentation.display.viewport.scroll_anchor =
            ui::viewport::ScrollAnchor::new(next_doc_line, 0.0);
        self.presentation.display.viewport.scroll_top = next_doc_line as f64;
    }

    pub fn scroll_to_doc_line_for_viewport(&mut self, doc_line: usize, line_height: f32) {
        if self.line_count() == 0 {
            return;
        }

        let target_doc_line = doc_line.min(self.line_count().saturating_sub(1));
        if self.presentation.display.display_map.line_count() == self.line_count() {
            self.set_scroll_anchor_and_refresh(
                ui::viewport::ScrollAnchor::new(target_doc_line, 0.0),
                line_height,
            );
            return;
        }

        self.presentation.display.viewport.scroll_anchor =
            ui::viewport::ScrollAnchor::new(target_doc_line, 0.0);
        self.presentation.display.viewport.scroll_top = target_doc_line as f64;
    }

    /// Move cursor visually (up or down)
    pub(crate) fn move_cursor_visual(
        &mut self,
        delta: isize,
        ctx: crate::cursor_motion::CursorContext,
    ) {
        if let Some(offset) = crate::cursor_motion::move_cursor_visual(delta, ctx, self) {
            self.set_cursor_offset_synced(offset.to_usize());
        }
        self.presentation.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
    }

    /// Get the line index (0-based) that contains the cursor.
    pub fn cursor_line(&self) -> usize {
        // O(log N) binary search, cached per cursor_offset
        if let Some((cached_offset, cached_line)) = self.cursor.cached_line
            && cached_offset == self.cursor.offset
        {
            return cached_line;
        }
        // Cache miss — compute and store (caller must use cursor_line_cached for caching)
        self.line_index
            .offsets
            .partition_point(|&offset| offset <= self.cursor.offset.to_usize())
            .saturating_sub(1)
    }

    /// Get cursor_line with caching. Call this when cursor_offset won't change.
    pub fn cursor_line_cached(&mut self) -> usize {
        if let Some((cached_offset, cached_line)) = self.cursor.cached_line
            && cached_offset == self.cursor.offset
        {
            return cached_line;
        }
        let line = self
            .line_index
            .offsets
            .partition_point(|&offset| offset <= self.cursor.offset.to_usize())
            .saturating_sub(1);
        self.cursor.cached_line = Some((self.cursor.offset, line));
        line
    }

    /// Invalidate cursor_line cache (call after cursor_offset changes).
    /// Get the cursor column (byte offset within the line).
    pub fn cursor_column(&self) -> usize {
        let line = self.cursor_line();
        if line < self.line_index.offsets.len() {
            self.cursor.offset.to_usize().saturating_sub(self.line_index.offsets[line])
        } else {
            0
        }
    }

    // ── Viewport rebuild ─────────────────────────────────────────────

    /// Sync cursor offset and dirty flag from TextBuffer after an edit (full rebuild).

    pub fn rebuild_viewport(&mut self) {
        self.line_index = LineIndex::rebuild_from(&self.tb);
        let _total = self.line_index.line_count().max(1);
    }

    /// Execute search for the current query against the text buffer.
    /// Updates SearchState with results and jumps cursor to first match.
    pub fn perform_search(&mut self) {
        let query = self.presentation.search_state.query.clone();
        if query.is_empty() {
            self.presentation.search_state.matches.clear();
            self.presentation.search_state.active_match_idx = 0;
            self.presentation.search_state.buffer_generation = self.tb.gap_buffer().generation();
            return;
        }

        let chunk1 = self.tb.gap_buffer().read_forward(0);
        let chunk2 = self.tb.gap_buffer().read_forward(chunk1.len());

        let query_bytes = query.as_bytes();
        let search_fn: fn(&[u8], &[u8]) -> Vec<std::ops::Range<usize>> =
            if self.presentation.search_state.options.match_case {
                core::buffer::simd_search::find_all
            } else {
                core::buffer::simd_search::find_all_case_insensitive_ascii
            };

        let mut matches = Vec::new();

        // Search first chunk
        if !chunk1.is_empty() {
            matches.extend(search_fn(query_bytes, chunk1));
        }

        // Search across the gap
        if !chunk1.is_empty() && !chunk2.is_empty() && query_bytes.len() > 1 {
            let cross_len = query_bytes.len() - 1;
            let take1 = cross_len.min(chunk1.len());
            let take2 = cross_len.min(chunk2.len());

            let mut cross_buf = Vec::with_capacity(take1 + take2);
            cross_buf.extend_from_slice(&chunk1[chunk1.len() - take1..]);
            cross_buf.extend_from_slice(&chunk2[..take2]);

            let cross_matches = search_fn(query_bytes, &cross_buf);
            for m in cross_matches {
                let start_in_doc = chunk1.len() - take1 + m.start;
                matches.push(start_in_doc..start_in_doc + query_bytes.len());
            }
        }

        // Search second chunk
        if !chunk2.is_empty() {
            let m2 = search_fn(query_bytes, chunk2);
            for m in m2 {
                matches.push(m.start + chunk1.len()..m.end + chunk1.len());
            }
        }

        let generation = self.tb.gap_buffer().generation();
        self.presentation.search_state.update_matches(matches, generation);

        // Jump to first match
        if self.presentation.search_state.active_match().is_some() {
            let range = self.presentation.search_state.matches[0].clone();
            self.set_cursor_offset_synced(range.start);
            self.cursor.selection_anchor = Some(range.start);
            self.set_cursor_offset_synced(range.end);
        }
    }

    /// Returns the cached highlight spans for the given logical line.
    pub fn highlights_for_line(&mut self, line_index: usize) -> &[Highlight<HighlightKind>] {
        use core::highlight::Highlighter as CoreHighlighter;
        use stdext::arena::scratch_arena;

        let language = match self.language {
            Some(lang) => lang,
            None => return &[],
        };

        let arena = scratch_arena(None);
        let model = &self.model;
        let mut highlighter = CoreHighlighter::new(&model.tb, language);
        self.presentation.highlighter_cache.parse_line(
            &arena,
            &mut highlighter,
            line_index as core::helpers::CoordType,
            |line| model.line_index.offsets.get(line as usize).copied().unwrap_or(0),
        )
    }

    pub fn invalidate_highlights_from(&mut self, line: isize) {
        self.presentation.highlighter_cache.invalidate_from(line);
    }

    fn scroll_anchor_doc_line(&self) -> usize {
        self.presentation.display.viewport.scroll_anchor.doc_line
    }

    fn refresh_scroll_metrics(&mut self, line_height: f32) {
        self.presentation
            .display
            .viewport
            .clamp_anchor(&self.presentation.display.display_map, line_height);
        self.presentation
            .display
            .viewport
            .derive_scroll_top(&self.presentation.display.display_map, line_height);
    }

    fn set_scroll_anchor_and_refresh(
        &mut self,
        anchor: ui::viewport::ScrollAnchor,
        line_height: f32,
    ) {
        self.presentation.display.viewport.scroll_anchor = anchor;
        self.refresh_scroll_metrics(line_height);
    }

    fn scroll_viewport_by_pixels(&mut self, pixels: f32, line_height: f32) {
        self.presentation.display.viewport.scroll_pixels(
            pixels,
            &self.presentation.display.display_map,
            line_height,
        );
        self.refresh_scroll_metrics(line_height);
    }

    pub(crate) fn click_hint(&self) -> Option<(UniCharOffset, usize)> {
        self.presentation.cursor_render_state.click_hint
    }

    pub(crate) fn set_click_hint(&mut self, offset: UniCharOffset, vis_line: usize) {
        self.presentation.cursor_render_state.click_hint = Some((offset, vis_line));
    }

    pub(crate) fn clear_click_hint(&mut self) {
        self.presentation.cursor_render_state.click_hint = None;
    }

    pub(crate) fn note_pointer_cursor_x(&mut self, px: f32) {
        self.presentation.cursor_render_state.sticky_x = px;
        self.presentation.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
    }

    pub(crate) fn page_step_rows(&self) -> usize {
        self.presentation.display.viewport.visible_rows.saturating_sub(1).max(1)
    }

    pub(crate) fn sub_line_pixel_offset(&self, line_height: f32) -> f32 {
        self.presentation.display.viewport.sub_line_pixel_offset(line_height)
    }
}

impl DocView for DocumentView {
    fn line_count(&self) -> usize {
        LineIndex::line_count(&self.line_index)
    }

    fn doc_line_text(&self, line: usize) -> Cow<'_, str> {
        let bytes = self.doc_line_bytes(line).unwrap_or(Cow::Borrowed(&[]));
        match bytes {
            Cow::Borrowed(b) => Cow::Borrowed(
                std::str::from_utf8(b)
                    .expect("AST range must align with valid UTF-8 character boundaries"),
            ),
            Cow::Owned(b) => Cow::Owned(
                String::from_utf8(b)
                    .expect("AST range must align with valid UTF-8 character boundaries"),
            ),
        }
    }

    fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> Cow<'_, str> {
        let bytes = self.doc_bytes_in_range(range).unwrap_or(Cow::Borrowed(&[]));
        match bytes {
            Cow::Borrowed(b) => Cow::Borrowed(
                std::str::from_utf8(b)
                    .expect("AST range must align with valid UTF-8 character boundaries"),
            ),
            Cow::Owned(b) => Cow::Owned(
                String::from_utf8(b)
                    .expect("AST range must align with valid UTF-8 character boundaries"),
            ),
        }
    }

    fn line_byte_offset(&self, line: usize) -> usize {
        self.line_index.offsets.get(line).copied().unwrap_or(0)
    }

    fn line_byte_length(&self, line: usize) -> usize {
        self.line_index.lengths.get(line).copied().unwrap_or(0)
    }

    fn scroll_y(&self) -> f32 {
        self.presentation.display.viewport.scroll_top as f32
    }

    fn viewport_height(&self) -> f32 {
        self.presentation.display.viewport.viewport_height as f32
    }

    fn is_empty(&self) -> bool {
        self.tb.text_length() == 0
    }
}

impl DocViewMut for DocumentView {
    fn set_scroll_y(&mut self, y: f32) {
        self.presentation.display.viewport.scroll_top = y as f64;
    }

    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.tb.replace_range(range, text.as_bytes());
        // replace_range 之后需要重建 line_index
        self.rebuild_viewport();
        self.dirty = true;
    }

    fn begin_edit(&mut self) {
        self.tb.edit_begin_grouping();
    }

    fn end_edit(&mut self) {
        self.tb.edit_end_grouping();
        self.rebuild_viewport();
        self.dirty = true;
    }
}

/// Replace null bytes (\0) with Unicode replacement character U+FFFD.
///
/// Null bytes are invalid in text content and would cause display issues.
/// U+FFFD (encoded as \xEF\xBF\xBD in UTF-8) is the standard replacement character.
pub(crate) fn replace_null_bytes(data: &[u8]) -> Vec<u8> {
    if !data.contains(&0) {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    for &b in data {
        if b == 0 {
            // U+FFFD in UTF-8
            out.extend_from_slice(&[0xEF, 0xBF, 0xBD]);
        } else {
            out.push(b);
        }
    }
    out
}

/// Normalize external clipboard text for insertion into the editor.
/// - Converts CRLF and CR line endings to LF
/// - Strips UTF-8 BOM if present
pub fn normalize_paste_text(input: &[u8]) -> Vec<u8> {
    let (_, stripped) = file::strip_bom(input);
    let mut out = Vec::with_capacity(stripped.len());
    let mut i = 0;
    let len = stripped.len();

    while i < len {
        match stripped[i] {
            b'\r' => {
                out.push(b'\n');
                i += 1;
                if i < len && stripped[i] == b'\n' {
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod basic_tests;
#[cfg(test)]
mod boundary_tests;
#[cfg(test)]
mod cursor_visual_tests;
#[cfg(test)]
mod normalize_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod word_wrap_tests;
