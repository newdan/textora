use super::*;

fn horizontal_whitespace_start_before(source: &str, byte: usize) -> usize {
    source
        .get(..byte)
        .and_then(|prefix| {
            prefix
                .char_indices()
                .rev()
                .find(|(_, character)| !matches!(character, ' ' | '\t'))
                .map(|(offset, character)| offset + character.len_utf8())
        })
        .unwrap_or(0)
}

fn trailing_ws_reaches_newline(source: &str, cursor_byte: usize) -> bool {
    let Some(rest) = source.get(cursor_byte..) else {
        return false;
    };
    let after_ws = rest.trim_start_matches([' ', '\t']);
    after_ws.starts_with('\n') || after_ws.starts_with("\r\n")
}

impl<S: BlockSource> PreviewEngine<S> {
    /// 查询：返回光标在插件渲染空间中的 (x, y, w, h)。
    /// y = fl.rect.y - scroll_y，与 render_line_with_offset 的投影一致。
    /// 调用方加 plugin_render_bounds().origin 即得窗口像素坐标。
    pub fn cursor_screen_pos(&self) -> Option<(f32, f32, f32, f32)> {
        if let Some(composition) = &self.replacement_preedit {
            return composition.engine.cursor_screen_pos();
        }
        let ctx = self.edit_ctx.as_ref()?;
        self.cursor_screen_pos_for_byte(ctx.cursor_byte)
    }

    pub(in crate::view) fn grapheme_x_for_line(
        &self,
        line: &crate::layout::FlatLine,
        grapheme_position: usize,
    ) -> f32 {
        if line.shaped.is_some() {
            return crate::layout::grapheme_x(line, grapheme_position);
        }
        self.navigation_font_metrics_for_line(line).grapheme_x(
            &line.text,
            grapheme_position,
            line.font_size,
        )
    }

    pub(in crate::view) fn grapheme_at_x_for_line(
        &self,
        line: &crate::layout::FlatLine,
        relative_x: f32,
    ) -> usize {
        if line.shaped.is_some() {
            return crate::layout::grapheme_at_x(line, relative_x);
        }
        self.navigation_font_metrics_for_line(line).grapheme_at_x(
            &line.text,
            relative_x,
            line.font_size,
        )
    }

    pub(in crate::view) fn navigation_font_metrics_for_line(
        &self,
        line: &crate::layout::FlatLine,
    ) -> &NavigationFontMetrics {
        if line.is_code {
            &self.code_navigation_font_metrics
        } else {
            &self.body_navigation_font_metrics
        }
    }

    pub(in crate::view) fn cursor_screen_pos_for_byte(
        &self,
        cursor_byte: usize,
    ) -> Option<(f32, f32, f32, f32)> {
        if let Some(composition) = &self.replacement_preedit {
            return composition
                .engine
                .cursor_screen_pos_for_byte(composition.preview_byte(cursor_byte));
        }
        let lazy = self.lazy.as_ref()?;

        if let Some(rect) = self.empty_source_line_cursor_screen_pos(cursor_byte) {
            return Some(rect);
        }

        let (visual_line_idx, visual_grapheme) = self
            .cursor_visual_position_for_byte(
                cursor_byte,
                self.cursor_affinity_for_byte(cursor_byte),
            )
            .map(|position| (position.flat_line_idx, position.grapheme_pos))?;
        let flat_idx = lazy.flat_line_idx_for_projection(visual_line_idx)?;
        let fl = lazy.flat_lines.get(flat_idx)?;
        let x = fl.rect.x
            + crate::layout::grapheme_x(fl, visual_grapheme)
            + self.trailing_stripped_space_advance(flat_idx, cursor_byte);
        let cursor_height = fl.font_size.min(fl.rect.h);
        let text_baseline_y = fl.rect.y + fl.font_size - self.scroll_y;
        let cursor_y = text_baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;
        Some((x, cursor_y, 2.0, cursor_height))
    }

    #[cfg(test)]
    pub(crate) fn projection_index(&self) -> &SourceProjectionIndex {
        self.lazy
            .as_ref()
            .and_then(|lazy| lazy.source_projection_index.as_ref())
            .expect("rendered WYSIWYG test view must publish a source projection index")
    }

    pub(crate) fn cursor_visual_position_for_byte(
        &self,
        source_byte: usize,
        affinity: CursorAffinity,
    ) -> Option<VisualPosition> {
        self.lazy
            .as_ref()?
            .source_projection_index
            .as_ref()?
            .visual_position_for_source(source_byte, affinity)
    }

    /// Extra x advance when the cursor sits past bytes that were present in the
    /// source but stripped by the markdown parser (e.g. trailing whitespace after
    /// a paragraph). Without this compensation, typing a trailing space visually
    /// stops the cursor at the end of the last shaped grapheme.
    pub(in crate::view) fn trailing_stripped_space_advance(
        &self,
        flat_idx: usize,
        cursor_byte: usize,
    ) -> f32 {
        let Some(lazy) = self.lazy.as_ref() else { return 0.0 };
        let Some(fl) = lazy.flat_lines.get(flat_idx) else { return 0.0 };
        let Some(projection) = fl.source_projection.as_ref() else { return 0.0 };
        let Some(sentinel) = projection.boundaries.last().map(|anchor| anchor.byte) else {
            return 0.0;
        };
        let Some(source) = self.edit_source.as_deref() else { return 0.0 };

        let missing_space_count = if cursor_byte >= sentinel {
            self.terminal_stripped_space_count(
                source,
                fl,
                projection.flat_line_idx,
                sentinel,
                cursor_byte,
            )
        } else {
            self.internal_soft_break_stripped_space_count(
                source,
                projection.flat_line_idx,
                cursor_byte,
            )
        };
        if missing_space_count == 0 {
            return 0.0;
        }
        // Prefer the shaped space advance from the flat_line if a trailing space
        // survived shaping; otherwise fall back to the same estimate the wrap
        // pipeline uses.
        let per_space = fl
            .shaped
            .as_ref()
            .and_then(|shaped| {
                shaped.clusters.iter().rev().find_map(|c| {
                    let bytes = fl.text.as_bytes().get(c.byte_range.clone())?;
                    (bytes == b" ").then_some(c.advance)
                })
            })
            .unwrap_or(fl.font_size * 0.3);
        missing_space_count as f32 * per_space
    }

    fn terminal_stripped_space_count(
        &self,
        source: &str,
        fl: &crate::layout::FlatLine,
        expected_proj_idx: usize,
        sentinel: usize,
        cursor_byte: usize,
    ) -> usize {
        let ws_start = horizontal_whitespace_start_before(source, cursor_byte);
        let effective_start =
            if cursor_byte == sentinel { ws_start } else { ws_start.max(sentinel) };
        let Some(stripped) = source.get(effective_start..cursor_byte) else {
            return 0;
        };
        // Only whitespace gets stripped by pulldown-cmark from paragraph text;
        // bail out for any other content so we don't misplace the cursor.
        if stripped.is_empty()
            || !stripped.chars().all(|c| c == ' ' || c == '\t')
            || (cursor_byte == sentinel && fl.text.ends_with(stripped))
        {
            return 0;
        }
        if effective_start > sentinel {
            return self.unadvanced_space_count_on_line(
                stripped,
                effective_start,
                expected_proj_idx,
                cursor_byte,
            );
        }
        stripped.chars().count()
    }

    fn internal_soft_break_stripped_space_count(
        &self,
        source: &str,
        expected_proj_idx: usize,
        cursor_byte: usize,
    ) -> usize {
        let before_ws_start = horizontal_whitespace_start_before(source, cursor_byte);
        let Some(ws_before) = source.get(before_ws_start..cursor_byte) else {
            return 0;
        };
        if ws_before.is_empty() || !trailing_ws_reaches_newline(source, cursor_byte) {
            return 0;
        }
        self.unadvanced_space_count_on_line(
            ws_before,
            before_ws_start,
            expected_proj_idx,
            cursor_byte,
        )
    }

    fn unadvanced_space_count_on_line(
        &self,
        ws_before: &str,
        before_ws_start: usize,
        expected_proj_idx: usize,
        cursor_byte: usize,
    ) -> usize {
        let Some(before_pos) = self.cursor_visual_position_for_byte(
            before_ws_start,
            self.cursor_affinity_for_byte(before_ws_start),
        ) else {
            return 0;
        };
        let Some(cursor_pos) = self.cursor_visual_position_for_byte(
            cursor_byte,
            self.cursor_affinity_for_byte(cursor_byte),
        ) else {
            return 0;
        };
        if cursor_pos.flat_line_idx != expected_proj_idx
            || before_pos.flat_line_idx != expected_proj_idx
        {
            return 0;
        }
        let advanced_graphemes = cursor_pos.grapheme_pos.saturating_sub(before_pos.grapheme_pos);
        ws_before.chars().count().saturating_sub(advanced_graphemes)
    }

    pub(in crate::view) fn visual_cursor_screen_pos(&self) -> Option<(f32, f32, f32, f32)> {
        if let Some(composition) = &self.replacement_preedit {
            return composition.engine.cursor_screen_pos();
        }
        let ctx = self.edit_ctx.as_ref()?;
        let preedit_text = ctx.preedit_text.as_deref().filter(|text| !text.is_empty())?;
        let lazy = self.lazy.as_ref()?;
        let preedit_cursor_grapheme =
            preedit_cursor_grapheme_index(preedit_text, ctx.preedit_cursor);
        let virtual_position = lazy
            .source_projection_index
            .as_ref()?
            .virtual_position_for_source(ctx.cursor_byte, preedit_cursor_grapheme);
        let Some(virtual_position) = virtual_position else {
            let (x, y, width, height) = self.cursor_screen_pos_for_byte(ctx.cursor_byte)?;
            return Some((
                x + self.standalone_preedit_cursor_advance.unwrap_or_default(),
                y,
                width,
                height,
            ));
        };
        let flat_idx = lazy.flat_line_idx_for_projection(virtual_position.flat_line_idx)?;
        let fl = lazy.flat_lines.get(flat_idx)?;
        let x = fl.rect.x
            + crate::layout::grapheme_x(fl, virtual_position.grapheme_pos)
            + self.trailing_stripped_space_advance(flat_idx, ctx.cursor_byte);
        let cursor_height = fl.font_size.min(fl.rect.h);
        let text_baseline_y = fl.rect.y + fl.font_size - self.scroll_y;
        let cursor_y = text_baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;
        Some((x, cursor_y, 2.0, cursor_height))
    }

    pub(in crate::view) fn standalone_preedit_render_data(
        &self,
    ) -> Option<StandalonePreeditRenderData<'_>> {
        let ctx = self.edit_ctx.as_ref()?;
        let preedit_text = ctx.preedit_text.as_deref().filter(|text| !text.is_empty())?;
        let lazy = self.lazy.as_ref()?;
        let source = self.edit_source.as_deref()?;
        let source_line = source_line_at_byte(source, ctx.cursor_byte)?;
        if self.empty_source_line_role(source_line, lazy, source)
            == EmptySourceLineRole::HiddenBlockSeparator
        {
            return None;
        }
        let preedit_cursor_grapheme =
            preedit_cursor_grapheme_index(preedit_text, ctx.preedit_cursor);
        if lazy
            .source_projection_index
            .as_ref()?
            .virtual_position_for_source(ctx.cursor_byte, preedit_cursor_grapheme)
            .is_some()
        {
            return None;
        }
        let (x, cursor_y, _, cursor_height) = self.cursor_screen_pos_for_byte(ctx.cursor_byte)?;
        Some(StandalonePreeditRenderData {
            text: preedit_text,
            cursor: ctx.preedit_cursor,
            x,
            baseline_y: cursor_y + cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO,
            font_size: cursor_height,
        })
    }

    pub(in crate::view) fn set_standalone_preedit_cursor_advance(&mut self, advance: f32) {
        self.standalone_preedit_cursor_advance = Some(advance);
    }

    pub(in crate::view) fn empty_source_line_cursor_screen_pos(
        &self,
        cursor_byte: usize,
    ) -> Option<(f32, f32, f32, f32)> {
        let source = self.edit_source.as_ref()?;
        let source_line = source_line_at_byte(source, cursor_byte)?;
        if !source_line.is_empty() {
            return None;
        }

        let lazy = self.lazy.as_ref()?;
        if self.empty_source_line_role(source_line, lazy, source)
            == EmptySourceLineRole::HiddenBlockSeparator
        {
            return None;
        }

        let line = self.own_rendered_line(source_line, lazy)?;
        let source_start = line.source_projection.as_ref()?.boundaries.first()?.byte;
        let whitespace_before_cursor = source.get(source_start..cursor_byte)?;
        let whitespace_graphemes = crate::grapheme_map::grapheme_count(whitespace_before_cursor);
        let whitespace_advance = if line.text.starts_with(whitespace_before_cursor) {
            self.grapheme_x_for_line(line, whitespace_graphemes)
        } else {
            self.navigation_font_metrics_for_line(line).grapheme_x(
                whitespace_before_cursor,
                whitespace_graphemes,
                line.font_size,
            )
        };
        let x = line.rect.x + whitespace_advance;
        let cursor_height = line.font_size.min(line.rect.h);
        let baseline_y = line.rect.y + cursor_height;
        let cursor_y = baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO - self.scroll_y;
        Some((x, cursor_y, 2.0, cursor_height))
    }

    #[cfg(test)]
    pub(in crate::view) fn empty_source_line_metrics(
        &self,
        source_line: SourceLineAtByte,
        lazy: &LazyLayout<S>,
        _source: &str,
    ) -> Option<(f32, f32, f32, f32)> {
        let line = self.own_rendered_line(source_line, lazy)?;
        Some((line.rect.x, line.rect.y, line.font_size, line.rect.h))
    }

    /// 空源码行自身的渲染行（代码块/metadata 块会为内部空行生成投影）。
    /// 判定依据：投影的源码范围完整落在该空行的源码区间内。
    pub(in crate::view) fn own_rendered_line<'a>(
        &self,
        source_line: SourceLineAtByte,
        lazy: &'a LazyLayout<S>,
    ) -> Option<&'a crate::layout::FlatLine> {
        lazy.flat_lines.iter().find(|flat_line| {
            flat_line.source_projection.as_ref().is_some_and(|projection| {
                projection.source_extent.start >= source_line.start
                    && projection.source_extent.end <= source_line.end
            })
        })
    }
}
