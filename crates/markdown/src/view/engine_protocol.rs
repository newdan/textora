use super::*;

impl<S: BlockSource> BlockViewEngine<S> {
    /// 编辑干预：Enter/Backspace/Tab/InsertText 的 markdown 感知行为。委派到
    /// [`crate::augmenter`]（2026-07-06 方案阶段 2b/2c 抽出）。
    pub fn augment_edit(
        &self,
        current_byte: usize,
        kind: crate::augmenter::AugmentKind,
    ) -> Option<crate::augmenter::EditAugmentation> {
        let source = self.edit_source.as_deref()?;
        crate::augmenter::augment_source(source, current_byte, kind)
    }

    /// 判断光标是否在某个 span 内。
    pub fn cursor_in_span(&self, span: &crate::builder::StyleSpan) -> bool {
        let Some(ctx) = self.edit_ctx.as_ref() else {
            return false;
        };
        crate::edit::cursor_in_span(span, ctx.cursor_byte)
    }

    pub(in crate::view) fn set_view_render_settings(
        &mut self,
        settings: ui::plugin::ViewRenderSettings,
    ) {
        if self.base_font_family != settings.font_family {
            self.base_font_family = settings.font_family;
            if !matches!(self.dirty, EngineDirty::SourceChanged) {
                self.dirty = EngineDirty::StyleChanged;
            }
            self.cached_dl = None;
            self.cached_vertices = None;
        }
        self.base_font_size = settings.font_size;
        self.base_line_height = settings.line_height;
        self.toc_max_depth = settings.toc_max_depth;
        self.markdown_first_line_indent = settings.markdown_first_line_indent;
    }

    pub(in crate::view) fn set_view_text_spacing_mode(
        &mut self,
        mode: ui::typography::TextSpacingMode,
    ) {
        if self.text_spacing_mode == mode {
            return;
        }
        self.text_spacing_mode = mode;
        self.dirty = EngineDirty::StyleChanged;
        self.cached_dl = None;
        self.cached_vertices = None;
    }
}

impl<S: BlockSource> ui::plugin::OutlineProvider for BlockViewEngine<S> {
    fn headings(&self) -> Vec<ui::plugin::HeadingEntry> {
        self.headings()
            .iter()
            .map(|heading| ui::plugin::HeadingEntry {
                title: heading.text.clone(),
                y: heading.y_offset,
                level: heading.level,
            })
            .collect()
    }

    fn current_heading_index(&self, scroll_y: f32) -> Option<usize> {
        BlockViewEngine::current_heading_index(self, scroll_y)
    }

    fn scroll_to_heading(&mut self, index: usize) {
        BlockViewEngine::scroll_to_heading(self, index);
    }
}

impl<S: BlockSource> ui::plugin::ScrollableView for BlockViewEngine<S> {
    fn scroll_y(&self, _document: &dyn DocView) -> f32 {
        self.scroll_y
    }

    fn content_height(&self) -> f32 {
        self.content_height
    }

    fn scroll(&mut self, delta: f32, viewport_height: f32) -> bool {
        BlockViewEngine::scroll(self, delta, viewport_height)
    }
}

fn reading_position(position: ViewPos) -> ui::plugin::ReadingPosition {
    let (line_index, grapheme_index) = position.indices();
    ui::plugin::ReadingPosition::new(line_index, grapheme_index)
}

impl<S: BlockSource> BlockViewEngine<S> {
    fn reading_flat_lines(&self) -> Vec<ui::plugin::FlatLine> {
        self.flat_lines()
            .iter()
            .map(|line| {
                let grapheme_count = crate::grapheme_map::grapheme_count(&line.text);
                ui::plugin::FlatLine {
                    text: line.text.clone(),
                    grapheme_count,
                    rect: line.rect,
                    grapheme_x: (0..=grapheme_count)
                        .map(|grapheme| crate::layout::grapheme_x(line, grapheme))
                        .collect(),
                }
            })
            .collect()
    }
}

macro_rules! markdown_selection_view {
    ($view:ty) => {
        impl ui::plugin::SelectionView for $view {
            fn has_selection(&self) -> bool {
                self.engine.has_selection()
            }

            fn selected_text(&self) -> Option<String> {
                self.engine.selected_reading_text(&self.source)
            }

            fn selection_cursor(&self) -> Option<ui::plugin::ReadingPosition> {
                self.engine.sel.cursor.map(reading_position)
            }

            fn selection_source_range(&self) -> Option<std::ops::Range<usize>> {
                self.engine.selection_source_range().map(|(start, end)| start..end)
            }

            fn hit_test_position(
                &self,
                x: f32,
                y: f32,
                offset_x: f32,
                offset_y: f32,
            ) -> Option<ui::plugin::ReadingPosition> {
                self.engine.hit_test(x, y, offset_x, offset_y).map(reading_position)
            }

            fn word_range(
                &self,
                position: ui::plugin::ReadingPosition,
            ) -> Option<std::ops::Range<ui::plugin::ReadingPosition>> {
                let (start, end) = self.engine.word_at_pos(ViewPos::from_indices(
                    position.line_index,
                    position.grapheme_index,
                ));
                Some(reading_position(start)..reading_position(end))
            }

            fn line_range(
                &self,
                position: ui::plugin::ReadingPosition,
            ) -> Option<std::ops::Range<ui::plugin::ReadingPosition>> {
                let (start, end) = self.engine.line_range_at_pos(ViewPos::from_indices(
                    position.line_index,
                    position.grapheme_index,
                ));
                Some(reading_position(start)..reading_position(end))
            }

            fn flat_lines(&self) -> Vec<ui::plugin::FlatLine> {
                self.engine.reading_flat_lines()
            }

            fn selection_highlights(&self, color: [f32; 4]) -> DrawList {
                self.engine.selection_highlights(color)
            }

            fn set_selection_cursor(
                &mut self,
                position: Option<ui::plugin::ReadingPosition>,
                _document: &dyn DocView,
            ) -> bool {
                self.engine.set_sel_cursor(position.map(ui::plugin::ReadingPosition::indices));
                true
            }

            fn set_selection_anchor(
                &mut self,
                position: Option<ui::plugin::ReadingPosition>,
                _document: &dyn DocView,
            ) -> bool {
                self.engine.set_sel_anchor(position.map(ui::plugin::ReadingPosition::indices));
                true
            }

            fn set_selection_cursor_byte(&mut self, byte: Option<usize>) -> bool {
                self.engine.set_sel_cursor_byte(byte);
                true
            }

            fn set_selection_anchor_byte(&mut self, byte: Option<usize>) -> bool {
                self.engine.set_sel_anchor_byte(byte);
                true
            }

            fn clear_selection(&mut self) -> bool {
                self.engine.clear_selection();
                true
            }

            fn select_all(&mut self) -> bool {
                self.engine.select_all();
                true
            }
        }
    };
}

markdown_selection_view!(MarkdownEditorView);
markdown_selection_view!(MarkdownView);

impl ui::plugin::SelectionView for NovelView {
    fn has_selection(&self) -> bool {
        self.engine.has_selection()
    }

    fn selected_text(&self) -> Option<String> {
        self.engine.selected_text()
    }

    fn selection_cursor(&self) -> Option<ui::plugin::ReadingPosition> {
        self.engine.sel.cursor.map(reading_position)
    }

    fn hit_test_position(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<ui::plugin::ReadingPosition> {
        self.engine.hit_test(x, y, offset_x, offset_y).map(reading_position)
    }

    fn flat_lines(&self) -> Vec<ui::plugin::FlatLine> {
        self.engine.reading_flat_lines()
    }
}
