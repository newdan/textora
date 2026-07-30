use std::ops::Range;

use crate::document::DocumentModel;
use core::types::ByteIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub source_generation: u32,
    pub range: Range<usize>,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    pub executed: bool,
    pub dirty_line_start: usize,
    pub dirty_line_end: usize,
    pub line_count_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    StaleGeneration { expected: u32, actual: u32 },
    InvalidRange { start: usize, end: usize, len: usize },
    InvalidCharBoundary { byte: usize },
    InvalidGraphemeBoundary { byte: usize },
}

pub fn apply_text_edit(
    model: &mut DocumentModel,
    edit: TextEdit,
) -> Result<EditOutcome, EditError> {
    validate_source_generation(model, edit.source_generation)?;

    let source = full_text(&model.tb);
    validate_text_edit(&source, &edit)?;

    let old_line_count = model.line_index.line_count();
    let min_line = match model.line_index.offsets.binary_search(&edit.range.start) {
        Ok(index) => index,
        Err(index) => index.saturating_sub(1),
    };
    let max_line = match model.line_index.offsets.binary_search(&edit.range.end) {
        Ok(index) => index,
        Err(index) => index.saturating_sub(1),
    };

    model.tb.edit_begin_grouping();
    model.tb.replace_range(edit.range.clone(), edit.replacement.as_bytes());
    model.tb.edit_end_grouping();

    model.line_index = crate::line_index::LineIndex::rebuild_from(&model.tb);
    model.content_revision = model.content_revision.saturating_add(1);
    model.dirty = model.tb.is_dirty();
    model.cursor.offset = model.tb.cursor_offset();
    model.cursor.cached_line = None;

    let new_line_count = model.line_index.line_count();
    let lines_deleted = old_line_count.saturating_sub(new_line_count);
    let dirty_line_end = (max_line + 1 + lines_deleted).min(old_line_count);

    Ok(EditOutcome {
        executed: true,
        dirty_line_start: min_line,
        dirty_line_end,
        line_count_changed: old_line_count != new_line_count,
    })
}

pub fn validate_text_edit(source: &str, edit: &TextEdit) -> Result<(), EditError> {
    validate_replacement_range(source, &edit.range)
}

fn validate_source_generation(
    model: &DocumentModel,
    source_generation: u32,
) -> Result<(), EditError> {
    let actual = model.tb.generation();
    if source_generation == actual {
        return Ok(());
    }
    Err(EditError::StaleGeneration { expected: source_generation, actual })
}

fn validate_replacement_range(source: &str, range: &Range<usize>) -> Result<(), EditError> {
    let len = source.len();
    if range.start > range.end || range.end > len {
        return Err(EditError::InvalidRange { start: range.start, end: range.end, len });
    }

    if !source.is_char_boundary(range.start) {
        return Err(EditError::InvalidCharBoundary { byte: range.start });
    }
    if !source.is_char_boundary(range.end) {
        return Err(EditError::InvalidCharBoundary { byte: range.end });
    }
    if !is_grapheme_boundary_in_text(source, range.start) {
        return Err(EditError::InvalidGraphemeBoundary { byte: range.start });
    }
    if !is_grapheme_boundary_in_text(source, range.end) {
        return Err(EditError::InvalidGraphemeBoundary { byte: range.end });
    }

    Ok(())
}

fn is_grapheme_boundary_in_text(text: &str, byte: usize) -> bool {
    if byte > text.len() || !text.is_char_boundary(byte) {
        return false;
    }

    let document = text.as_bytes();
    core::unicode::CursorNav::new(&document).goto_byte(ByteIndex(byte)).offset == ByteIndex(byte)
}

fn full_text(text_buffer: &core::buffer::TextBuffer) -> String {
    let total = text_buffer.text_length();
    let mut bytes = Vec::with_capacity(total);
    let mut offset = 0;
    while offset < total {
        let chunk = text_buffer.read_forward(offset);
        if chunk.is_empty() {
            break;
        }
        let remaining = total - offset;
        let take = remaining.min(chunk.len());
        bytes.extend_from_slice(&chunk[..take]);
        offset += take;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{EditError, TextEdit, apply_text_edit};
    use crate::document::DocumentModel;
    use core::buffer::TextBuffer;

    fn model_from_text(text: &str) -> DocumentModel {
        let mut text_buffer = TextBuffer::new(false).expect("TextBuffer creation failed");
        if !text.is_empty() {
            text_buffer.write_raw(text.as_bytes());
        }
        text_buffer.mark_as_clean();
        DocumentModel::new(text_buffer)
    }

    #[test]
    fn apply_text_edit_replaces_range_and_updates_metadata() {
        let mut model = model_from_text("hello world");
        let source_generation = model.tb.generation();

        let outcome = apply_text_edit(
            &mut model,
            TextEdit { source_generation, range: 5..11, replacement: "\n\nnext".into() },
        )
        .expect("valid edit should succeed");

        assert!(outcome.executed);
        assert_eq!(outcome.dirty_line_start, 0);
        assert_eq!(outcome.dirty_line_end, 1);
        assert!(outcome.line_count_changed);
        assert!(model.dirty);
        assert_eq!(model.content_revision, 1);
    }

    #[test]
    fn apply_text_edit_rejects_stale_generation() {
        let mut model = model_from_text("abc");
        let stale_generation = model.tb.generation().wrapping_sub(1);

        let error = apply_text_edit(
            &mut model,
            TextEdit { source_generation: stale_generation, range: 1..2, replacement: "Z".into() },
        )
        .expect_err("stale generation should fail");

        assert!(matches!(error, EditError::StaleGeneration { expected: _, actual: _ }));
    }

    #[test]
    fn apply_text_edit_rejects_range_inside_grapheme_cluster() {
        let mut model = model_from_text("e\u{301}x");
        let source_generation = model.tb.generation();

        let error = apply_text_edit(
            &mut model,
            TextEdit { source_generation, range: 1..3, replacement: "Q".into() },
        )
        .expect_err("grapheme-splitting edit should fail");

        assert_eq!(error, EditError::InvalidGraphemeBoundary { byte: 1 });
    }
}
