use super::*;

// ===== NovelView =====

/// Novel reading view — wraps PreviewEngine with txt→NovelStructure conversion.
/// Uses `MarkdownStyle::novel()` for independent theme section.
pub struct NovelView {
    pub(in crate::view) engine: PreviewEngine<crate::builder::NovelStructure>,
}

impl Default for NovelView {
    fn default() -> Self {
        Self::new()
    }
}

impl NovelView {
    pub fn new() -> Self {
        Self { engine: PreviewEngine::new() }
    }

    pub(in crate::view) fn find_chapter_heading<S: BlockSource>(
        engine: &PreviewEngine<S>,
        forward: bool,
    ) -> Option<usize> {
        let cur = engine.current_heading_index(engine.scroll_y).unwrap_or(0);
        let headings = engine.headings();
        if headings.is_empty() {
            return None;
        }
        if forward {
            if cur + 1 < headings.len() { Some(cur + 1) } else { None }
        } else if cur > 0 {
            Some(cur - 1)
        } else if engine.scroll_y > 0.0 {
            Some(0)
        } else {
            None
        }
    }
}

impl ViewPlugin for NovelView {
    fn name(&self) -> &str {
        "novel_view"
    }
    fn allows_editing(&self) -> bool {
        false
    }
    fn shows_cursor(&self) -> bool {
        false
    }
    fn shows_gutter(&self) -> bool {
        false
    }

    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let font_size = self.engine.base_font_size * dpi_scale;
        let line_height = self.engine.base_line_height * dpi_scale;
        let mut style = MarkdownStyle::novel_at_dpi(theme, font_size, line_height, dpi_scale);
        style.text_spacing_mode = self.engine.text_spacing_mode;
        self.engine.toc_max_depth = 3;

        let (dl, _) = self.engine.render(
            theme,
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y,
            &style,
            |_| crate::builder::NovelStructure::scan(doc),
            Some(shaper),
            doc,
            false,
            false, // preview: viewport-driven layout
        );
        dl
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn textora_core::document::DocViewMut,
    ) -> bool {
        match msg {
            PluginMessage::SetTextSpacingMode(_) => {
                self.engine.handle_message_common(&msg).unwrap_or(false)
            }
            PluginMessage::Scroll { delta, viewport_h } => self.engine.scroll(delta, viewport_h),
            PluginMessage::ScrollToHeading(index) => {
                self.engine.scroll_to_heading(index);
                true
            }
            PluginMessage::ScrollToNextChapter => {
                if let Some(idx) = Self::find_chapter_heading(&self.engine, true) {
                    self.engine.scroll_to_heading(idx);
                }
                true
            }
            PluginMessage::ScrollToPrevChapter => {
                if let Some(idx) = Self::find_chapter_heading(&self.engine, false) {
                    self.engine.scroll_to_heading(idx);
                } else {
                    self.engine.scroll_y = 0.0;
                }
                true
            }
            PluginMessage::UpdateSource { .. } => {
                self.engine.mark_source_dirty();
                true
            }
            PluginMessage::SetRenderSettings { font_size, line_height, .. } => {
                self.engine.base_font_size = font_size;
                self.engine.base_line_height = line_height;
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        match q {
            PluginQuery::ScrollY => PluginResponse::Float(self.engine.scroll_y),
            PluginQuery::ContentHeight => PluginResponse::Float(self.engine.content_height),
            PluginQuery::TOCHeadings => PluginResponse::Headings(
                self.engine
                    .headings()
                    .iter()
                    .map(|h| ui::plugin::HeadingEntry {
                        title: h.text.clone(),
                        y: h.y_offset,
                        level: h.level,
                    })
                    .collect(),
            ),
            PluginQuery::CurrentHeadingIndex(scroll_y) => PluginResponse::Position(
                self.engine.current_heading_index(scroll_y).map(|i| (i, 0)),
            ),
            PluginQuery::HasSelection => PluginResponse::Bool(self.engine.has_selection()),
            PluginQuery::SelectedText => {
                PluginResponse::String(self.engine.selected_text().unwrap_or_default())
            }
            PluginQuery::SelCursor => PluginResponse::Position(
                self.engine.sel.cursor.map(|p| (p.flat_line_idx, p.grapheme_pos)),
            ),
            PluginQuery::SelectionRange => PluginResponse::PositionPair(None),
            PluginQuery::HitTest { x, y, offset_x, offset_y } => PluginResponse::Position(
                self.engine
                    .hit_test(x, y, offset_x, offset_y)
                    .map(|p| (p.flat_line_idx, p.grapheme_pos)),
            ),
            PluginQuery::FlatLines => PluginResponse::FlatLines(
                self.engine
                    .flat_lines()
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
                    .collect(),
            ),
            _ => PluginResponse::None,
        }
    }
}

// ===== NovelViewFactory =====

pub struct NovelViewFactory;

impl PluginFactory for NovelViewFactory {
    fn name(&self) -> &str {
        "novel_view"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.extension()).is_some_and(|e| e == "txt")
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(NovelView::new())
    }
}
