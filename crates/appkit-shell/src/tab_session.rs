use std::borrow::Cow;

use appkit_core::document::DocumentModel;
use appkit_core::tab_set::types::TabId;
use shaping::Shaper;
use textora_core::document::DocView;
use ui::canvas::CanvasViewportSnapshot;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::core::widget::{KeyCode, Modifiers};
use ui::plugin::{
    CanvasContentMetrics, CanvasDragRequest, CanvasDragResponse, ViewPluginCapabilities,
};
use ui::theme::Theme;

use crate::cursor_motion::CursorRenderState;
use crate::display_state::DisplayState;
use crate::document_presentation::{DocumentPresentation, SourceSpacingState};
use crate::tab_runtime::TabRuntime;

fn document_line_text(document: &DocumentModel, line: usize) -> Cow<'_, str> {
    bytes_as_text(document.document_line_bytes(line).unwrap_or(Cow::Borrowed(&[])))
}

fn document_text_in_range(document: &DocumentModel, range: std::ops::Range<usize>) -> Cow<'_, str> {
    bytes_as_text(document.document_bytes_in_range(range))
}

fn bytes_as_text(bytes: Cow<'_, [u8]>) -> Cow<'_, str> {
    match bytes {
        Cow::Borrowed(bytes) => Cow::Borrowed(
            std::str::from_utf8(bytes).expect("document model must preserve valid UTF-8"),
        ),
        Cow::Owned(bytes) => {
            Cow::Owned(String::from_utf8(bytes).expect("document model must preserve valid UTF-8"))
        }
    }
}

struct PresentedDocument<'a> {
    document: &'a DocumentModel,
    presentation: &'a DocumentPresentation,
}

impl DocView for PresentedDocument<'_> {
    fn line_count(&self) -> usize {
        self.document.line_count()
    }

    fn doc_line_text(&self, line: usize) -> Cow<'_, str> {
        document_line_text(self.document, line)
    }

    fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> Cow<'_, str> {
        document_text_in_range(self.document, range)
    }

    fn line_byte_offset(&self, line: usize) -> usize {
        self.document.line_byte_offset(line).unwrap_or(0)
    }

    fn line_byte_length(&self, line: usize) -> usize {
        self.document.line_byte_length(line).unwrap_or(0)
    }

    fn scroll_y(&self) -> f32 {
        self.presentation.display.viewport.scroll_top as f32
    }

    fn viewport_height(&self) -> f32 {
        self.presentation.display.viewport.viewport_height as f32
    }

    fn is_empty(&self) -> bool {
        self.document.is_empty()
    }
}

pub struct TabSession<'a> {
    pub id: TabId,
    pub(crate) document: &'a DocumentModel,
    pub runtime: &'a TabRuntime,
}

impl<'a> TabSession<'a> {
    pub fn new(id: TabId, document: &'a DocumentModel, runtime: &'a TabRuntime) -> Self {
        Self { id, document, runtime }
    }

    pub fn document(&self) -> &'a DocumentModel {
        self.document
    }

    pub fn plugin(&self) -> &'a dyn ViewPluginCapabilities {
        self.runtime.plugin.as_ref()
    }

    pub fn plugin_name(&self) -> &'a str {
        self.plugin().name()
    }

    pub fn paste_preference(&self) -> ui::plugin::PastePreference {
        self.runtime
            .plugin
            .editable()
            .map_or(ui::plugin::PastePreference::PlainText, |view| view.paste_preference())
    }

    pub fn allows_editing(&self) -> bool {
        self.runtime.editing_access() == crate::tab_runtime::DocumentEditingAccess::Editable
            && self.runtime.plugin.allows_editing()
    }

    pub fn editing_access(&self) -> crate::tab_runtime::DocumentEditingAccess {
        self.runtime.editing_access()
    }

    pub fn handles_own_rendering(&self) -> bool {
        self.runtime.plugin.mode().renders_content()
    }

    pub fn is_canvas(&self) -> bool {
        self.runtime.plugin.mode() == ui::plugin::PluginMode::Canvas
    }

    pub fn shows_gutter(&self) -> bool {
        self.runtime.plugin.mode().uses_host_gutter()
    }

    pub fn needs_cursor_blink_wakeup(&self) -> bool {
        self.runtime.plugin.needs_cursor_blink_wakeup()
    }

    pub fn content_height(&self) -> f32 {
        self.runtime.plugin.scrollable().map_or(0.0, |view| view.content_height())
    }

    pub fn scroll_y(&self) -> f32 {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.scrollable().map_or(0.0, |view| view.scroll_y(&document))
    }

    pub fn plugin_has_selection(&self) -> bool {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection.has_selection();
        }
        false
    }

    pub fn selected_text(&self) -> String {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection.selected_text().unwrap_or_default();
        }
        String::new()
    }

    fn selection_cursor(&self) -> Option<(usize, usize)> {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection.selection_cursor().map(ui::plugin::ReadingPosition::indices);
        }
        None
    }

    pub fn flat_lines(&self) -> Vec<ui::plugin::FlatLine> {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection.flat_lines();
        }
        Vec::new()
    }

    pub fn hit_test_position(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<(usize, usize)> {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection
                .hit_test_position(x, y, offset_x, offset_y)
                .map(ui::plugin::ReadingPosition::indices);
        }
        None
    }

    fn word_range_at_pos(
        &self,
        line_index: usize,
        cluster_pos: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection
                .word_range(ui::plugin::ReadingPosition::new(line_index, cluster_pos))
                .map(|range| (range.start.indices(), range.end.indices()));
        }
        None
    }

    fn line_range_at_pos(
        &self,
        line_index: usize,
        cluster_pos: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection
                .line_range(ui::plugin::ReadingPosition::new(line_index, cluster_pos))
                .map(|range| (range.start.indices(), range.end.indices()));
        }
        None
    }

    pub fn hit_test_caret(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<ui::plugin::VisualCaretPosition> {
        self.runtime.plugin.editable()?.hit_test_caret(x, y, offset_x, offset_y)
    }

    pub fn hit_test_byte(&self, x: f32, y: f32, offset_x: f32, offset_y: f32) -> Option<usize> {
        self.hit_test_caret(x, y, offset_x, offset_y).map(|caret| caret.byte_offset)
    }

    pub fn query_cursor_screen_rect(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        self.runtime.plugin.editable()?.cursor_screen_rect(cursor_byte)
    }

    pub fn hit_test_edit_target(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<Option<ui::plugin::EditHitTarget>> {
        self.runtime.plugin.editable()?.hit_test_edit_target(x, y, offset_x, offset_y)
    }

    pub fn hit_test_canvas_control(
        &self,
        position: ui::canvas::CanvasPoint,
    ) -> Option<std::ops::Range<usize>> {
        self.runtime.plugin.canvas()?.hit_test_canvas_control(position)
    }

    pub fn canvas_control_edit_plan(
        &self,
        source_range: std::ops::Range<usize>,
        source_generation: u32,
    ) -> Option<ui::plugin::EditPlan> {
        if !self.allows_editing() {
            return None;
        }
        Some(self.runtime.plugin.canvas()?.plan_canvas_control(source_range, source_generation))
    }

    pub fn move_edit_target(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<ui::plugin::EditHitTarget> {
        self.runtime.plugin.editable()?.move_edit_target(current_byte, direction, target_x)
    }

    pub fn visual_move(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<ui::plugin::VisualCaretPosition> {
        self.runtime.plugin.editable()?.visual_move(current_byte, direction, target_x)
    }

    pub fn plan_semantic_edit(
        &self,
        command: ui::plugin::SemanticEditCommand,
        selection: Option<std::ops::Range<usize>>,
    ) -> ui::plugin::SemanticEditPlan {
        let Some(view) = self.runtime.plugin.editable() else {
            return ui::plugin::SemanticEditPlan::Unsupported;
        };
        view.plan_semantic_edit(
            command,
            self.document.generation(),
            self.document.cursor_offset().to_usize(),
            selection,
        )
    }

    pub fn table_structure_capabilities(
        &self,
        cursor_byte: usize,
    ) -> Option<ui::plugin::TableStructureCapabilities> {
        self.runtime
            .plugin
            .editable()?
            .table_structure_capabilities(self.document.generation(), cursor_byte)
    }

    pub fn plan_edit_request(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        self.runtime
            .plugin
            .editable()
            .map_or(ui::plugin::EditPlan::UseDefault, |policy| policy.plan_edit(request))
    }

    pub fn selection_highlights(&self, color: [f32; 4]) -> DrawList {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection.selection_highlights(color);
        }
        DrawList::new()
    }

    pub fn search_highlights(
        &self,
        query: String,
        match_case: bool,
        use_regex: bool,
        active_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    ) -> DrawList {
        let Some(view) = self.runtime.plugin.search() else {
            return DrawList::new();
        };
        let search = self.search_state();
        if query == search.query
            && match_case == search.options.match_case
            && use_regex == search.options.use_regex
            && let Some(draw_list) =
                view.source_search_highlights(ui::plugin::SourceSearchHighlightsRequest {
                    matches: &search.matches,
                    source_generation: search.buffer_generation,
                    active_idx,
                    match_color,
                    inactive_color,
                })
        {
            return draw_list;
        }
        view.search_highlights(ui::plugin::SearchHighlightsRequest {
            query: &query,
            match_case,
            use_regex,
            active_idx,
            match_color,
            inactive_color,
        })
    }

    fn selection_byte_range(&self) -> Option<(usize, usize)> {
        if let Some(selection) = self.runtime.plugin.selection() {
            return selection.selection_source_range().map(|range| (range.start, range.end));
        }
        None
    }

    pub fn scroll_anchor(&self) -> Option<(String, f32)> {
        self.runtime.plugin.scrollable()?.scroll_anchor()
    }

    pub fn has_canvas_viewport_snapshot(&self) -> bool {
        self.runtime.canvas_viewport.snapshot().is_some()
    }

    pub fn toc_visible(&self) -> bool {
        self.runtime.toc_visible
    }

    fn presentation(&self) -> &DocumentPresentation {
        &self.runtime.presentation
    }

    pub fn display(&self) -> &DisplayState {
        &self.presentation().display
    }

    pub fn source_spacing(&self) -> &SourceSpacingState {
        &self.presentation().source_spacing
    }

    pub fn search_state(&self) -> &appkit_core::document::SearchState {
        &self.presentation().search_state
    }

    pub fn cursor_render_state(&self) -> &CursorRenderState {
        &self.presentation().cursor_render_state
    }

    pub fn display_map_entry(
        &self,
        doc_line: usize,
    ) -> Option<&crate::snap_tree::DisplayLineEntry> {
        self.display().display_map.get_entry(doc_line)
    }

    pub fn visible_rows(&self) -> usize {
        self.display().viewport.visible_rows
    }

    pub fn viewport_height(&self) -> f64 {
        self.display().viewport.viewport_height
    }

    pub fn scroll_top(&self) -> f64 {
        self.display().viewport.scroll_top
    }

    pub fn sub_line_pixel_offset(&self, line_height: f32) -> f32 {
        self.display().viewport.sub_line_pixel_offset(line_height)
    }

    pub fn total_display_rows(&self) -> usize {
        self.display().display_map.total_rows()
    }

    pub fn cursor_visual_line(&self) -> Option<usize> {
        self.cursor_render_state().cursor_visual_line
    }

    pub fn cursor_pixel_x(&self) -> f32 {
        self.cursor_render_state().cursor_pixel_x
    }

    pub fn cursor_blink_instant(&self) -> std::time::Instant {
        self.cursor_render_state().cursor_blink_instant
    }

    pub fn scroll_anchor_doc_line(&self) -> usize {
        self.display().viewport.scroll_anchor.doc_line
    }

    pub fn scroll_anchor_pixel_offset(&self) -> f32 {
        self.display().viewport.scroll_anchor.pixel_offset
    }

    pub fn scroll_anchor_state(&self) -> ui::viewport::ScrollAnchor {
        self.display().viewport.scroll_anchor
    }

    pub fn visible_doc_range_from_anchor(&self, line_height: f32) -> std::ops::Range<usize> {
        self.display()
            .viewport
            .visible_doc_range_from_anchor(&self.display().display_map, line_height)
    }

    pub fn advance_cache(&self) -> &[ui::render_geom::AdvanceCacheEntry] {
        &self.display().advance_cache
    }

    pub fn map_key_intent(
        &self,
        key_code: &KeyCode,
        modifiers: &Modifiers,
    ) -> Option<ui::plugin::EditIntent> {
        self.runtime
            .plugin
            .editable()?
            .key_intent_mapper()
            .and_then(|mapper| mapper.map_key(key_code, modifiers))
    }
}

pub struct TabSessionMut<'a> {
    pub id: TabId,
    pub(crate) document: &'a mut DocumentModel,
    pub runtime: &'a mut TabRuntime,
}

impl<'a> TabSessionMut<'a> {
    pub fn new(id: TabId, document: &'a mut DocumentModel, runtime: &'a mut TabRuntime) -> Self {
        Self { id, document, runtime }
    }

    fn as_ref(&self) -> TabSession<'_> {
        TabSession::new(self.id, self.document, self.runtime)
    }

    pub fn document(&self) -> &DocumentModel {
        self.document
    }

    pub fn document_mut(&mut self) -> &mut DocumentModel {
        self.document
    }

    pub fn plugin(&self) -> &dyn ViewPluginCapabilities {
        self.runtime.plugin.as_ref()
    }

    pub fn scroll_to_heading(&mut self, index: usize) {
        if let Some(outline) = self.runtime.plugin.outline_mut() {
            outline.scroll_to_heading(index);
        }
    }

    pub fn scroll_to_chapter(&mut self, direction: ui::plugin::ChapterDirection) {
        if let Some(outline) = self.runtime.plugin.outline_mut() {
            outline.scroll_to_chapter(direction);
        }
    }

    pub fn plugin_name(&self) -> &str {
        self.plugin().name()
    }

    pub fn allows_editing(&self) -> bool {
        self.as_ref().allows_editing()
    }

    pub fn handles_own_rendering(&self) -> bool {
        self.as_ref().handles_own_rendering()
    }

    pub fn is_canvas(&self) -> bool {
        self.runtime.plugin.mode() == ui::plugin::PluginMode::Canvas
    }

    pub fn has_canvas_viewport_snapshot(&self) -> bool {
        self.runtime.canvas_viewport.snapshot().is_some()
    }

    pub fn apply_canvas_viewport_action(
        &mut self,
        action: crate::canvas_viewport::CanvasViewportAction,
    ) {
        self.runtime.canvas_viewport.apply(action);
    }

    pub fn source_spacing_protected_ranges(
        &self,
    ) -> Option<std::sync::Arc<[std::ops::Range<usize>]>> {
        self.runtime.plugin.source_spacing_protected_ranges(self.document.generation())
    }

    pub fn content_height(&self) -> f32 {
        self.as_ref().content_height()
    }

    pub fn scroll_y(&self) -> f32 {
        self.as_ref().scroll_y()
    }

    pub fn set_view_render_settings(&mut self, settings: ui::plugin::ViewRenderSettings) {
        self.runtime.plugin.set_view_render_settings(settings);
    }

    pub fn set_view_text_spacing_mode(&mut self, mode: ui::typography::TextSpacingMode) {
        self.runtime.plugin.set_view_text_spacing_mode(mode);
    }

    pub fn scroll_to_search_match(&mut self, query: &str, match_case: bool, active_idx: usize) {
        if let Some(search) = self.runtime.plugin.search_mut() {
            search.scroll_to_search_match(query, match_case, active_idx);
        }
    }

    pub fn set_plugin_selection_cursor(&mut self, position: Option<(usize, usize)>) -> bool {
        let Some(selection) = self.runtime.plugin.selection_mut() else {
            return false;
        };
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        let position =
            position.map(|(line, grapheme)| ui::plugin::ReadingPosition::new(line, grapheme));
        selection.set_selection_cursor(position, &document)
    }

    pub fn set_plugin_selection_cursor_byte(&mut self, byte: Option<usize>) -> bool {
        if let Some(selection) = self.runtime.plugin.selection_mut() {
            return selection.set_selection_cursor_byte(byte);
        }
        false
    }

    pub fn set_plugin_selection_anchor(&mut self, position: Option<(usize, usize)>) -> bool {
        let Some(selection) = self.runtime.plugin.selection_mut() else {
            return false;
        };
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        let position =
            position.map(|(line, grapheme)| ui::plugin::ReadingPosition::new(line, grapheme));
        selection.set_selection_anchor(position, &document)
    }

    pub fn set_plugin_selection_anchor_byte(&mut self, byte: Option<usize>) -> bool {
        if let Some(selection) = self.runtime.plugin.selection_mut() {
            return selection.set_selection_anchor_byte(byte);
        }
        false
    }

    pub fn clear_plugin_selection(&mut self) -> bool {
        if let Some(selection) = self.runtime.plugin.selection_mut() {
            return selection.clear_selection();
        }
        false
    }

    pub fn select_all_in_plugin(&mut self) -> bool {
        if let Some(selection) = self.runtime.plugin.selection_mut() {
            return selection.select_all();
        }
        false
    }

    pub fn plugin_has_selection(&self) -> bool {
        self.as_ref().plugin_has_selection()
    }

    pub fn selection_cursor(&self) -> Option<(usize, usize)> {
        self.as_ref().selection_cursor()
    }

    pub fn flat_lines(&self) -> Vec<ui::plugin::FlatLine> {
        self.as_ref().flat_lines()
    }

    pub fn hit_test_position(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<(usize, usize)> {
        self.as_ref().hit_test_position(x, y, offset_x, offset_y)
    }

    pub fn word_range_at_pos(
        &self,
        line_index: usize,
        cluster_pos: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        self.as_ref().word_range_at_pos(line_index, cluster_pos)
    }

    pub fn line_range_at_pos(
        &self,
        line_index: usize,
        cluster_pos: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        self.as_ref().line_range_at_pos(line_index, cluster_pos)
    }

    pub fn hit_test_byte(&self, x: f32, y: f32, offset_x: f32, offset_y: f32) -> Option<usize> {
        self.as_ref().hit_test_byte(x, y, offset_x, offset_y)
    }

    pub fn hit_test_edit_target(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<Option<ui::plugin::EditHitTarget>> {
        self.as_ref().hit_test_edit_target(x, y, offset_x, offset_y)
    }

    fn presentation(&self) -> &DocumentPresentation {
        &self.runtime.presentation
    }

    fn presentation_mut(&mut self) -> &mut DocumentPresentation {
        &mut self.runtime.presentation
    }

    pub fn with_presentation<Output>(
        &mut self,
        operation: impl FnOnce(&mut DocumentModel, &mut DocumentPresentation) -> Output,
    ) -> Output {
        operation(self.document, &mut self.runtime.presentation)
    }

    pub fn source_spacing(&self) -> &SourceSpacingState {
        &self.presentation().source_spacing
    }

    pub fn set_source_spacing(&mut self, source_spacing: SourceSpacingState) {
        self.presentation_mut().source_spacing = source_spacing;
    }

    pub fn display(&self) -> &DisplayState {
        &self.presentation().display
    }

    pub fn display_mut(&mut self) -> &mut DisplayState {
        &mut self.presentation_mut().display
    }

    pub fn search_state(&self) -> &appkit_core::document::SearchState {
        &self.presentation().search_state
    }

    pub fn search_state_mut(&mut self) -> &mut appkit_core::document::SearchState {
        &mut self.presentation_mut().search_state
    }

    pub fn cursor_render_state(&self) -> &CursorRenderState {
        &self.presentation().cursor_render_state
    }

    pub fn cursor_render_state_mut(&mut self) -> &mut CursorRenderState {
        &mut self.presentation_mut().cursor_render_state
    }

    pub fn display_map_entry(
        &self,
        doc_line: usize,
    ) -> Option<&crate::snap_tree::DisplayLineEntry> {
        self.display().display_map.get_entry(doc_line)
    }

    pub fn visible_rows(&self) -> usize {
        self.display().viewport.visible_rows
    }

    pub fn scroll_top(&self) -> f64 {
        self.display().viewport.scroll_top
    }

    pub fn cursor_visual_line(&self) -> Option<usize> {
        self.cursor_render_state().cursor_visual_line
    }

    pub fn cursor_blink_instant(&self) -> std::time::Instant {
        self.cursor_render_state().cursor_blink_instant
    }

    pub fn last_cursor_offset(&self) -> textora_core::types::ByteIndex {
        self.cursor_render_state().last_cursor_offset
    }

    pub fn set_last_cursor_offset(&mut self, offset: textora_core::types::ByteIndex) {
        self.cursor_render_state_mut().last_cursor_offset = offset;
    }

    pub(crate) fn invalidate_cursor_visibility(&mut self) {
        self.set_last_cursor_offset(textora_core::types::ByteIndex::MAX);
    }

    pub fn scroll_anchor_doc_line(&self) -> usize {
        self.display().viewport.scroll_anchor.doc_line
    }

    pub fn set_scroll_anchor_state(&mut self, anchor: ui::viewport::ScrollAnchor) {
        self.display_mut().viewport.scroll_anchor = anchor;
    }

    pub fn set_scroll_anchor(&mut self, doc_line: usize, pixel_offset: f32) {
        self.display_mut().viewport.scroll_anchor =
            ui::viewport::ScrollAnchor::new(doc_line, pixel_offset);
    }

    pub fn set_scroll_top_rows(&mut self, scroll_top: f64, line_height: f32) {
        let DisplayState { viewport, display_map, .. } = self.display_mut();
        viewport.set_scroll_top(scroll_top, display_map, line_height);
        viewport.derive_scroll_top(display_map, line_height);
    }

    pub fn scroll_viewport_by_pages(&mut self, amount: f64, line_height: f32) {
        let page_pixels = self.visible_rows().max(1) as f32 * line_height;
        let pixels = if amount > 0.0 { page_pixels } else { -page_pixels };
        self.scroll_viewport_by_pixels(pixels, line_height);
    }

    pub fn scroll_viewport_by_pixels(&mut self, pixels: f32, line_height: f32) {
        let DisplayState { viewport, display_map, .. } = self.display_mut();
        viewport.scroll_pixels(pixels, display_map, line_height);
        self.refresh_scroll_metrics(line_height);
    }

    pub fn clamp_scroll_anchor(&mut self, line_height: f32) {
        let DisplayState { viewport, display_map, .. } = self.display_mut();
        viewport.clamp_anchor(display_map, line_height);
    }

    pub fn derive_scroll_top(&mut self, line_height: f32) {
        let DisplayState { viewport, display_map, .. } = self.display_mut();
        viewport.derive_scroll_top(display_map, line_height);
    }

    pub fn visible_doc_range_from_anchor(&self, line_height: f32) -> std::ops::Range<usize> {
        self.display()
            .viewport
            .visible_doc_range_from_anchor(&self.display().display_map, line_height)
    }

    pub fn clear_advance_cache(&mut self) {
        self.display_mut().advance_cache.clear();
    }

    pub fn invalidate_render_cache_all(&mut self) {
        self.display_mut().render_cache.invalidate_all();
    }

    pub fn invalidate_render_cache_line(&mut self, doc_line: usize) {
        self.display_mut().render_cache.invalidate(doc_line);
    }

    pub fn update_display_map_entry(
        &mut self,
        doc_line: usize,
        entry: crate::snap_tree::DisplayLineEntry,
    ) {
        self.display_mut().display_map.update_entry_in_place(doc_line, entry);
    }

    pub fn rebuild_display_map(&mut self) {
        self.display_mut().display_map.rebuild_tree();
    }

    pub fn refresh_scroll_metrics(&mut self, line_height: f32) {
        self.clamp_scroll_anchor(line_height);
        self.derive_scroll_top(line_height);
    }

    pub fn resize_and_refresh_presentation(
        &mut self,
        visible_rows: usize,
        viewport_height: f64,
        line_height: f32,
    ) {
        self.display_mut().viewport.resize(visible_rows, viewport_height);
        self.refresh_scroll_metrics(line_height);
    }

    pub fn resize_presentation(&mut self, visible_rows: usize, viewport_height: f64) {
        self.display_mut().viewport.resize(visible_rows, viewport_height);
    }

    pub fn ensure_cursor_visible(&mut self, line_height: f32) {
        let cursor_line = self.document.cursor_line();
        let visible_range = self.visible_doc_range_from_anchor(line_height);
        let anchor = self.scroll_anchor_doc_line();

        if visible_range.contains(&cursor_line) {
            return;
        }

        self.cursor_render_state_mut().click_hint = None;
        if cursor_line < anchor {
            self.set_scroll_anchor(cursor_line, 0.0);
        } else {
            let visible_count = visible_range.len().max(1);
            self.set_scroll_anchor(
                cursor_line.saturating_sub(visible_count.saturating_sub(1)),
                0.0,
            );
        }
        self.refresh_scroll_metrics(line_height);
    }

    pub(crate) fn ensure_cursor_visual_row_visible(&mut self, line_height: f32) -> bool {
        const PIXEL_COMPARISON_TOLERANCE: f32 = 0.01;

        let cursor_offset = self.document.cursor().offset;
        if cursor_offset == self.last_cursor_offset() {
            return false;
        }
        self.set_last_cursor_offset(cursor_offset);

        let Some(cursor_visual_row) = self.cursor_visual_line() else {
            return false;
        };
        let cursor_bottom_px = (cursor_visual_row + 1) as f32 * line_height
            + self.display().viewport.sub_line_pixel_offset(line_height);
        let viewport_bottom_px = self.display().viewport.viewport_height as f32 * line_height;
        let overflow_px = cursor_bottom_px - viewport_bottom_px;
        if overflow_px <= PIXEL_COMPARISON_TOLERANCE {
            return false;
        }

        self.cursor_render_state_mut().click_hint = None;
        self.scroll_viewport_by_pixels(overflow_px, line_height);
        true
    }

    pub fn page_up(&mut self, line_height: f32) {
        self.move_page(-1.0, line_height);
    }

    pub fn page_down(&mut self, line_height: f32) {
        self.move_page(1.0, line_height);
    }

    fn move_page(&mut self, direction: f64, line_height: f32) {
        self.scroll_viewport_by_pages(direction, line_height);
        let first_document_line = self.visible_doc_range_from_anchor(line_height).start;
        if let Some(offset) = self.document.line_byte_offset(first_document_line) {
            self.document.set_cursor_offset_synced(offset);
        }
        self.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
    }

    pub(crate) fn move_cursor_visual(
        &mut self,
        delta: isize,
        frame_cache: &crate::frame_cache::FrameCache,
        dpi_scale: f32,
    ) {
        self.with_presentation(|document, presentation| {
            let viewport = &presentation.display.viewport;
            let cursor = &mut presentation.cursor_render_state;
            let context = crate::cursor_motion::CursorContext {
                display_map: &presentation.display.display_map,
                cursor_visual_line: cursor.cursor_visual_line,
                advance_cache: &presentation.display.advance_cache,
                first_line: &frame_cache.first_line,
                last_line: &frame_cache.last_line,
                visible_rows: viewport.visible_rows,
                first_visible_row: viewport.first_visible_row(),
                scroll_top: viewport.scroll_top,
                sticky_x: cursor.sticky_x,
                dpi_scale,
            };
            if let Some(offset) = crate::cursor_motion::move_cursor_visual(delta, context, document)
            {
                document.set_cursor_offset_synced(offset.to_usize());
            }
            cursor.cursor_blink_instant = std::time::Instant::now();
        });
    }

    pub fn sync_document(&mut self, reason: ui::plugin::SyncReason) -> ui::plugin::SyncOutcome {
        let document = &*self.document;
        self.runtime
            .plugin
            .sync_document(document.generation(), reason, &mut || document.full_text())
    }

    pub fn selection_byte_range(&self) -> Option<(usize, usize)> {
        self.as_ref().selection_byte_range()
    }

    pub fn set_cursor_byte(&mut self, cursor_byte: usize) {
        if let Some(view) = self.runtime.plugin.editable_mut() {
            view.set_cursor_byte(cursor_byte);
        }
    }

    pub fn set_visual_caret(&mut self, position: ui::plugin::VisualCaretPosition) {
        if let Some(view) = self.runtime.plugin.editable_mut() {
            view.set_visual_caret(position);
        }
    }

    pub fn clear_edit_focus(&mut self) {
        if let Some(view) = self.runtime.plugin.editable_mut() {
            view.clear_edit_focus();
        }
    }

    pub fn reveal_cursor(&mut self, cursor_byte: usize) -> bool {
        self.runtime.plugin.editable_mut().is_some_and(|view| view.reveal_cursor(cursor_byte))
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        if let Some(view) = self.runtime.plugin.editable_mut() {
            view.set_cursor_visible(visible);
        }
    }

    pub fn set_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if let Some(view) = self.runtime.plugin.editable_mut() {
            view.set_preedit(text, cursor);
        }
    }

    pub fn scroll_plugin(&mut self, delta: f32, viewport_height: f32) -> bool {
        self.runtime.plugin.scrollable_mut().is_some_and(|view| view.scroll(delta, viewport_height))
    }

    pub fn restore_plugin_scroll_y(&mut self, y: f32) -> bool {
        self.runtime.plugin.scrollable_mut().is_some_and(|view| view.restore_y(y))
    }

    pub fn restore_plugin_scroll_ratio(&mut self, ratio: f32) -> bool {
        self.runtime.plugin.scrollable_mut().is_some_and(|view| view.restore_ratio(ratio))
    }

    pub fn restore_plugin_scroll_anchor(&mut self, text: &str, offset: f32) -> bool {
        self.runtime.plugin.scrollable_mut().is_some_and(|view| view.restore_anchor(text, offset))
    }

    pub fn toggle_toc_visible(&mut self) {
        self.runtime.toc_visible = !self.runtime.toc_visible;
    }

    pub fn swap_in_toggle_plugin(&mut self, replacement: Box<dyn ViewPluginCapabilities>) {
        self.runtime.toggle_source_scroll_y = self.scroll_y();
        self.runtime.cached_toggle_source = Some(self.runtime.replace_plugin(replacement));
    }

    pub fn restore_cached_toggle_source(&mut self) -> bool {
        let Some(mut cached) = self.runtime.cached_toggle_source.take() else {
            return false;
        };
        let scroll_y = self.runtime.toggle_source_scroll_y;
        if let Some(scrollable) = cached.scrollable_mut() {
            scrollable.restore_y(scroll_y);
        }
        self.runtime.replace_plugin(cached);
        true
    }

    pub fn cache_toggle_source_plugin(&mut self, plugin: Box<dyn ViewPluginCapabilities>) {
        self.runtime.cached_toggle_source = Some(plugin);
    }

    pub fn replace_plugin(&mut self, plugin: Box<dyn ViewPluginCapabilities>) {
        self.runtime.replace_plugin(plugin);
    }

    pub fn prepare_canvas_viewport(
        &mut self,
        metrics: CanvasContentMetrics,
        bounds: Rect,
        dpi: f32,
    ) -> Option<CanvasViewportSnapshot> {
        self.runtime.canvas_viewport.prepare(
            metrics,
            bounds,
            ui::canvas::CanvasViewportConfig::for_dpi(dpi),
        )
    }

    pub fn canvas_viewport_scrollbars_input(
        &self,
    ) -> crate::canvas_viewport::CanvasViewportScrollbarsInput {
        self.runtime.canvas_viewport.scrollbars_input()
    }

    pub fn render_plugin(
        &mut self,
        bounds: Rect,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.render(&document, bounds, theme, shaper, dpi_scale)
    }

    pub fn prepare_canvas_plugin(
        &mut self,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> Option<CanvasContentMetrics> {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.canvas_mut()?.prepare_canvas(&document, theme, shaper, dpi_scale)
    }

    pub fn render_canvas_plugin(
        &mut self,
        snapshot: &CanvasViewportSnapshot,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.canvas_mut().map_or_else(DrawList::new, |canvas| {
            canvas.render_canvas(&document, snapshot, theme, shaper, dpi_scale)
        })
    }

    pub fn set_canvas_pointer(&mut self, pointer: Option<ui::canvas::CanvasPoint>) -> bool {
        self.runtime.plugin.canvas_mut().is_some_and(|canvas| canvas.set_canvas_pointer(pointer))
    }

    pub fn handle_canvas_drag_plugin(&mut self, request: CanvasDragRequest) -> CanvasDragResponse {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.canvas_mut().map_or(CanvasDragResponse::Ignore, |canvas| {
            canvas.handle_canvas_drag(request, &document)
        })
    }
}

#[cfg(test)]
mod tests {
    use appkit_core::document::DocumentModel;
    use appkit_core::tab_set::types::TabIdAllocator;
    use textora_core::buffer::TextBuffer;
    use textora_core::document::DocView;
    use ui::plugin::{PastePreference, ViewPlugin};

    use crate::tab_runtime::TabRuntime;

    use super::{TabSession, TabSessionMut};
    use crate::editor_plugin::EditorPlugin;

    fn document(text: &str) -> DocumentModel {
        let mut buffer =
            TextBuffer::new(false).expect("tab session test buffer must be constructible");
        buffer.write_raw(text.as_bytes());
        DocumentModel::new(buffer)
    }

    struct ScrollProbePlugin;

    impl ViewPlugin for ScrollProbePlugin {
        fn name(&self) -> &str {
            "scroll-probe"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::Source
        }

        fn render(
            &mut self,
            _document: &dyn DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
    }

    impl ui::plugin::ScrollableView for ScrollProbePlugin {
        fn scroll_y(&self, document: &dyn DocView) -> f32 {
            document.scroll_y()
        }
    }

    impl ui::plugin::EditPolicy for ScrollProbePlugin {
        fn plan_edit(&self, _: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
            ui::plugin::EditPlan::UseDefault
        }
    }

    impl ui::plugin::EditableView for ScrollProbePlugin {
        fn paste_preference(&self) -> PastePreference {
            PastePreference::SemanticMarkdown
        }
    }

    impl ui::plugin::ViewPluginCapabilities for ScrollProbePlugin {
        fn editable(&self) -> Option<&dyn ui::plugin::EditableView> {
            Some(self)
        }

        fn scrollable(&self) -> Option<&dyn ui::plugin::ScrollableView> {
            Some(self)
        }
    }

    struct ModeProbe(ui::plugin::PluginMode);

    impl ViewPlugin for ModeProbe {
        fn name(&self) -> &str {
            "mode-probe"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            self.0
        }

        fn render(
            &mut self,
            _document: &dyn DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }
    }

    impl ui::plugin::ViewPluginCapabilities for ModeProbe {}

    #[test]
    fn view_mode_routes_rendering_and_document_access_gates_editing() {
        use crate::tab_runtime::DocumentEditingAccess::ReadOnly;
        use ui::plugin::{EditIntent, EditPlan, EditRequest, PluginMode};

        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        for (mode, draws_content, editable) in [
            (PluginMode::Source, false, true),
            (PluginMode::Reading, true, false),
            (PluginMode::RichText, true, true),
            (PluginMode::Canvas, true, true),
        ] {
            let mut runtime = TabRuntime::new(Box::new(ModeProbe(mode)));
            let session = TabSession::new(id, &document, &runtime);
            assert_eq!(session.handles_own_rendering(), draws_content);
            assert_eq!(session.is_canvas(), mode == PluginMode::Canvas);
            assert_eq!(session.shows_gutter(), mode == PluginMode::Source);
            assert_eq!(session.allows_editing(), editable);

            if mode == PluginMode::Source {
                let request = EditRequest {
                    source_generation: document.generation(),
                    cursor_byte: 0,
                    selection: None,
                    intent: EditIntent::InsertText("text".into()),
                };
                assert_eq!(session.plan_edit_request(&request), EditPlan::UseDefault);
            }

            runtime.set_editing_access(ReadOnly);
            let session = TabSession::new(id, &document, &runtime);
            assert!(!session.allows_editing());
            assert_eq!(session.handles_own_rendering(), draws_content);
            assert_eq!(session.is_canvas(), mode == PluginMode::Canvas);
        }
    }

    #[test]
    fn immutable_session_preserves_tab_identity_and_borrows_state() {
        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        let runtime = TabRuntime::new(Box::new(EditorPlugin::new()));

        let session = TabSession::new(id, &document, &runtime);

        assert_eq!(session.id, id);
        assert_eq!(session.document().full_text(), "hello");
        assert_eq!(session.plugin().name(), ui::plugin::PLUGIN_EDITOR);
    }

    #[test]
    fn document_selection_is_independent_of_plugin_selection() {
        let id = TabIdAllocator::new().allocate();
        let mut document = document("hello");
        let mut runtime = TabRuntime::new(Box::new(ScrollProbePlugin));
        let mut session = TabSessionMut::new(id, &mut document, &mut runtime);

        session.document_mut().select_all();

        assert!(session.document().has_selection());
        assert!(!session.plugin_has_selection());
        assert_eq!(session.plugin().name(), "scroll-probe");
    }

    #[test]
    fn plugin_queries_receive_runtime_presentation_viewport() {
        const RUNTIME_SCROLL: f64 = 17.0;

        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        let mut runtime = TabRuntime::new(Box::new(ScrollProbePlugin));
        runtime.presentation.display.viewport.scroll_top = RUNTIME_SCROLL;

        let session = TabSession::new(id, &document, &runtime);

        assert_eq!(session.scroll_y(), RUNTIME_SCROLL as f32);
    }

    #[test]
    fn session_paste_preference_forwards_active_plugin() {
        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        let runtime = TabRuntime::new(Box::new(ScrollProbePlugin));
        let session = TabSession::new(id, &document, &runtime);

        assert_eq!(session.paste_preference(), PastePreference::SemanticMarkdown);
    }

    #[test]
    fn source_editor_session_keeps_plain_text_paste() {
        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        let runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        let session = TabSession::new(id, &document, &runtime);

        assert_eq!(session.paste_preference(), PastePreference::PlainText);
    }

    #[test]
    fn mutable_session_changes_only_the_borrowed_runtime() {
        let mut ids = TabIdAllocator::new();
        let first_id = ids.allocate();
        let second_id = ids.allocate();
        let mut first_document = document("first");
        let mut first_runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        let second_runtime = TabRuntime::new(Box::new(EditorPlugin::new()));

        {
            let session = TabSessionMut::new(first_id, &mut first_document, &mut first_runtime);
            session.runtime.toc_visible = true;

            assert_eq!(session.id, first_id);
            assert_eq!(session.document.full_text(), "first");
            assert_ne!(session.id, second_id);
        }
        assert!(first_runtime.toc_visible);
        assert!(!second_runtime.toc_visible);
    }

    #[test]
    fn presentation_scope_keeps_advance_cache_on_error() {
        fn cache_entry(doc_line: usize) -> ui::render_geom::AdvanceCacheEntry {
            ui::render_geom::AdvanceCacheEntry {
                doc_line,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: Vec::new(),
            }
        }

        let id = TabIdAllocator::new().allocate();
        let mut document = document("hello");
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        runtime.presentation.display.advance_cache = vec![cache_entry(2), cache_entry(3)];

        let mut session = TabSessionMut::new(id, &mut document, &mut runtime);
        let result: Result<(), ()> = session.with_presentation(|_, presentation| {
            let cache = &mut presentation.display.advance_cache;
            assert_eq!(cache.len(), 2);
            cache.push(cache_entry(4));
            Err(())
        });

        assert_eq!(result, Err(()));
        assert_eq!(session.display().advance_cache.len(), 3);
    }

    #[test]
    fn cursor_on_wrapped_row_below_viewport_scrolls_into_view() {
        const LINE_HEIGHT_PX: f32 = 20.0;
        const VISIBLE_ROWS: usize = 10;
        const VIEWPORT_HEIGHT_ROWS: f64 = 10.5;
        const CURSOR_VISUAL_ROW: usize = 10;
        const WRAPPED_ROW_COUNT: u16 = 11;

        let id = TabIdAllocator::new().allocate();
        let mut document = document(&"word ".repeat(80));
        document.cursor_move_to_offset(document.buffer_len());
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        runtime.presentation.display.viewport.resize(VISIBLE_ROWS, VIEWPORT_HEIGHT_ROWS);
        runtime.presentation.display.display_map.set_entries(vec![
            crate::snap_tree::DisplayLineEntry::placeholder(
                0,
                document.buffer_len() as u32,
                0,
                WRAPPED_ROW_COUNT,
            ),
        ]);
        runtime.presentation.cursor_render_state.cursor_visual_line = Some(CURSOR_VISUAL_ROW);

        let mut session = TabSessionMut::new(id, &mut document, &mut runtime);
        session.ensure_cursor_visual_row_visible(LINE_HEIGHT_PX);

        assert_eq!(session.scroll_top(), 0.5);
    }

    #[test]
    fn stationary_cursor_below_viewport_does_not_override_manual_scroll() {
        const LINE_HEIGHT_PX: f32 = 20.0;
        const VISIBLE_ROWS: usize = 10;
        const VIEWPORT_HEIGHT_ROWS: f64 = 10.5;
        const CURSOR_VISUAL_ROW: usize = 10;
        const WRAPPED_ROW_COUNT: u16 = 11;

        let id = TabIdAllocator::new().allocate();
        let mut document = document(&"word ".repeat(80));
        document.cursor_move_to_offset(document.buffer_len());
        let cursor_offset = document.cursor().offset;
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        runtime.presentation.display.viewport.resize(VISIBLE_ROWS, VIEWPORT_HEIGHT_ROWS);
        runtime.presentation.display.display_map.set_entries(vec![
            crate::snap_tree::DisplayLineEntry::placeholder(
                0,
                document.buffer_len() as u32,
                0,
                WRAPPED_ROW_COUNT,
            ),
        ]);
        runtime.presentation.cursor_render_state.cursor_visual_line = Some(CURSOR_VISUAL_ROW);
        runtime.presentation.cursor_render_state.last_cursor_offset = cursor_offset;

        let mut session = TabSessionMut::new(id, &mut document, &mut runtime);
        let scroll_changed = session.ensure_cursor_visual_row_visible(LINE_HEIGHT_PX);

        assert!(!scroll_changed, "未移动的光标不能覆盖用户滚动");
        assert_eq!(session.scroll_top(), 0.0);
    }
}
