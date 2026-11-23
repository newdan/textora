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

    fn mode(&self) -> ui::plugin::PluginMode {
        ui::plugin::PluginMode::Reading
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

    fn sync_document(
        &mut self,
        generation: u32,
        reason: ui::plugin::SyncReason,
        load_source: &mut dyn FnMut() -> String,
    ) -> ui::plugin::SyncOutcome {
        if reason == ui::plugin::SyncReason::Normal && !self.needs_source_update(generation) {
            return ui::plugin::SyncOutcome::Unchanged;
        }
        self.set_source(load_source(), generation);
        ui::plugin::SyncOutcome::Updated
    }
}

impl ui::plugin::ViewPluginCapabilities for MarkdownView {
    fn scrollable(&self) -> Option<&dyn ui::plugin::ScrollableView> {
        Some(&self.engine)
    }

    fn scrollable_mut(&mut self) -> Option<&mut dyn ui::plugin::ScrollableView> {
        Some(&mut self.engine)
    }

    fn outline(&self) -> Option<&dyn ui::plugin::OutlineProvider> {
        Some(&self.engine)
    }

    fn outline_mut(&mut self) -> Option<&mut dyn ui::plugin::OutlineProvider> {
        Some(&mut self.engine)
    }

    fn search(&self) -> Option<&dyn ui::plugin::SearchView> {
        Some(self)
    }

    fn search_mut(&mut self) -> Option<&mut dyn ui::plugin::SearchView> {
        Some(self)
    }

    fn selection(&self) -> Option<&dyn ui::plugin::SelectionView> {
        Some(self)
    }

    fn selection_mut(&mut self) -> Option<&mut dyn ui::plugin::SelectionView> {
        Some(self)
    }
    fn allows_editing(&self) -> bool {
        false
    }

    fn set_view_render_settings(&mut self, settings: ui::plugin::ViewRenderSettings) {
        self.engine.set_view_render_settings(settings);
    }

    fn set_view_text_spacing_mode(&mut self, mode: ui::typography::TextSpacingMode) {
        self.engine.set_view_text_spacing_mode(mode);
    }
    fn source_spacing_protected_ranges(
        &self,
        source_generation: u32,
    ) -> Option<std::sync::Arc<[std::ops::Range<usize>]>> {
        (!self.needs_source_update(source_generation))
            .then(|| self.source.source_spacing_protected_ranges())
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
    fn create(&self) -> Box<dyn ViewPluginCapabilities> {
        Box::new(MarkdownView::new())
    }
}
