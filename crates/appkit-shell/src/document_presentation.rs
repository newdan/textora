use crate::cursor_motion::CursorRenderState;
use crate::display_state::DisplayState;
use appkit_core::document::SearchState;
use core::highlight::HighlighterCache;

/// Rebuildable presentation state layered on top of a headless document
/// model.
pub struct DocumentPresentation {
    pub display: DisplayState,
    pub highlighter_cache: HighlighterCache,
    pub cursor_render_state: CursorRenderState,
    pub search_state: SearchState,
}

impl DocumentPresentation {
    pub fn new(visible_rows: usize, viewport_height: f64) -> Self {
        Self {
            display: DisplayState::new(visible_rows, viewport_height),
            highlighter_cache: HighlighterCache::new(),
            cursor_render_state: CursorRenderState::new(),
            search_state: SearchState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DocumentPresentation;
    use appkit_core::document::DocumentModel;
    use core::buffer::TextBuffer;
    use core::types::ByteIndex;

    #[test]
    fn presentation_can_be_rebuilt_without_changing_model_state() {
        let mut text_buffer = TextBuffer::new(false)
            .expect("TextBuffer creation should not require presentation state");
        text_buffer.write_raw(b"hello\nworld");
        let model = DocumentModel::new(text_buffer);

        let first = DocumentPresentation::new(10, 120.0);
        let second = DocumentPresentation::new(20, 240.0);

        drop(first);
        drop(second);

        assert_eq!(model.cursor.offset, ByteIndex::ZERO);
        assert_eq!(model.line_index.line_count(), 2);
        assert!(!model.dirty);
    }
}
