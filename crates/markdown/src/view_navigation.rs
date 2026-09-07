//! Source updates publish interaction geometry before the next paint.

use super::*;

impl PreviewEngine<MarkdownDoc> {
    pub(super) fn prepare_current_source_layout(&mut self, source: &str) {
        let Some(style) = self.cached_query_style.clone() else { return };
        let document_view = core::document::StringDocView::new(source);
        let parsed = crate::parser::parse_markdown(source);
        let document = MarkdownDoc::build_for_editing(&parsed, &style, source);
        let mut layout = LazyLayout::new(document, &style, self.cached_viewport_w, &document_view);
        layout.set_source_generation(self.source_generation);
        layout.set_edit_source(self.edit_source.clone());
        layout.set_edit_ctx(self.edit_ctx.clone());
        layout.set_selection_range(self.byte_selection_range().map(|(start, end)| start..end));
        if incremental_layout_reuse_enabled()
            && let Some(previous) = self.lazy.take()
        {
            layout.reuse_unchanged_blocks_from(previous);
        }
        layout.ensure_all_blocks(&style, self.cached_viewport_w, None, None, &document_view);
        layout.build_flat_lines(&document_view);
        self.content_height = layout.total_height;
        self.lazy = Some(layout);
        self.dirty = EngineDirty::QueryLayoutReady;
        self.collect_headings();
    }
}

impl<S: BlockSource> PreviewEngine<S> {
    pub(super) fn refresh_query_cursor_layout(&mut self, previous: Option<usize>, current: usize) {
        let Some(style) = self.cached_query_style.as_ref() else { return };
        let Some(source) = self.edit_source.as_deref() else { return };
        let Some(layout) = self.lazy.as_mut() else { return };
        let document_view = core::document::StringDocView::new(source);
        layout.set_edit_ctx(self.edit_ctx.clone());
        layout.invalidate_lines_for_source_bytes(
            previous.into_iter().chain(std::iter::once(current)),
        );
        layout.ensure_all_blocks(style, self.cached_viewport_w, None, None, &document_view);
        layout.build_flat_lines(&document_view);
        self.content_height = layout.total_height;
        self.dirty = EngineDirty::QueryLayoutReady;
    }
}
