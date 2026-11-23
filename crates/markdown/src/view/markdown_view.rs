use super::*;

// ===== MarkdownView =====

/// Markdown preview view — wraps PreviewEngine with markdown-specific doc building.
pub struct MarkdownView {
    pub(in crate::view) engine: PreviewEngine,
    pub(in crate::view) source: SourceSnapshot,
    pub(in crate::view) cached_generation: Option<u32>,
}

impl Default for MarkdownView {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownView {
    pub fn new() -> Self {
        Self {
            engine: PreviewEngine::new(),
            source: MarkdownSource::snapshot(String::new()),
            cached_generation: None,
        }
    }

    pub fn set_source(&mut self, text: String, generation: u32) {
        if self.source.as_str() != text {
            self.source = MarkdownSource::snapshot(text);
            self.engine.mark_source_dirty();
        }
        self.engine.set_source_generation(generation);
        self.cached_generation = Some(generation);
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        self.cached_generation != Some(generation)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        theme: &Theme,
        viewport_w: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        settings: MarkdownRenderSettings<'_>,
        shaper: Option<&mut shaping::Shaper>,
    ) -> (DrawList, bool) {
        self.render_at_dpi(theme, viewport_w, viewport_h, offset_x, offset_y, settings, 1.0, shaper)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn render_at_dpi(
        &mut self,
        theme: &Theme,
        viewport_w: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        settings: MarkdownRenderSettings<'_>,
        dpi_scale: f32,
        shaper: Option<&mut shaping::Shaper>,
    ) -> (DrawList, bool) {
        let style = settings.style_at_dpi(theme, dpi_scale);
        self.engine.toc_max_depth = settings.toc_max_depth;
        let source = &self.source;
        let editing = self.engine.edit_source.is_some();
        let engine = &mut self.engine;
        let string_doc = textora_core::document::StringDocView::new(source);
        engine.render(
            theme,
            viewport_w,
            viewport_h,
            offset_x,
            offset_y,
            &style,
            |_| {
                let document = source.layout_document();
                if editing { document.with_editable_paragraphs(source) } else { document }
            },
            shaper,
            &string_doc,
            true,  // preview still needs whole-document flat lines for selection/copy.
            false, // precise shaping/highlighting stays viewport-driven for responsiveness.
        )
    }

    pub fn engine(&self) -> &PreviewEngine {
        &self.engine
    }
    pub fn engine_mut(&mut self) -> &mut PreviewEngine {
        &mut self.engine
    }
    pub fn scroll_y(&self) -> f32 {
        self.engine.scroll_y
    }
    pub fn headings(&self) -> &[HeadingEntry] {
        self.engine.headings()
    }
}

impl ViewPlugin for MarkdownView {
    fn name(&self) -> &str {
        "markdown_view"
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
        _doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let font_family = self.engine.base_font_family.clone();
        let settings = MarkdownRenderSettings {
            font_family: &font_family,
            font_size: self.engine.base_font_size * dpi_scale,
            line_height: self.engine.base_line_height * dpi_scale,
            toc_max_depth: self.engine.toc_max_depth,
            markdown_first_line_indent: self.engine.markdown_first_line_indent,
            text_spacing_mode: self.engine.text_spacing_mode,
        };
        let (dl, _) = self.render_at_dpi(
            theme,
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y,
            settings,
            dpi_scale,
            Some(shaper),
        );
        dl
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn textora_core::document::DocViewMut,
    ) -> bool {
        if let Some(result) = self.engine.handle_message_common(&msg) {
            return result;
        }
        match msg {
            PluginMessage::ScrollToSearchMatch { query, match_case, active_idx } => {
                self.engine.scroll_to_search_match(&query, match_case, active_idx);
                true
            }
            PluginMessage::UpdateSource { text, generation } => {
                self.set_source(text, generation);
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        if matches!(q, PluginQuery::SelectedText) {
            return PluginResponse::String(
                self.engine.selected_reading_text(&self.source).unwrap_or_default(),
            );
        }
        if let Some(resp) = self.engine.query_common(&q) {
            return resp;
        }
        match q {
            PluginQuery::SourceSpacingProtectedRanges { source_generation }
                if !self.needs_source_update(source_generation) =>
            {
                PluginResponse::SourceSpacingProtectedRanges(
                    self.source.source_spacing_protected_ranges(),
                )
            }
            PluginQuery::NeedsSourceUpdate(gen_id) => {
                PluginResponse::Bool(self.needs_source_update(gen_id))
            }
            PluginQuery::SearchHighlights {
                query,
                match_case,
                use_regex,
                active_idx,
                match_color,
                inactive_color,
            } => PluginResponse::DrawList(self.engine.search_highlights(
                &query,
                match_case,
                use_regex,
                active_idx,
                match_color,
                inactive_color,
            )),
            _ => PluginResponse::None,
        }
    }
}

// ===== MarkdownViewFactory =====

pub struct MarkdownViewFactory;

impl PluginFactory for MarkdownViewFactory {
    fn name(&self) -> &str {
        "markdown_view"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        is_plain_markdown_path(path)
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownView::new())
    }
}
