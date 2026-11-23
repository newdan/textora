use super::*;

impl Default for PreviewEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: BlockSource> PreviewEngine<S> {
    pub fn new() -> Self {
        Self {
            lazy: None,
            dirty: EngineDirty::SourceChanged,
            cached_style_hash: 0,
            cached_query_style: None,
            cached_viewport_w: 0.0,
            scroll_y: 0.0,
            content_height: 0.0,
            headings: Vec::new(),
            pending_heading_jump: None,
            pending_cursor_reveal: None,
            cursor_affinity: CursorAffinity::Downstream,
            sel: SelectionState::new(),
            sel_anchor_byte: None,
            sel_cursor_byte: None,
            search: SearchHighlightCache::new(),
            cached_dl: None,
            cached_dl_scroll_y: -1.0,
            cached_dl_viewport: (0.0, 0.0),
            cached_vertices: None,
            cached_offset_x: 0.0,
            cached_offset_y: 0.0,
            base_font_family: ui::settings::platform_default_font_family().to_string(),
            base_font_size: 15.0,
            base_line_height: 24.0,
            rendered_body_font_size: 15.0,
            rendered_line_height: 24.0,
            body_navigation_font_metrics: NavigationFontMetrics::default(),
            code_navigation_font_metrics: NavigationFontMetrics::default(),
            navigation_metrics_style_hash: 0,
            paragraph_spacing: 12.0,
            toc_max_depth: 3,
            markdown_first_line_indent: false,
            text_spacing_mode: ui::typography::TextSpacingMode::Natural,
            edit_ctx: None,
            edit_source: None,
            source_generation: 0,
            cursor_visible: true,
            standalone_preedit_cursor_advance: None,
            replacement_preedit: None,
        }
    }

    /// Mark the engine cache as dirty (source changed). Full rebuild.
    pub fn mark_source_dirty(&mut self) {
        self.cursor_affinity = CursorAffinity::Downstream;
        self.dirty = EngineDirty::SourceChanged;
        self.sel.clear();
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    /// Mark only the cursor position as changed — no full rebuild needed.
    pub(in crate::view) fn mark_cursor_moved(&mut self, new_byte: usize) {
        let old_byte = self.edit_ctx.as_ref().map(|ctx| ctx.cursor_byte);
        if old_byte == Some(new_byte) {
            return;
        }
        self.cursor_affinity = CursorAffinity::Downstream;
        let preedit_text = self.edit_ctx.as_ref().and_then(|ctx| ctx.preedit_text.clone());
        let preedit_cursor = self.edit_ctx.as_ref().and_then(|ctx| ctx.preedit_cursor);
        self.edit_ctx =
            Some(crate::edit::EditContext { cursor_byte: new_byte, preedit_text, preedit_cursor });
        self.update_replacement_preedit();
        if old_byte.is_none() {
            self.dirty = EngineDirty::SourceChanged;
        } else if !matches!(self.dirty, EngineDirty::SourceChanged) {
            self.dirty = EngineDirty::CursorMoved { old_byte, new_byte };
        }
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    /// 设置 IME preedit 文本。**严格保证不改动 `edit_ctx.cursor_byte`**（方案
    /// 2026-07-06 阶段 3a）：如果尚未收到 cursor，直接忽略 preedit——不能凭空
    /// 造出 cursor_byte=0 的假状态。
    pub(in crate::view) fn set_preedit_text(
        &mut self,
        text: String,
        cursor: Option<(usize, usize)>,
    ) {
        let Some(existing_ctx) = self.edit_ctx.as_ref() else {
            // 没有 cursor 时，preedit 无处附着；直接丢弃，避免制造 cursor_byte=0
            // 的伪状态被后续 visual_move / augment_edit 读到。
            return;
        };
        let cursor_byte = existing_ctx.cursor_byte;
        let preedit_text = if text.is_empty() { None } else { Some(text) };
        let preedit_cursor = preedit_text.as_ref().and(cursor);
        if existing_ctx.preedit_text == preedit_text
            && existing_ctx.preedit_cursor == preedit_cursor
        {
            self.update_replacement_preedit();
            return;
        }
        self.edit_ctx =
            Some(crate::edit::EditContext { cursor_byte, preedit_text, preedit_cursor });
        self.update_replacement_preedit();
        if !matches!(self.dirty, EngineDirty::SourceChanged) {
            self.dirty =
                EngineDirty::CursorMoved { old_byte: Some(cursor_byte), new_byte: cursor_byte };
        }
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    // ── Heading collection ──

    pub(in crate::view) fn collect_headings(&mut self) {
        self.headings.clear();
        let Some(ref lazy) = self.lazy else { return };
        for bi in 0..lazy.estimated_heights.len() {
            let doc_idx = match lazy.laid_to_doc(bi) {
                Some(i) => i,
                None => continue,
            };
            let kind = match lazy.source.blocks().get(doc_idx) {
                Some(b) => &b.kind,
                None => continue,
            };
            if let crate::builder::BlockKind::Heading { level } = kind {
                if *level > self.toc_max_depth {
                    continue;
                }
                // Get heading text from source blocks' headings or from materialized block.
                let text = if let Some(src_block) = lazy.source.blocks().get(doc_idx) {
                    src_block.text_lines.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                let y = lazy.estimated_positions[bi] + lazy.y_delta.get(bi).copied().unwrap_or(0.0);
                self.headings.push(HeadingEntry { text, level: *level, y_offset: y });
            }
        }
    }

    pub fn headings(&self) -> &[HeadingEntry] {
        &self.headings
    }

    pub fn current_heading_index(&self, scroll_y: f32) -> Option<usize> {
        if self.headings.is_empty() {
            return None;
        }
        match self.headings.binary_search_by(|h| {
            h.y_offset.partial_cmp(&scroll_y).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(i) => Some(i),
            Err(0) => Some(0),
            Err(i) => Some(i - 1),
        }
    }

    pub fn scroll_to_heading(&mut self, index: usize) {
        if let Some(h) = self.headings.get(index) {
            self.scroll_y = h.y_offset.clamp(0.0, self.content_height);
            self.pending_heading_jump = Some(index);
        }
    }

    // ── Layout rebuild ──

    pub(in crate::view) fn needs_rebuild(&self, style_hash: u64, viewport_w: f32) -> bool {
        matches!(self.dirty, EngineDirty::SourceChanged | EngineDirty::StyleChanged)
            || self.lazy.is_none()
            || style_hash != self.cached_style_hash
            || viewport_w != self.cached_viewport_w
    }

    pub(in crate::view) fn create_layout_with_view_state(
        &self,
        document: S,
        style: &MarkdownStyle,
        viewport_width: f32,
        document_view: &dyn DocView,
    ) -> LazyLayout<S> {
        let mut layout = LazyLayout::new(document, style, viewport_width, document_view);
        layout.code_viewport = self.lazy.as_ref().and_then(|previous| previous.code_viewport);
        layout.set_source_generation(self.source_generation);
        layout.set_shared_source(self.edit_source.as_deref().map(MarkdownSource::shared_text));
        layout.set_edit_ctx(self.edit_ctx.clone());
        layout.set_selection_range(self.byte_selection_range().map(|(start, end)| start..end));
        layout
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn rebuild_layout(
        &mut self,
        doc: S,
        style: &MarkdownStyle,
        viewport_w: f32,
        viewport_h: f32,
        shaper: Option<&mut shaping::Shaper>,
        highlighter: &AppCodeHighlighter<'_>,
        doc_view: &dyn textora_core::document::DocView,
        full_flat_lines: bool,
        full_layout: bool,
    ) {
        self.paragraph_spacing = style.paragraph_spacing;
        let mut lazy = self.create_layout_with_view_state(doc, style, viewport_w, doc_view);
        let previous_layout = (incremental_layout_reuse_enabled()
            && matches!(self.dirty, EngineDirty::SourceChanged)
            && style_hash_quick(style) == self.cached_style_hash
            && viewport_w == self.cached_viewport_w)
            .then(|| self.lazy.take())
            .flatten();
        if let Some(previous_layout) = previous_layout {
            lazy.reuse_unchanged_blocks_from(previous_layout);
        }
        if full_layout {
            // Editing mode: materialize all blocks with full shaping.
            lazy.ensure_all_blocks(style, viewport_w, shaper, Some(highlighter), doc_view);
        } else if full_flat_lines {
            // WYSIWYG editing needs whole-document flat line mappings for
            // selection/navigation, but only visible blocks need precise shaping.
            lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
            if let Some(s) = shaper {
                lazy.refresh_precise_range(
                    self.scroll_y,
                    viewport_h,
                    style,
                    s,
                    Some(highlighter),
                    doc_view,
                );
            }
        } else if let Some(s) = shaper {
            // Preview with shaper: viewport-driven lazy layout.
            lazy.ensure_visible(
                self.scroll_y,
                viewport_h,
                style,
                viewport_w,
                s,
                Some(highlighter),
                doc_view,
            );
        } else {
            // No shaper: materialize all blocks without text shaping.
            lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
        }
        lazy.build_flat_lines(doc_view);
        self.content_height = lazy.total_height;
        self.lazy = Some(lazy);
        self.dirty = EngineDirty::Clean;
        self.collect_headings();
        self.cached_dl = None;
        self.cached_vertices = None;
    }

    // ── Precision pass ──

    pub(in crate::view) fn precision_pass_on_scroll(
        &mut self,
        style: &MarkdownStyle,
        viewport_h: f32,
        shaper: Option<&mut shaping::Shaper>,
        highlighter: &AppCodeHighlighter<'_>,
        doc_view: &dyn textora_core::document::DocView,
    ) {
        let scroll_anchor = self.lazy.as_ref().and_then(|lazy| {
            if lazy.estimated_heights.is_empty() {
                return None;
            }
            let idx = lazy.block_at_y(self.scroll_y);
            let block_y =
                lazy.estimated_positions[idx] + lazy.y_delta.get(idx).copied().unwrap_or(0.0);
            Some(BlockAnchor { block_idx: idx, offset_in_block: self.scroll_y - block_y })
        });
        let had_deltas = if let Some(ref mut lazy) = self.lazy {
            if let Some(s) = shaper {
                let deltas = lazy.ensure_precise_range(
                    self.scroll_y,
                    viewport_h,
                    style,
                    s,
                    Some(highlighter),
                    doc_view,
                );
                if !deltas.is_empty() {
                    self.content_height = lazy.total_height;
                }
                !deltas.is_empty()
            } else {
                false
            }
        } else {
            false
        };
        if had_deltas {
            self.collect_headings();
            if let Some(idx) = self.pending_heading_jump.take() {
                if let Some(h) = self.headings.get(idx) {
                    self.scroll_y = h.y_offset.clamp(0.0, self.content_height);
                }
            } else if let Some(ref anchor) = scroll_anchor {
                self.restore_anchor(anchor);
            }
        }
    }

    pub(in crate::view) fn restore_anchor(&mut self, anchor: &BlockAnchor) {
        let Some(ref lazy) = self.lazy else {
            return;
        };
        if anchor.block_idx >= lazy.estimated_heights.len() {
            return;
        }
        let block_y = lazy.estimated_positions[anchor.block_idx]
            + lazy.y_delta.get(anchor.block_idx).copied().unwrap_or(0.0);
        self.scroll_y = (block_y + anchor.offset_in_block).clamp(0.0, self.content_height);
    }

    // ── Core render ──

    /// Core render method. `build_doc` produces a `S: BlockSource` from the given style.
    /// Returns (DrawList, needs_drain).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        theme: &Theme,
        viewport_w: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        style: &MarkdownStyle,
        build_doc: impl FnOnce(&MarkdownStyle) -> S,
        mut shaper: Option<&mut shaping::Shaper>,
        doc_view: &dyn textora_core::document::DocView,
        full_flat_lines: bool,
        full_layout: bool,
    ) -> (DrawList, bool) {
        let perf_total_started_at = Instant::now();
        let mut perf_build_doc_us = 0;
        let mut perf_rebuild_us = 0;
        let mut perf_cursor_us = 0;
        let mut render_path = "draw";
        self.rendered_body_font_size = style.body_font_size;
        self.rendered_line_height = style.line_height;
        self.cached_query_style = Some(style.clone());
        let style_hash = style_hash_quick(style);
        let highlighter = AppCodeHighlighter { theme };
        if style_hash != self.navigation_metrics_style_hash
            && let Some(active_shaper) = shaper.as_deref_mut()
        {
            self.body_navigation_font_metrics = NavigationFontMetrics::measure(
                active_shaper,
                style.body_font_size,
                style.body_font_family.first().map(String::as_str),
            );
            self.code_navigation_font_metrics = NavigationFontMetrics::measure(
                active_shaper,
                style.code_font_size,
                style.code_font_family.as_deref(),
            );
            self.navigation_metrics_style_hash = style_hash;
        }

        // If cursor moved but no shaper available, escalate to full rebuild
        // so that needs_rebuild / rebuild_layout (below) will handle it.
        if matches!(&self.dirty, EngineDirty::CursorMoved { .. }) && shaper.is_none() {
            self.dirty = EngineDirty::SourceChanged;
            self.cached_dl = None;
            self.cached_vertices = None;
        }

        if self.needs_rebuild(style_hash, viewport_w) {
            render_path = "rebuild";
            let build_doc_started_at = Instant::now();
            let doc = build_doc(style);
            perf_build_doc_us = build_doc_started_at.elapsed().as_micros();
            let rebuild_started_at = Instant::now();
            self.rebuild_layout(
                doc,
                style,
                viewport_w,
                viewport_h,
                shaper.as_deref_mut(),
                &highlighter,
                doc_view,
                full_flat_lines,
                full_layout,
            );
            perf_rebuild_us = rebuild_started_at.elapsed().as_micros();
            self.cached_style_hash = style_hash;
            self.cached_viewport_w = viewport_w;
        }

        if self.dirty == EngineDirty::QueryLayoutReady {
            let selection_range = self.byte_selection_range().map(|(start, end)| start..end);
            if let Some(lazy) = self.lazy.as_mut() {
                if let Some(active_shaper) = shaper.as_deref_mut() {
                    lazy.refresh_precise_range(
                        self.scroll_y,
                        viewport_h,
                        style,
                        active_shaper,
                        Some(&highlighter),
                        doc_view,
                    );
                }
                lazy.set_selection_range(selection_range);
                lazy.build_flat_lines(doc_view);
                self.content_height = lazy.total_height;
            }
            self.dirty = EngineDirty::Clean;
        }

        // Handle cursor-only changes: invalidate affected blocks without full rebuild.
        if let EngineDirty::CursorMoved { old_byte, new_byte } = self.dirty.clone() {
            render_path = "cursor";
            let cursor_started_at = Instant::now();
            let selection_range = self.byte_selection_range().map(|(start, end)| start..end);
            // shaper is guaranteed Some here (None case was promoted above).
            if let Some(ref mut s) = shaper {
                if let Some(lazy) = self.lazy.as_mut() {
                    lazy.set_edit_ctx(self.edit_ctx.clone());
                    lazy.set_selection_range(selection_range);
                    // 2026-07-06 阶段 4b：full_layout 分支不再对整个文档重 shape，
                    // 而是走"invalidate 视口内命中块 → ensure_all_blocks 结构性重排
                    // （不 shape）→ refresh_precise_range 只 shape 可见范围"。
                    // 与 full_flat_lines 分支共享同一路径，避免长文档下光标一次移动
                    // 就把全部块都 reshape 一遍。
                    if full_layout || full_flat_lines {
                        // 光标块无论是否在渲染窗口内都必须失效：屏外块保持
                        // precise 会让 active marker 永久残留（P7）。失效本身
                        // 只清标记，重排仍由 refresh_precise_range 限定在可见范围。
                        lazy.invalidate_lines_for_source_bytes(
                            old_byte.into_iter().chain(std::iter::once(new_byte)),
                        );
                        lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
                        lazy.refresh_precise_range(
                            self.scroll_y,
                            viewport_h,
                            style,
                            s,
                            Some(&highlighter),
                            doc_view,
                        );
                    } else {
                        lazy.invalidate_lines_for_source_bytes(
                            old_byte.into_iter().chain(std::iter::once(new_byte)),
                        );
                        lazy.ensure_visible(
                            self.scroll_y,
                            viewport_h,
                            style,
                            viewport_w,
                            s,
                            Some(&highlighter),
                            doc_view,
                        );
                    }
                    lazy.build_flat_lines(doc_view);
                    self.content_height = lazy.total_height;
                }
                self.dirty = EngineDirty::Clean;
            }
            perf_cursor_us = cursor_started_at.elapsed().as_micros();
        }

        if self.dirty == EngineDirty::SelectionChanged {
            render_path = "selection";
            let selection_range = self.byte_selection_range().map(|(start, end)| start..end);
            if let Some(lazy) = self.lazy.as_mut() {
                lazy.set_selection_range(selection_range);
                if full_layout || full_flat_lines {
                    lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
                    if let Some(active_shaper) = shaper.as_deref_mut() {
                        lazy.refresh_precise_range(
                            self.scroll_y,
                            viewport_h,
                            style,
                            active_shaper,
                            Some(&highlighter),
                            doc_view,
                        );
                    }
                } else if let Some(active_shaper) = shaper.as_deref_mut() {
                    lazy.ensure_visible(
                        self.scroll_y,
                        viewport_h,
                        style,
                        viewport_w,
                        active_shaper,
                        Some(&highlighter),
                        doc_view,
                    );
                }
                lazy.build_flat_lines(doc_view);
                self.content_height = lazy.total_height;
            }
            self.dirty = EngineDirty::Clean;
        }

        self.reveal_pending_cursor(viewport_h);
        let vp_key = (viewport_w, viewport_h);
        if let Some(ref cached) = self.cached_dl
            && self.cached_dl_scroll_y == self.scroll_y
            && self.cached_dl_viewport == vp_key
        {
            let elapsed_us = perf_total_started_at.elapsed().as_micros();
            if should_log_perf(elapsed_us) {
                eprintln!(
                    "[perf:md_engine] path=cache total={}us cmds={} vertices_cached={} viewport=({:.0}x{:.0}) scroll_y={:.1}",
                    elapsed_us,
                    cached.cmds.len(),
                    self.cached_vertices.is_some(),
                    viewport_w,
                    viewport_h,
                    self.scroll_y,
                );
            }
            return if self.cached_vertices.is_some() {
                (DrawList::new(), false)
            } else {
                (cached.clone(), true)
            };
        }

        let precision_started_at = Instant::now();
        self.precision_pass_on_scroll(
            style,
            viewport_h,
            shaper.as_deref_mut(),
            &highlighter,
            doc_view,
        );
        let perf_precision_us = precision_started_at.elapsed().as_micros();
        if self.reveal_pending_cursor(viewport_h) {
            self.pending_cursor_reveal = None;
        }

        let lazy = self.lazy.as_ref().expect("lazy layout must exist after precision pass");
        let mut dl = DrawList::new();
        let (visible, visible_yd) = lazy.materialized_blocks();
        let block_count = lazy.source.blocks().len();
        let flat_line_count = lazy.flat_lines.len();
        let visible_block_count = visible.blocks.len();
        let draw_started_at = Instant::now();
        crate::render::render_doc_with_offset_and_embedded(
            &visible,
            style,
            &mut dl,
            self.scroll_y,
            viewport_h,
            offset_x,
            offset_y,
            shaper,
            &visible_yd,
            Some(lazy.ascii_diagrams()),
            Some(lazy.embedded_images()),
        );
        let perf_draw_us = draw_started_at.elapsed().as_micros();
        let command_count = dl.cmds.len();
        self.cached_dl = Some(dl.clone());
        self.cached_dl_scroll_y = self.scroll_y;
        self.cached_dl_viewport = vp_key;
        self.cached_vertices = None;
        self.cached_offset_x = offset_x;
        self.cached_offset_y = offset_y;
        let elapsed_us = perf_total_started_at.elapsed().as_micros();
        if should_log_perf(elapsed_us) {
            eprintln!(
                "[perf:md_engine] path={} total={}us build_doc={}us rebuild={}us cursor={}us precision={}us draw={}us blocks={} visible_blocks={} flat_lines={} cmds={} viewport=({:.0}x{:.0}) scroll_y={:.1} full_flat={} full_layout={}",
                render_path,
                elapsed_us,
                perf_build_doc_us,
                perf_rebuild_us,
                perf_cursor_us,
                perf_precision_us,
                perf_draw_us,
                block_count,
                visible_block_count,
                flat_line_count,
                command_count,
                viewport_w,
                viewport_h,
                self.scroll_y,
                full_flat_lines,
                full_layout,
            );
        }
        (dl, true)
    }
}
