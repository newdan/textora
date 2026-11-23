use super::*;

impl<S: BlockSource> PreviewEngine<S> {
    // ── Scroll ──

    pub fn scroll(&mut self, delta: f32, viewport_h: f32) -> bool {
        self.pending_cursor_reveal = None;
        let max_scroll = (self.content_height - viewport_h).max(0.0);
        let old = self.scroll_y;
        self.scroll_y = (self.scroll_y + delta).clamp(0.0, max_scroll);
        if (self.scroll_y - old).abs() > 0.5 {
            self.pending_heading_jump = None;
            true
        } else {
            false
        }
    }

    // ── Selection ──

    pub fn hit_test(&self, px: f32, py: f32, offset_x: f32, offset_y: f32) -> Option<ViewPos> {
        if let Some(composition) = &self.replacement_preedit {
            return composition.engine.hit_test(px, py, offset_x, offset_y);
        }
        let lazy = self.lazy.as_ref()?;
        selection::hit_test(&lazy.flat_lines, self.scroll_y, px, py, offset_x, offset_y)
    }

    pub fn selection_range(&self) -> Option<(ViewPos, ViewPos)> {
        self.sel.range()
    }
    pub fn clear_selection(&mut self) {
        self.sel.clear();
        self.sel_anchor_byte = None;
        self.sel_cursor_byte = None;
        self.mark_selection_changed();
    }

    pub fn select_all(&mut self) {
        if let Some(lazy) = &self.lazy {
            self.sel.select_all(&lazy.flat_lines);
        } else if self.edit_source.is_none() {
            return;
        }
        self.sel_anchor_byte = self.edit_source.as_ref().map(|_| 0);
        self.sel_cursor_byte = self.edit_source.as_deref().map(|source| source.len());
        self.mark_selection_changed();
    }

    pub fn word_at_pos(&self, pos: ViewPos) -> (ViewPos, ViewPos) {
        if let Some(composition) = &self.replacement_preedit {
            return composition.engine.word_at_pos(pos);
        }
        let Some(ref lazy) = self.lazy else { return (pos, pos) };
        selection::word_at_pos(&lazy.flat_lines, pos)
    }

    pub fn line_range_at_pos(&self, pos: ViewPos) -> (ViewPos, ViewPos) {
        if let Some(composition) = &self.replacement_preedit {
            return composition.engine.line_range_at_pos(pos);
        }
        let Some(ref lazy) = self.lazy else {
            return (
                ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: GraphemeIdx::new(0) },
                ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: GraphemeIdx::new(0) },
            );
        };
        selection::line_range_at_pos(&lazy.flat_lines, pos)
    }

    pub fn selected_text(&self) -> Option<String> {
        let lazy = self.lazy.as_ref()?;
        self.sel.selected_text(&lazy.flat_lines)
    }

    pub fn selected_reading_text(&self, source: &str) -> Option<String> {
        let lazy = self.lazy.as_ref()?;
        self.sel.selected_reading_text(&lazy.flat_lines, source)
    }

    pub(in crate::view) fn byte_selection_range(&self) -> Option<(usize, usize)> {
        if let (Some(anchor), Some(cursor)) = (self.sel_anchor_byte, self.sel_cursor_byte) {
            return (anchor != cursor).then_some((anchor.min(cursor), anchor.max(cursor)));
        }

        let (start, end) = self.sel.range()?;
        let start_byte =
            self.byte_from_flat_line_and_visual_grapheme(start.flat_line_idx, start.grapheme_pos)?;
        let end_byte =
            self.byte_from_flat_line_and_visual_grapheme(end.flat_line_idx, end.grapheme_pos)?;

        if start_byte <= end_byte {
            Some((start_byte, end_byte))
        } else {
            Some((end_byte, start_byte))
        }
    }

    pub fn selection_source_range(&self) -> Option<(usize, usize)> {
        self.byte_selection_range()
    }

    pub fn selection_highlights(&self, sel_color: [f32; 4]) -> DrawList {
        if self.replacement_preedit.is_some() {
            return DrawList::new();
        }
        let Some(ref lazy) = self.lazy else { return DrawList::new() };
        let Some((start, end)) =
            self.sel.range().or_else(|| self.visual_range_for_byte_selection())
        else {
            return DrawList::new();
        };
        let selection = SelectionState { anchor: Some(start), cursor: Some(end) };
        self.clip_code_highlights(selection.highlights(
            &lazy.flat_lines,
            self.scroll_y,
            self.cached_offset_x,
            self.cached_offset_y,
            self.cached_dl_viewport.1,
            sel_color,
        ))
    }

    pub fn has_selection(&self) -> bool {
        self.sel.has_selection() || self.byte_selection_range().is_some()
    }

    pub fn set_sel_cursor(&mut self, pos: Option<(usize, usize)>) {
        self.sel.cursor =
            pos.map(|(line_idx, grapheme_pos)| ViewPos::from_indices(line_idx, grapheme_pos));
        self.sel_cursor_byte = None;
        self.mark_selection_changed();
    }

    pub fn set_sel_anchor(&mut self, pos: Option<(usize, usize)>) {
        self.sel.anchor =
            pos.map(|(line_idx, grapheme_pos)| ViewPos::from_indices(line_idx, grapheme_pos));
        self.sel_anchor_byte = None;
        self.mark_selection_changed();
    }

    pub fn set_sel_cursor_byte(&mut self, byte: Option<usize>) {
        if let Some(b) = byte {
            self.sel_cursor_byte = Some(b);
            self.sel.cursor = self.find_flat_and_grapheme_for_byte(b);
        } else {
            self.sel.cursor = None;
            self.sel_cursor_byte = None;
        }
        self.mark_selection_changed();
    }

    pub fn set_sel_anchor_byte(&mut self, byte: Option<usize>) {
        if let Some(b) = byte {
            self.sel_anchor_byte = Some(b);
            self.sel.anchor = self.find_flat_and_grapheme_for_byte(b);
        } else {
            self.sel.anchor = None;
            self.sel_anchor_byte = None;
        }
        self.mark_selection_changed();
    }

    pub(in crate::view) fn mark_selection_changed(&mut self) {
        self.update_replacement_preedit();
        if matches!(self.dirty, EngineDirty::Clean | EngineDirty::SelectionChanged) {
            self.dirty = EngineDirty::SelectionChanged;
        }
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    // ── Search ──

    pub fn search_highlights(
        &self,
        query: &str,
        case_sensitive: bool,
        _use_regex: bool,
        active_match_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    ) -> DrawList {
        if query.is_empty() || self.lazy.is_none() {
            return DrawList::new();
        }
        // Search uses its own internal update; we pass generation=0 as search
        // invalidation is handled by the view layer.
        self.search.update_if_needed(
            query,
            case_sensitive,
            0,
            &self.lazy.as_ref().expect("lazy layout must exist for search").flat_lines,
        );
        self.clip_code_highlights(self.search.highlights(
            self.scroll_y,
            self.cached_dl_viewport.1,
            self.cached_offset_x,
            self.cached_offset_y,
            active_match_idx,
            match_color,
            inactive_color,
        ))
    }

    pub fn scroll_to_search_match(&mut self, query: &str, case_sensitive: bool, active_idx: usize) {
        if query.is_empty() {
            return;
        }
        if let Some(ref lazy) = self.lazy {
            self.search.update_if_needed(query, case_sensitive, 0, &lazy.flat_lines);
        }
        let viewport_h = self.cached_dl_viewport.1;
        self.scroll_y = self.search.scroll_to(active_idx, viewport_h, self.scroll_y);
    }

    // ── Vertex cache ──

    pub fn cache_vertices(&mut self, verts: Vec<GlyphVertex>) {
        self.cached_vertices = Some(verts);
    }

    pub fn get_cached_vertices(&self) -> Option<&Vec<GlyphVertex>> {
        self.cached_vertices.as_ref()
    }

    // ── Flat lines ──

    pub fn flat_lines(&self) -> &[crate::layout::FlatLine] {
        if let Some(composition) = &self.replacement_preedit {
            return composition.engine.flat_lines();
        }
        self.lazy.as_ref().map_or(&[], |l| &l.flat_lines)
    }

    /// Expose canonical projection boundaries for tests.
    #[cfg(test)]
    pub fn flat_line_projection_boundaries(&self) -> Vec<FlatLineProjectionBoundaries> {
        if let Some(composition) = &self.replacement_preedit {
            return composition
                .engine
                .flat_line_projection_boundaries()
                .into_iter()
                .map(|mut line| {
                    for byte in &mut line.boundaries {
                        *byte = composition.source_byte(*byte);
                    }
                    line
                })
                .collect();
        }
        self.lazy
            .as_ref()
            .map(|lazy| {
                lazy.flat_lines
                    .iter()
                    .filter_map(|line| {
                        line.source_projection.as_ref().map(|projection| {
                            FlatLineProjectionBoundaries {
                                flat_idx: line.flat_idx,
                                boundaries: projection
                                    .boundaries
                                    .iter()
                                    .map(|anchor| anchor.byte.get())
                                    .collect(),
                            }
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn sel_mut(&mut self) -> &mut SelectionState {
        &mut self.sel
    }

    // ── WYSIWYG edit support ──

    /// 接收来自 app 层的光标位置变更通知。
    pub fn handle_set_cursor_byte(&mut self, byte: usize) {
        if self.pending_cursor_reveal.is_some() {
            self.pending_cursor_reveal = Some(byte);
        }
        let query_layout_ready = self.dirty == EngineDirty::QueryLayoutReady;
        let previous_byte = self.edit_ctx.as_ref().map(|context| context.cursor_byte);
        self.mark_cursor_moved(byte);
        if query_layout_ready && previous_byte != Some(byte) {
            self.refresh_query_cursor_layout(previous_byte, byte);
        }
    }

    /// 设置 WYSIWYG 编辑的完整源码文本。用于 span 展开时读取原始 markers。
    pub fn set_edit_source(&mut self, source: Option<String>) {
        self.edit_source = source.map(MarkdownSource::snapshot);
    }

    pub(crate) fn set_source_generation(&mut self, source_generation: u32) {
        if self.source_generation == source_generation {
            return;
        }

        self.source_generation = source_generation;
        if let Some(lazy) = self.lazy.as_mut() {
            lazy.set_source_generation(source_generation);
        }
        self.mark_source_dirty();
    }
}
