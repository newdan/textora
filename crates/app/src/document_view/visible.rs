//! Visible line accessors — methods that map viewport rows to document lines.

use std::borrow::Cow;

use super::DocumentView;

impl DocumentView {
    /// Get visible doc line range using an explicit line height.
    pub(crate) fn visible_doc_range_with_line_height(
        &self,
        line_height: f32,
    ) -> std::ops::Range<usize> {
        let total = self.line_index.line_count();
        if total == 0 {
            return 0..0;
        }
        if self.presentation.display.display_map.line_count() == total {
            self.presentation
                .display
                .viewport
                .visible_doc_range_from_anchor(&self.presentation.display.display_map, line_height)
        } else {
            let max_start = total.saturating_sub(1);
            let start =
                (self.presentation.display.viewport.scroll_top.floor() as usize).min(max_start);
            let visible_count = self.presentation.display.viewport.visible_rows.max(1);
            let end = (start + visible_count).min(total);
            start..end
        }
    }

    /// Internal helper: read bytes for a visible index within a precomputed range.
    fn visible_line_in_range(
        &self,
        vis_idx: usize,
        range: &std::ops::Range<usize>,
    ) -> Option<Cow<'_, [u8]>> {
        let doc_idx = range.start + vis_idx;
        if doc_idx >= range.end || doc_idx >= self.line_index.offsets.len() {
            return None;
        }
        let offset = self.line_index.offsets[doc_idx];
        let length = self.line_index.lengths[doc_idx];
        if length == 0 {
            return Some(Cow::Borrowed(&[]));
        }
        let total = self.tb.text_length();
        if offset >= total {
            return Some(Cow::Borrowed(&[]));
        }
        let chunk = self.tb.read_forward(offset);
        if chunk.len() >= length {
            return Some(Cow::Borrowed(&chunk[..length]));
        }
        let mut result = Vec::with_capacity(length);
        let mut i = offset;
        while result.len() < length && i < total {
            let chunk = self.tb.read_forward(i);
            if chunk.is_empty() {
                break;
            }
            let take = (length - result.len()).min(chunk.len());
            result.extend_from_slice(&chunk[..take]);
            i += take;
        }
        Some(Cow::Owned(result))
    }

    /// Borrow a single visible line as raw bytes using an explicit line height.
    pub fn visible_line_with_line_height(
        &self,
        vis_idx: usize,
        line_height: f32,
    ) -> Option<Cow<'_, [u8]>> {
        let range = self.visible_doc_range_with_line_height(line_height);
        self.visible_line_in_range(vis_idx, &range)
    }

    /// Get a visible line by index using WrapIndex with an explicit line height.
    pub fn visible_line_wrap_with_line_height(
        &self,
        vis_idx: usize,
        line_height: f32,
    ) -> Option<Cow<'_, [u8]>> {
        let range = self.visible_doc_range_with_line_height(line_height);
        self.visible_line_in_range(vis_idx, &range)
    }

    /// Get visible lines as Vec<String> using an explicit line height.
    pub fn visible_lines_with_line_height(&self, line_height: f32) -> Vec<String> {
        let range = self.visible_doc_range_with_line_height(line_height);
        let mut lines = Vec::with_capacity(range.end - range.start);
        for vis_idx in 0..(range.end - range.start) {
            if let Some(bytes) = self.visible_line_in_range(vis_idx, &range) {
                lines.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        lines
    }

    /// Number of visible lines using an explicit line height.
    pub fn visible_line_count_with_line_height(&self, line_height: f32) -> usize {
        let range = self.visible_doc_range_with_line_height(line_height);
        (range.end - range.start).min(self.line_index.offsets.len())
    }

    /// Number of visible lines using WrapIndex with an explicit line height.
    pub fn visible_line_count_wrap_with_line_height(&self, line_height: f32) -> usize {
        let range = self.visible_doc_range_with_line_height(line_height);
        (range.end - range.start).min(self.line_index.offsets.len())
    }

    /// Get the (byte_offset, byte_length) of a visible line using an explicit line height.
    pub fn visible_line_key_with_line_height(
        &self,
        vis_idx: usize,
        line_height: f32,
    ) -> Option<(usize, usize)> {
        let range = self.visible_doc_range_with_line_height(line_height);
        let doc_idx = range.start + vis_idx;
        if doc_idx >= range.end || doc_idx >= self.line_index.offsets.len() {
            return None;
        }
        Some((self.line_index.offsets[doc_idx], self.line_index.lengths[doc_idx]))
    }

    /// Get the (byte_offset, byte_length) of a visible line using WrapIndex with an explicit line height.
    pub fn visible_line_key_wrap_with_line_height(
        &self,
        vis_idx: usize,
        line_height: f32,
    ) -> Option<(usize, usize)> {
        let range = self.visible_doc_range_with_line_height(line_height);
        let doc_idx = range.start + vis_idx;
        if doc_idx >= range.end || doc_idx >= self.line_index.offsets.len() {
            return None;
        }
        Some((self.line_index.offsets[doc_idx], self.line_index.lengths[doc_idx]))
    }
}
