use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use core::buffer::text_buffer::{CursorMovement, TextBuffer};
use core::highlight::{FILE_ASSOCIATIONS, Language};
use core::types::{ByteIndex, LogicalPoint, UniCharOffset};

use crate::document::CursorState;
use crate::file_safety::DiskRevision;
use crate::line_index::LineIndex;

/// Headless document model that owns persistent editing state without any
/// viewport or rendering data.
pub struct DocumentModel {
    pub tb: TextBuffer,
    pub line_index: LineIndex,
    pub file_path: Option<PathBuf>,
    pub disk_revision: Option<DiskRevision>,
    pub content_revision: u64,
    pub cursor: CursorState,
    pub dirty: bool,
    pub dirty_snapshot_id: Option<String>,
    pub crlf: bool,
    pub had_bom: bool,
    pub original_encoding: Option<&'static str>,
    pub language: Option<&'static Language>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum DocumentSaveError {
    Untitled,
    ConcurrentModification,
    Io { message: String },
}

impl fmt::Display for DocumentSaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untitled => formatter.write_str("document has no file path"),
            Self::ConcurrentModification => {
                formatter.write_str("file changed externally during save")
            }
            Self::Io { message } => write!(formatter, "save failed: {message}"),
        }
    }
}

impl std::error::Error for DocumentSaveError {}

impl DocumentModel {
    pub fn new(tb: TextBuffer) -> Self {
        let line_index = LineIndex::rebuild_from(&tb);
        Self {
            tb,
            line_index,
            file_path: None,
            disk_revision: None,
            content_revision: 0,
            cursor: CursorState::new(),
            dirty: false,
            dirty_snapshot_id: None,
            crlf: false,
            had_bom: false,
            original_encoding: None,
            language: None,
        }
    }

    pub fn cursor(&self) -> &CursorState {
        &self.cursor
    }

    pub fn cursor_mut(&mut self) -> &mut CursorState {
        &mut self.cursor
    }

    pub fn text_buffer(&self) -> &TextBuffer {
        &self.tb
    }

    pub fn tb(&self) -> &TextBuffer {
        &self.tb
    }

    pub fn cursor_offset(&self) -> ByteIndex {
        self.tb.cursor_offset()
    }

    pub fn line_byte_offset(&self, document_line: usize) -> Option<usize> {
        self.line_index.offsets.get(document_line).copied()
    }

    pub fn line_byte_length(&self, document_line: usize) -> Option<usize> {
        self.line_index.lengths.get(document_line).copied()
    }

    pub fn document_bytes_in_range(&self, range: std::ops::Range<usize>) -> Cow<'_, [u8]> {
        let requested_length = range.len();
        if requested_length == 0 {
            return Cow::Borrowed(&[]);
        }
        let total_length = self.tb.text_length();
        if range.start >= total_length {
            return Cow::Borrowed(&[]);
        }
        let available_length = requested_length.min(total_length - range.start);
        let first_chunk = self.tb.read_forward(range.start);
        if first_chunk.len() >= available_length {
            return Cow::Borrowed(&first_chunk[..available_length]);
        }

        let mut bytes = Vec::with_capacity(available_length);
        let mut offset = range.start;
        while bytes.len() < available_length && offset < total_length {
            let chunk = self.tb.read_forward(offset);
            if chunk.is_empty() {
                break;
            }
            let take = (available_length - bytes.len()).min(chunk.len());
            bytes.extend_from_slice(&chunk[..take]);
            offset += take;
        }
        Cow::Owned(bytes)
    }

    pub fn document_line_bytes(&self, document_line: usize) -> Option<Cow<'_, [u8]>> {
        let offset = self.line_byte_offset(document_line)?;
        let length = self.line_byte_length(document_line)?;
        Some(self.document_bytes_in_range(offset..offset + length))
    }

    pub fn doc_bytes_in_range(&self, range: std::ops::Range<usize>) -> Cow<'_, [u8]> {
        self.document_bytes_in_range(range)
    }

    pub fn doc_line_bytes(&self, document_line: usize) -> Option<Cow<'_, [u8]>> {
        self.document_line_bytes(document_line)
    }

    pub fn line_count(&self) -> usize {
        self.line_index.line_count()
    }

    pub fn is_empty(&self) -> bool {
        self.tb.text_length() == 0
    }

    pub fn buffer_len(&self) -> usize {
        self.tb.text_length()
    }

    pub fn full_text(&self) -> String {
        let total_length = self.tb.text_length();
        let mut bytes = Vec::with_capacity(total_length);
        let mut offset = 0;
        while offset < total_length {
            let chunk = self.tb.read_forward(offset);
            if chunk.is_empty() {
                break;
            }
            let take = (total_length - offset).min(chunk.len());
            bytes.extend_from_slice(&chunk[..take]);
            offset += take;
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub fn generation(&self) -> u32 {
        self.tb.generation()
    }

    pub fn mark_content_changed(&mut self) {
        self.content_revision = self.content_revision.saturating_add(1);
    }

    pub fn content_revision(&self) -> u64 {
        self.content_revision
    }

    pub fn set_language_from_path(&mut self, path: &Path) {
        self.language =
            path.extension().and_then(|extension| extension.to_str()).and_then(|extension| {
                FILE_ASSOCIATIONS
                    .iter()
                    .find(|(pattern, _)| {
                        *pattern == extension || pattern.ends_with(&format!(".{extension}"))
                    })
                    .map(|(_, language)| *language)
            });
    }

    pub fn save(&mut self) -> Result<(), DocumentSaveError> {
        let path = self.file_path.clone().ok_or(DocumentSaveError::Untitled)?;
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), DocumentSaveError> {
        let expected_revision = (self.file_path.as_deref() == Some(path))
            .then_some(self.disk_revision.as_ref())
            .flatten();
        let contents = self.serialized_contents_for_save();
        let revision = core::file::save_file_if_unchanged(path, &contents, expected_revision)
            .map_err(map_save_error)?;
        self.file_path = Some(path.to_path_buf());
        self.disk_revision = Some(revision);
        self.dirty = false;
        self.tb.mark_as_clean();
        Ok(())
    }

    /// Produce an immutable byte snapshot for an off-thread save.
    pub fn serialized_contents_for_save(&self) -> Vec<u8> {
        let text = self.full_text();
        let normalized = if self.crlf { text.replace('\n', "\r\n") } else { text };
        if !self.had_bom {
            return normalized.into_bytes();
        }
        let mut contents = Vec::with_capacity(normalized.len() + 3);
        contents.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        contents.extend_from_slice(normalized.as_bytes());
        contents
    }

    /// Apply a successful worker save without clearing newer edits.
    ///
    /// The disk revision always records the worker's actual write. The clean baseline is
    /// updated only when the saved content revision is still current.
    pub fn apply_save_completion(
        &mut self,
        path: PathBuf,
        saved_content_revision: u64,
        disk_revision: DiskRevision,
    ) -> bool {
        self.file_path = Some(path);
        self.disk_revision = Some(disk_revision);
        if self.content_revision != saved_content_revision {
            return false;
        }
        self.dirty = false;
        self.tb.mark_as_clean();
        true
    }

    pub fn set_cursor_offset_synced(&mut self, offset: usize) {
        self.tb.cursor_move_to_byte(ByteIndex(offset));
        self.cursor.offset = self.tb.cursor_offset();
        self.cursor.cached_line = None;
    }

    pub fn sync_cursor_from_buffer(&mut self) {
        self.cursor.selection_anchor = None;
        self.cursor.offset = self.tb.cursor_offset();
        self.cursor.cached_line = None;
    }

    pub fn sync_cursor(&mut self) {
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_to_offset(&mut self, offset: usize) {
        self.tb.cursor_move_to_byte(ByteIndex(offset));
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_to_unichar(&mut self, offset: UniCharOffset) {
        self.tb.cursor_move_to_unichar(offset, &self.line_index);
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_to_unichar_on_line(&mut self, offset: UniCharOffset, document_line: usize) {
        let line_start = self.line_index.unichar_of_line(document_line);
        let local_offset = offset.to_usize().saturating_sub(line_start.to_usize());
        self.tb.cursor_move_to_logical(LogicalPoint { line: document_line, unichar: local_offset });
        self.sync_cursor_from_buffer();
    }

    pub fn set_cursor_unichar_synced_on_line(
        &mut self,
        offset: UniCharOffset,
        document_line: usize,
    ) {
        let selection_anchor = self.cursor.selection_anchor;
        self.cursor_move_to_unichar_on_line(offset, document_line);
        self.cursor.selection_anchor = selection_anchor;
    }

    pub fn unichar_to_byte_offset(&self, offset: UniCharOffset) -> usize {
        let (line, line_local_grapheme) = self.line_index.line_at_unichar(offset);
        let line_start = self.line_index.offsets[line];
        let line_end =
            self.line_index.offsets.get(line + 1).copied().unwrap_or_else(|| self.tb.text_length());
        crate::line_index::grapheme_to_byte(&self.tb, line_start, line_end, line_local_grapheme)
    }

    pub fn byte_to_unichar_offset(&self, byte_offset: usize) -> UniCharOffset {
        let line = match self.line_index.offsets.binary_search(&byte_offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        };
        let line_start = self.line_index.offsets[line];
        let line_end =
            self.line_index.offsets.get(line + 1).copied().unwrap_or_else(|| self.tb.text_length());
        let grapheme_count =
            crate::line_index::count_graphemes_before(&self.tb, line_start, line_end, byte_offset);
        self.line_index.unichar_of_line(line) + grapheme_count
    }

    pub fn cursor_move_left(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Grapheme, -1);
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_right(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Grapheme, 1);
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_word_left(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Word, -1);
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_word_right(&mut self) {
        self.tb.cursor_move_delta(CursorMovement::Word, 1);
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_to_line_start(&mut self) {
        let line_start = self.line_byte_offset(self.cursor_line()).unwrap_or(0);
        self.cursor_move_to_offset(line_start);
    }

    pub fn cursor_move_to_line_end(&mut self) {
        let line = self.cursor_line();
        let line_start = self.line_byte_offset(line).unwrap_or(0);
        let line_length = self.line_byte_length(line).unwrap_or(0);
        self.cursor_move_to_offset(line_start + line_length);
    }

    pub fn cursor_move_up(&mut self) {
        let position = self.tb.cursor_logical_pos();
        if position.line == 0 {
            return;
        }
        self.tb.cursor_move_to_logical(LogicalPoint {
            unichar: position.unichar,
            line: position.line - 1,
        });
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_move_down(&mut self) {
        let position = self.tb.cursor_logical_pos();
        self.tb.cursor_move_to_logical(LogicalPoint {
            unichar: position.unichar,
            line: position.line + 1,
        });
        self.sync_cursor_from_buffer();
    }

    pub fn cursor_line(&self) -> usize {
        if let Some((cached_offset, cached_line)) = self.cursor.cached_line
            && cached_offset == self.cursor.offset
        {
            return cached_line;
        }
        self.line_index
            .offsets
            .partition_point(|&offset| offset <= self.cursor.offset.to_usize())
            .saturating_sub(1)
    }

    pub fn cursor_line_cached(&mut self) -> usize {
        if let Some((cached_offset, cached_line)) = self.cursor.cached_line
            && cached_offset == self.cursor.offset
        {
            return cached_line;
        }
        let line = self.cursor_line();
        self.cursor.cached_line = Some((self.cursor.offset, line));
        line
    }

    pub fn cursor_column(&self) -> usize {
        let line = self.cursor_line();
        let line_start = self.line_index.offsets.get(line).copied().unwrap_or(0);
        self.cursor.offset.to_usize().saturating_sub(line_start)
    }

    pub fn has_selection(&self) -> bool {
        self.cursor.selection_anchor.is_some()
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let cursor = self.cursor.offset.to_usize();
        self.cursor
            .selection_anchor
            .map(|anchor| if anchor <= cursor { (anchor, cursor) } else { (cursor, anchor) })
    }

    pub fn clear_selection(&mut self) {
        self.cursor.selection_anchor = None;
    }

    pub fn select_all(&mut self) {
        self.cursor.selection_anchor = Some(0);
        self.set_cursor_offset_synced(self.buffer_len());
    }

    pub fn word_select_at(&self, offset: usize) -> (usize, usize) {
        if self.tb.text_length() == 0 || offset >= self.tb.text_length() {
            return (offset, offset);
        }
        let range = core::buffer::word_select(&self.tb, offset);
        (range.start, range.end)
    }

    pub fn ensure_selection_active(&mut self) {
        if self.cursor.selection_anchor.is_none() {
            self.cursor.selection_anchor = Some(self.cursor.offset.to_usize());
        }
    }

    pub fn extend_selection_left(&mut self) {
        self.ensure_selection_active();
        self.tb.cursor_move_delta(CursorMovement::Grapheme, -1);
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    pub fn extend_selection_right(&mut self) {
        self.ensure_selection_active();
        self.tb.cursor_move_delta(CursorMovement::Grapheme, 1);
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    pub fn extend_selection_up(&mut self) {
        self.ensure_selection_active();
        let position = self.tb.cursor_logical_pos();
        if position.line == 0 {
            return;
        }
        self.tb.cursor_move_to_logical(LogicalPoint {
            unichar: position.unichar,
            line: position.line - 1,
        });
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    pub fn extend_selection_down(&mut self) {
        self.ensure_selection_active();
        let position = self.tb.cursor_logical_pos();
        self.tb.cursor_move_to_logical(LogicalPoint {
            unichar: position.unichar,
            line: position.line + 1,
        });
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    pub fn extend_selection_word_left(&mut self) {
        self.ensure_selection_active();
        self.tb.cursor_move_delta(CursorMovement::Word, -1);
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    pub fn extend_selection_word_right(&mut self) {
        self.ensure_selection_active();
        self.tb.cursor_move_delta(CursorMovement::Word, 1);
        self.set_cursor_offset_synced(self.tb.cursor_offset().to_usize());
    }

    pub fn extend_selection_to_line_start(&mut self) {
        self.ensure_selection_active();
        let line_start = self.line_byte_offset(self.cursor_line()).unwrap_or(0);
        self.set_cursor_offset_synced(line_start);
    }

    pub fn extend_selection_to_line_end(&mut self) {
        self.ensure_selection_active();
        let line = self.cursor_line();
        let line_start = self.line_byte_offset(line).unwrap_or(0);
        let line_length = self.line_byte_length(line).unwrap_or(0);
        self.set_cursor_offset_synced(line_start + line_length);
    }

    pub fn extend_selection_to_doc_start(&mut self) {
        self.ensure_selection_active();
        self.set_cursor_offset_synced(0);
    }

    pub fn extend_selection_to_doc_end(&mut self) {
        self.ensure_selection_active();
        self.set_cursor_offset_synced(self.buffer_len());
    }

    pub fn count_selection_chars(&self) -> Option<usize> {
        let (start, end) = self.selection_range()?;
        if start >= end {
            return None;
        }
        Some(String::from_utf8_lossy(&self.document_bytes_in_range(start..end)).chars().count())
    }

    pub fn extract_selected_text(&self) -> Option<Vec<u8>> {
        let (start, end) = self.selection_range()?;
        if start >= end {
            return None;
        }
        Some(self.document_bytes_in_range(start..end).into_owned())
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        if start >= end {
            self.clear_selection();
            return false;
        }

        self.tb.cursor_move_to_byte(ByteIndex(start));
        self.tb.selection_update_offset(end);
        self.tb.extract_user_selection(true);
        self.clear_selection();
        self.sync_after_edit();
        true
    }

    pub fn insert_at_cursor(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let sanitized = replace_null_bytes(bytes);
        self.tb.write_raw(&sanitized);
        self.sync_after_edit();
    }

    pub fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.tb.edit_begin_grouping();
        self.tb.replace_range(range, text.as_bytes());
        self.tb.edit_end_grouping();
        self.sync_after_edit();
    }

    pub fn delete_backward(&mut self, count: usize) {
        if count == 0 || self.cursor.offset == ByteIndex::ZERO {
            return;
        }
        self.tb.delete(CursorMovement::Grapheme, -(count as isize));
        self.sync_after_edit();
    }

    pub fn delete_forward(&mut self, count: usize) {
        if count == 0 || self.cursor.offset.to_usize() >= self.tb.text_length() {
            return;
        }
        self.tb.delete(CursorMovement::Grapheme, count as isize);
        self.sync_after_edit();
    }

    pub fn undo(&mut self) {
        self.tb.undo();
        self.sync_after_edit();
    }

    pub fn redo(&mut self) {
        self.tb.redo();
        self.sync_after_edit();
    }

    /// Breaks any ongoing undo coalescing run. Must be called on user-driven
    /// caret movement (keyboard navigation, mouse clicks, search jumps) so
    /// typing after the move starts a fresh undo entry; never called from
    /// post-transaction caret syncs, which are part of the edit itself.
    pub fn break_edit_merge(&mut self) {
        self.tb.break_edit_merge();
    }

    pub fn indent_column_offset(&self) -> usize {
        let line = self.cursor_line();
        let line_start = self.line_byte_offset(line).unwrap_or(self.cursor.offset.to_usize());
        let Some(line_bytes) = self.document_line_bytes(line) else {
            return line_start;
        };
        line_bytes
            .iter()
            .position(|byte| *byte != b' ' && *byte != b'\t')
            .map_or(line_start, |column| line_start + column)
    }

    fn sync_after_edit(&mut self) {
        self.sync_cursor_from_buffer();
        self.mark_content_changed();
        self.dirty = self.tb.is_dirty();
        self.line_index = LineIndex::rebuild_from(&self.tb);
    }
}

fn map_save_error(error: core::file::SaveError) -> DocumentSaveError {
    match error {
        core::file::SaveError::ConcurrentModification { .. } => {
            DocumentSaveError::ConcurrentModification
        }
        core::file::SaveError::Io { source, .. } => {
            DocumentSaveError::Io { message: source.to_string() }
        }
        core::file::SaveError::ReadOnly => {
            DocumentSaveError::Io { message: "file is read-only".to_owned() }
        }
    }
}

fn replace_null_bytes(bytes: &[u8]) -> Cow<'_, [u8]> {
    if !bytes.contains(&0) {
        return Cow::Borrowed(bytes);
    }
    let mut sanitized = Vec::with_capacity(bytes.len());
    for byte in bytes {
        if *byte == 0 {
            sanitized.extend_from_slice("\u{FFFD}".as_bytes());
        } else {
            sanitized.push(*byte);
        }
    }
    Cow::Owned(sanitized)
}

#[cfg(test)]
mod tests {
    use super::DocumentModel;

    fn model_from_text(text: &str) -> DocumentModel {
        let mut text_buffer = core::buffer::TextBuffer::new(false)
            .expect("TextBuffer creation should not require UI settings");
        text_buffer.write_raw(text.as_bytes());
        text_buffer.mark_as_clean();
        DocumentModel::new(text_buffer)
    }

    #[test]
    fn builds_editing_metadata_without_viewport_state() {
        let mut text_buffer = core::buffer::TextBuffer::new(false)
            .expect("TextBuffer creation should not require UI settings");
        text_buffer.write_raw(b"hello\nworld");

        let model = DocumentModel::new(text_buffer);

        assert_eq!(model.line_index.line_count(), 2);
        assert_eq!(model.cursor.offset, core::types::ByteIndex::ZERO);
        assert_eq!(model.file_path, None);
        assert!(!model.dirty);
    }

    #[test]
    fn exposes_content_cursor_and_selection_without_presentation_state() {
        let mut model = model_from_text("hello\nworld");

        assert_eq!(model.full_text(), "hello\nworld");
        assert_eq!(model.line_count(), 2);
        assert_eq!(model.document_line_bytes(1).as_deref(), Some(&b"world"[..]));

        model.cursor_move_to_offset(8);
        model.cursor_mut().selection_anchor = Some(1);

        assert_eq!(model.cursor_line(), 1);
        assert_eq!(model.cursor_column(), 2);
        assert_eq!(model.selection_range(), Some((1, 8)));
    }

    #[test]
    fn edits_and_navigates_without_a_document_view() {
        let mut model = model_from_text("alpha\nbeta");

        model.cursor_move_to_line_end();
        model.insert_at_cursor(b"!");
        assert_eq!(model.full_text(), "alpha!\nbeta");

        model.cursor_move_down();
        model.cursor_move_to_line_start();
        model.extend_selection_right();
        model.extend_selection_right();
        assert_eq!(model.selection_range(), Some((7, 9)));
        assert_eq!(model.count_selection_chars(), Some(2));

        assert!(model.delete_selection());
        assert_eq!(model.full_text(), "alpha!\nta");

        model.undo();
        assert_eq!(model.full_text(), "alpha!\nbeta");
        model.redo();
        assert_eq!(model.full_text(), "alpha!\nta");
    }

    #[test]
    fn save_snapshot_preserves_crlf_and_bom() {
        let mut model = model_from_text("first\nsecond");
        model.crlf = true;
        model.had_bom = true;

        assert_eq!(model.serialized_contents_for_save(), b"\xEF\xBB\xBFfirst\r\nsecond");
    }

    #[test]
    fn matching_save_completion_clears_dirty_and_records_path() {
        let directory = tempfile::tempdir().expect("save completion test directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "old").expect("save completion baseline should be written");
        let disk_revision = crate::file_safety::capture_revision(&path)
            .expect("save completion baseline revision should be captured");
        let mut model = model_from_text("new");
        model.insert_at_cursor(b"!");
        let content_revision = model.content_revision();

        assert!(model.apply_save_completion(path.clone(), content_revision, disk_revision));
        assert_eq!(model.file_path, Some(path));
        assert!(!model.dirty);
    }

    #[test]
    fn stale_save_completion_keeps_newer_edits_dirty() {
        let directory = tempfile::tempdir().expect("stale save test directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "old").expect("stale save baseline should be written");
        let disk_revision = crate::file_safety::capture_revision(&path)
            .expect("stale save baseline revision should be captured");
        let mut model = model_from_text("new");
        model.insert_at_cursor(b"!");
        let saved_content_revision = model.content_revision();
        model.insert_at_cursor(b"?");

        assert!(!model.apply_save_completion(path, saved_content_revision, disk_revision));
        assert!(model.dirty);
    }
}
