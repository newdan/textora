use super::*;

// ===== NovelView =====

/// Novel reading view — wraps BlockViewEngine with txt→NovelStructure conversion.
/// Uses `MarkdownStyle::novel()` for independent theme section.
pub struct NovelView {
    pub(in crate::view) engine: BlockViewEngine<crate::builder::NovelStructure>,
}

impl Default for NovelView {
    fn default() -> Self {
        Self::new()
    }
}

impl NovelView {
    pub fn new() -> Self {
        Self { engine: BlockViewEngine::new() }
    }

    pub(in crate::view) fn find_chapter_heading<S: BlockSource>(
        engine: &BlockViewEngine<S>,
        direction: ui::plugin::ChapterDirection,
    ) -> Option<usize> {
        let current = engine.current_heading_index(engine.scroll_y)?;
        match direction {
            ui::plugin::ChapterDirection::Next => {
                let next = current + 1;
                (next < engine.headings().len()).then_some(next)
            }
            ui::plugin::ChapterDirection::Previous => current.checked_sub(1),
        }
    }
}

impl ViewPlugin for NovelView {
    fn name(&self) -> &str {
        "novel_view"
    }

    fn mode(&self) -> ui::plugin::PluginMode {
        ui::plugin::PluginMode::Reading
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

    fn sync_document(
        &mut self,
        _generation: u32,
        reason: ui::plugin::SyncReason,
        _load_source: &mut dyn FnMut() -> String,
    ) -> ui::plugin::SyncOutcome {
        if reason == ui::plugin::SyncReason::Normal {
            return ui::plugin::SyncOutcome::Unchanged;
        }
        self.engine.mark_source_dirty();
        ui::plugin::SyncOutcome::Updated
    }
}

impl ui::plugin::ViewPluginCapabilities for NovelView {
    fn scrollable(&self) -> Option<&dyn ui::plugin::ScrollableView> {
        Some(&self.engine)
    }

    fn scrollable_mut(&mut self) -> Option<&mut dyn ui::plugin::ScrollableView> {
        Some(&mut self.engine)
    }

    fn outline(&self) -> Option<&dyn ui::plugin::OutlineProvider> {
        Some(self)
    }

    fn outline_mut(&mut self) -> Option<&mut dyn ui::plugin::OutlineProvider> {
        Some(self)
    }

    fn selection(&self) -> Option<&dyn ui::plugin::SelectionView> {
        Some(self)
    }
    fn allows_editing(&self) -> bool {
        false
    }

    fn set_view_render_settings(&mut self, settings: ui::plugin::ViewRenderSettings) {
        self.engine.base_font_size = settings.font_size;
        self.engine.base_line_height = settings.line_height;
    }

    fn set_view_text_spacing_mode(&mut self, mode: ui::typography::TextSpacingMode) {
        self.engine.set_view_text_spacing_mode(mode);
    }
}

// ===== NovelViewFactory =====

impl ui::plugin::OutlineProvider for NovelView {
    fn headings(&self) -> Vec<ui::plugin::HeadingEntry> {
        ui::plugin::OutlineProvider::headings(&self.engine)
    }

    fn current_heading_index(&self, scroll_y: f32) -> Option<usize> {
        self.engine.current_heading_index(scroll_y)
    }

    fn scroll_to_heading(&mut self, index: usize) {
        self.engine.scroll_to_heading(index);
    }

    fn scroll_to_chapter(&mut self, direction: ui::plugin::ChapterDirection) {
        use ui::plugin::ChapterDirection;

        if let Some(index) = Self::find_chapter_heading(&self.engine, direction) {
            self.engine.scroll_to_heading(index);
        } else if direction == ChapterDirection::Previous {
            self.engine.scroll_y = 0.0;
            self.engine.pending_heading_jump = None;
        }
    }
}

pub struct NovelViewFactory;

impl PluginFactory for NovelViewFactory {
    fn name(&self) -> &str {
        "novel_view"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.extension()).is_some_and(|e| e == "txt")
    }
    fn create(&self) -> Box<dyn ViewPluginCapabilities> {
        Box::new(NovelView::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_of_contents_keeps_novel_titles_outside_the_viewport() {
        let source = format!(
            "  《示例小说》  \n\n  第一章 起点  \n\n{}\n第二章 远方\n",
            "正文。\n\n".repeat(120),
        );
        let document = textora_core::document::StringDocView::new(&source);
        let theme = ui::theme::test_theme();
        let bounds = ui::Rect::new(0.0, 0.0, 800.0, 200.0);
        let mut shaper = shaping::Shaper::new().expect("novel layout requires fonts");
        let mut view = NovelView::new();

        view.render(&document, bounds, &theme, &mut shaper, 1.0);

        let headings = view.outline().expect("novel view provides an outline").headings();
        let titles: Vec<_> = headings.iter().map(|heading| heading.title.as_str()).collect();
        assert_eq!(titles, ["《示例小说》", "第一章 起点", "第二章 远方"]);
        assert!(headings[2].y > bounds.h);
    }

    #[test]
    fn previous_chapter_in_the_first_chapter_returns_to_the_start() {
        let source = format!("第一章 起点\n\n{}", "正文。\n\n".repeat(48));
        let document = textora_core::document::StringDocView::new(&source);
        let theme = ui::theme::test_theme();
        let bounds = ui::Rect::new(0.0, 0.0, 800.0, 200.0);
        let mut shaper = shaping::Shaper::new().expect("novel layout requires fonts");
        let mut view = NovelView::new();
        view.render(&document, bounds, &theme, &mut shaper, 1.0);
        assert!(view.engine.scroll(bounds.h, bounds.h));
        assert_eq!(view.engine.current_heading_index(view.engine.scroll_y), Some(0));

        view.outline_mut()
            .expect("novel view provides a mutable outline")
            .scroll_to_chapter(ui::plugin::ChapterDirection::Previous);

        assert_eq!(view.engine.scroll_y, 0.0);
    }
}
