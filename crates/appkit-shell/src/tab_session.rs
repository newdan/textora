use std::borrow::Cow;

use appkit_core::document::DocumentModel;
use appkit_core::workspace::types::TabId;
use core::document::{DocView, DocViewMut};
use shaping::Shaper;
use ui::canvas::CanvasViewportSnapshot;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::core::widget::{KeyCode, Modifiers};
use ui::plugin::{
    CanvasContentMetrics, CanvasDragRequest, CanvasDragResponse, PluginMessage, PluginQuery,
    PluginResponse, ViewPlugin,
};
use ui::theme::Theme;

use crate::cursor_motion::CursorRenderState;
use crate::display_state::DisplayState;
use crate::document_presentation::DocumentPresentation;
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

struct PresentedDocumentMut<'a> {
    document: &'a mut DocumentModel,
    presentation: &'a mut DocumentPresentation,
}

impl DocView for PresentedDocumentMut<'_> {
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

impl DocViewMut for PresentedDocumentMut<'_> {
    fn set_scroll_y(&mut self, y: f32) {
        self.presentation.display.viewport.scroll_top = y as f64;
    }

    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.document.tb.replace_range(range, text.as_bytes());
    }

    fn begin_edit(&mut self) {
        self.document.tb.edit_begin_grouping();
    }

    fn end_edit(&mut self) {
        self.document.tb.edit_end_grouping();
        self.document.line_index =
            appkit_core::line_index::LineIndex::rebuild_from(&self.document.tb);
        self.document.mark_content_changed();
        self.document.dirty = self.document.tb.is_dirty();
        self.document.sync_cursor_from_buffer();
    }
}

pub struct TabSession<'a> {
    pub id: TabId,
    pub document: &'a DocumentModel,
    pub runtime: &'a TabRuntime,
}

impl std::ops::Deref for TabSession<'_> {
    type Target = DocumentModel;

    fn deref(&self) -> &Self::Target {
        self.document
    }
}

impl<'a> TabSession<'a> {
    pub fn new(id: TabId, document: &'a DocumentModel, runtime: &'a TabRuntime) -> Self {
        Self { id, document, runtime }
    }

    pub fn plugin_name(&self) -> &'a str {
        self.runtime.plugin.name()
    }

    pub fn allows_editing(&self) -> bool {
        self.runtime.editing_access() == crate::tab_runtime::DocumentEditingAccess::Editable
            && self.runtime.plugin.allows_editing()
    }

    pub fn editing_access(&self) -> crate::tab_runtime::DocumentEditingAccess {
        self.runtime.editing_access()
    }

    pub fn handles_own_rendering(&self) -> bool {
        self.runtime.plugin.handles_own_rendering()
    }

    pub fn is_canvas(&self) -> bool {
        self.runtime.plugin.is_canvas()
    }

    pub fn shows_gutter(&self) -> bool {
        self.runtime.plugin.shows_gutter()
    }

    pub fn needs_cursor_blink_wakeup(&self) -> bool {
        self.runtime.plugin.needs_cursor_blink_wakeup()
    }

    pub fn query(&self, query: PluginQuery) -> PluginResponse {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.query(query, &document)
    }

    fn query_float(&self, query: PluginQuery) -> f32 {
        match self.query(query) {
            PluginResponse::Float(value) => value,
            _ => 0.0,
        }
    }

    pub fn query_bool(&self, query: PluginQuery) -> bool {
        match self.query(query) {
            PluginResponse::Bool(value) => value,
            _ => false,
        }
    }

    fn query_position(&self, query: PluginQuery) -> Option<(usize, usize)> {
        match self.query(query) {
            PluginResponse::Position(position) => position,
            _ => None,
        }
    }

    fn query_string(&self, query: PluginQuery) -> String {
        match self.query(query) {
            PluginResponse::String(value) => value,
            _ => String::new(),
        }
    }

    pub fn content_height(&self) -> f32 {
        self.query_float(PluginQuery::ContentHeight)
    }

    pub fn scroll_y(&self) -> f32 {
        self.query_float(PluginQuery::ScrollY)
    }

    pub fn has_selection(&self) -> bool {
        self.query_bool(PluginQuery::HasSelection)
    }

    pub fn selected_text(&self) -> String {
        self.query_string(PluginQuery::SelectedText)
    }

    fn selection_cursor(&self) -> Option<(usize, usize)> {
        self.query_position(PluginQuery::SelCursor)
    }

    pub fn flat_lines(&self) -> Vec<ui::plugin::FlatLine> {
        match self.query(PluginQuery::FlatLines) {
            PluginResponse::FlatLines(lines) => lines,
            _ => Vec::new(),
        }
    }

    pub fn hit_test_position(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<(usize, usize)> {
        self.query_position(PluginQuery::HitTest { x, y, offset_x, offset_y })
    }

    fn word_range_at_pos(
        &self,
        line_index: usize,
        cluster_pos: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        match self.query(PluginQuery::WordAtPos(line_index, cluster_pos)) {
            PluginResponse::PositionPair(range) => range,
            _ => None,
        }
    }

    fn line_range_at_pos(
        &self,
        line_index: usize,
        cluster_pos: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        match self.query(PluginQuery::LineRangeAtPos(line_index, cluster_pos)) {
            PluginResponse::PositionPair(range) => range,
            _ => None,
        }
    }

    pub fn hit_test_byte(&self, x: f32, y: f32, offset_x: f32, offset_y: f32) -> Option<usize> {
        match self.query(PluginQuery::HitTestByte { x, y, offset_x, offset_y }) {
            PluginResponse::BytePosition(byte) => byte,
            _ => None,
        }
    }

    pub fn query_cursor_screen_rect(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        match self.query(PluginQuery::CursorScreenPos(cursor_byte)) {
            PluginResponse::CursorScreenRect(rect) => rect,
            _ => None,
        }
    }

    pub fn hit_test_edit_target(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<Option<ui::plugin::EditHitTarget>> {
        match self.query(PluginQuery::HitTestEditTarget { x, y, offset_x, offset_y }) {
            PluginResponse::EditHitTarget(target) => Some(target),
            _ => None,
        }
    }

    pub fn canvas_control_edit_plan(
        &self,
        source_range: std::ops::Range<usize>,
        source_generation: u32,
    ) -> Option<ui::plugin::EditPlan> {
        match self.query(PluginQuery::PlanCanvasControl { source_range, source_generation }) {
            PluginResponse::EditPlan(plan) => Some(plan),
            _ => None,
        }
    }

    pub fn move_edit_target(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<ui::plugin::EditHitTarget> {
        match self.query(PluginQuery::MoveEditTarget { current_byte, direction, target_x }) {
            PluginResponse::EditHitTarget(Some(target)) => Some(target),
            _ => None,
        }
    }

    pub fn visual_move_byte(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<usize> {
        match self.query(PluginQuery::VisualMove { current_byte, direction, target_x }) {
            PluginResponse::BytePosition(Some(byte)) => Some(byte),
            _ => None,
        }
    }

    pub fn augment_edit(
        &self,
        current_byte: usize,
        kind: ui::plugin::AugmentKind,
    ) -> Option<ui::plugin::EditAugmentation> {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        let context = ui::plugin::AugmentContext { current_byte, kind, doc: &document };
        self.runtime.plugin.augmenter().augment(&context)
    }

    fn plan_mindmap_theme(
        &self,
        theme_id: String,
        source_generation: u32,
    ) -> Option<ui::plugin::EditPlan> {
        match self.query(PluginQuery::PlanMindmapTheme { theme_id, source_generation }) {
            PluginResponse::EditPlan(plan) => Some(plan),
            _ => None,
        }
    }

    pub fn plan_edit_request(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        self.runtime.plugin.edit_policy().plan_edit(request)
    }

    pub fn selection_highlights(&self, color: [f32; 4]) -> DrawList {
        match self.query(PluginQuery::SelectionHighlights(color)) {
            PluginResponse::DrawList(draw_list) => draw_list,
            _ => DrawList::new(),
        }
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
        match self.query(PluginQuery::SearchHighlights {
            query,
            match_case,
            use_regex,
            active_idx,
            match_color,
            inactive_color,
        }) {
            PluginResponse::DrawList(draw_list) => draw_list,
            _ => DrawList::new(),
        }
    }

    pub fn mindmap_theme_selection(&self) -> ui::theme::MindmapThemeSelection {
        match self.query(PluginQuery::MindmapThemeSelection) {
            PluginResponse::MindmapThemeSelection(selection) => selection,
            _ => ui::theme::MindmapThemeSelection::InvalidMetadata,
        }
    }

    fn needs_source_update(&self, generation: u32) -> bool {
        self.query_bool(PluginQuery::NeedsSourceUpdate(generation))
    }

    fn selection_byte_range(&self) -> Option<(usize, usize)> {
        match self.query(PluginQuery::SelectionRange) {
            PluginResponse::PositionPair(Some(((start, _), (end, _)))) => Some((start, end)),
            _ => None,
        }
    }

    pub fn scroll_anchor(&self) -> Option<(String, f32)> {
        match self.query(PluginQuery::ScrollAnchor) {
            PluginResponse::ScrollAnchor(text, offset) => Some((text, offset)),
            _ => None,
        }
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

    pub fn search_state(&self) -> &appkit_core::document::SearchState {
        &self.presentation().search_state
    }

    pub fn cursor_render_state(&self) -> &CursorRenderState {
        &self.presentation().cursor_render_state
    }

    pub fn mindmap_style_panel(&self) -> crate::mindmap_style_panel::MindmapStylePanelSession {
        self.runtime.mindmap_style_panel
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
            .key_intent_mapper()
            .and_then(|mapper| mapper.map_key(key_code, modifiers))
    }
}

pub struct TabSessionMut<'a> {
    pub id: TabId,
    pub document: &'a mut DocumentModel,
    pub runtime: &'a mut TabRuntime,
}

impl std::ops::Deref for TabSessionMut<'_> {
    type Target = DocumentModel;

    fn deref(&self) -> &Self::Target {
        self.document
    }
}

impl std::ops::DerefMut for TabSessionMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.document
    }
}

impl<'a> TabSessionMut<'a> {
    pub fn new(id: TabId, document: &'a mut DocumentModel, runtime: &'a mut TabRuntime) -> Self {
        Self { id, document, runtime }
    }

    fn as_ref(&self) -> TabSession<'_> {
        TabSession::new(self.id, self.document, self.runtime)
    }

    pub fn plugin_name(&self) -> &str {
        self.runtime.plugin.name()
    }

    pub fn allows_editing(&self) -> bool {
        self.as_ref().allows_editing()
    }

    pub fn is_canvas(&self) -> bool {
        self.runtime.plugin.is_canvas()
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

    pub fn content_height(&self) -> f32 {
        self.as_ref().content_height()
    }

    pub fn scroll_y(&self) -> f32 {
        self.as_ref().scroll_y()
    }

    pub fn has_selection(&self) -> bool {
        self.as_ref().has_selection()
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

    pub fn plan_mindmap_theme(
        &self,
        theme_id: String,
        source_generation: u32,
    ) -> Option<ui::plugin::EditPlan> {
        self.as_ref().plan_mindmap_theme(theme_id, source_generation)
    }

    fn presentation(&self) -> &DocumentPresentation {
        &self.runtime.presentation
    }

    fn presentation_mut(&mut self) -> &mut DocumentPresentation {
        &mut self.runtime.presentation
    }

    pub fn take_presentation(&mut self) -> DocumentPresentation {
        let visible_rows = self.runtime.presentation.display.viewport.visible_rows;
        let viewport_height = self.runtime.presentation.display.viewport.viewport_height;
        std::mem::replace(
            &mut self.runtime.presentation,
            DocumentPresentation::new(visible_rows, viewport_height),
        )
    }

    pub fn restore_presentation(&mut self, presentation: DocumentPresentation) {
        self.runtime.presentation = presentation;
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

    pub fn mindmap_style_panel(&self) -> crate::mindmap_style_panel::MindmapStylePanelSession {
        self.runtime.mindmap_style_panel
    }

    pub fn toggle_mindmap_style_panel(&mut self) {
        self.runtime.mindmap_style_panel.toggle_visibility();
    }

    pub fn close_mindmap_style_panel(&mut self) {
        self.runtime.mindmap_style_panel.close();
    }

    pub fn toggle_mindmap_style_presets(&mut self) {
        self.runtime.mindmap_style_panel.toggle_presets();
    }

    pub fn display_map_clone(&self) -> crate::display_line_map::DisplayLineMap {
        self.display().display_map.clone()
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

    pub fn last_cursor_offset(&self) -> core::types::ByteIndex {
        self.cursor_render_state().last_cursor_offset
    }

    pub fn set_last_cursor_offset(&mut self, offset: core::types::ByteIndex) {
        self.cursor_render_state_mut().last_cursor_offset = offset;
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
        let display_map = self.display_map_clone();
        self.display_mut().viewport.set_scroll_top(scroll_top, &display_map, line_height);
        self.display_mut().viewport.derive_scroll_top(&display_map, line_height);
    }

    pub fn scroll_viewport_by_pages(&mut self, amount: f64, line_height: f32) {
        let page_pixels = self.visible_rows().max(1) as f32 * line_height;
        let pixels = if amount > 0.0 { page_pixels } else { -page_pixels };
        self.scroll_viewport_by_pixels(pixels, line_height);
    }

    pub fn scroll_viewport_by_pixels(&mut self, pixels: f32, line_height: f32) {
        let display_map = self.display_map_clone();
        self.display_mut().viewport.scroll_pixels(pixels, &display_map, line_height);
        self.refresh_scroll_metrics(line_height);
    }

    pub fn clamp_scroll_anchor(&mut self, line_height: f32) {
        let display_map = self.display_map_clone();
        self.display_mut().viewport.clamp_anchor(&display_map, line_height);
    }

    pub fn derive_scroll_top(&mut self, line_height: f32) {
        let display_map = self.display_map_clone();
        self.display_mut().viewport.derive_scroll_top(&display_map, line_height);
    }

    pub fn visible_doc_range_from_anchor(&self, line_height: f32) -> std::ops::Range<usize> {
        self.display()
            .viewport
            .visible_doc_range_from_anchor(&self.display().display_map, line_height)
    }

    pub fn take_advance_cache(&mut self) -> Vec<ui::render_geom::AdvanceCacheEntry> {
        std::mem::take(&mut self.runtime.presentation.display.advance_cache)
    }

    pub fn restore_advance_cache(
        &mut self,
        advance_cache: Vec<ui::render_geom::AdvanceCacheEntry>,
    ) {
        self.runtime.presentation.display.advance_cache = advance_cache;
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

        let display_map = self.display_map_clone();
        self.cursor_render_state_mut().click_hint = None;
        self.display_mut().viewport.scroll_pixels(overflow_px, &display_map, line_height);
        self.refresh_scroll_metrics(line_height);
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

    pub fn move_cursor_visual(
        &mut self,
        delta: isize,
        context: crate::cursor_motion::CursorContext<'_>,
    ) {
        if let Some(offset) =
            crate::cursor_motion::move_cursor_visual(delta, context, self.document)
        {
            self.document.set_cursor_offset_synced(offset.to_usize());
        }
        self.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        self.as_ref().needs_source_update(generation)
    }

    pub fn selection_byte_range(&self) -> Option<(usize, usize)> {
        self.as_ref().selection_byte_range()
    }

    pub fn send_message(&mut self, message: PluginMessage) -> bool {
        let mut document = PresentedDocumentMut {
            document: self.document,
            presentation: &mut self.runtime.presentation,
        };
        self.runtime.plugin.handle_message(message, &mut document)
    }

    pub fn toggle_toc_visible(&mut self) {
        self.runtime.toc_visible = !self.runtime.toc_visible;
    }

    pub fn swap_in_toggle_plugin(&mut self, replacement: Box<dyn ViewPlugin>) {
        self.runtime.toggle_source_scroll_y = self.scroll_y();
        self.runtime.cached_toggle_source =
            Some(std::mem::replace(&mut self.runtime.plugin, replacement));
    }

    pub fn restore_cached_toggle_source(&mut self) -> bool {
        let Some(mut cached) = self.runtime.cached_toggle_source.take() else {
            return false;
        };
        let scroll_y = self.runtime.toggle_source_scroll_y;
        let mut document = PresentedDocumentMut {
            document: self.document,
            presentation: &mut self.runtime.presentation,
        };
        cached.handle_message(PluginMessage::SetScrollY(scroll_y), &mut document);
        self.runtime.plugin = cached;
        true
    }

    pub fn cache_toggle_source_plugin(&mut self, plugin: Box<dyn ViewPlugin>) {
        self.runtime.cached_toggle_source = Some(plugin);
    }

    pub fn replace_plugin(&mut self, plugin: Box<dyn ViewPlugin>) {
        self.runtime.plugin = plugin;
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
        self.runtime.plugin.prepare_canvas(&document, theme, shaper, dpi_scale)
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
        self.runtime.plugin.render_canvas(&document, snapshot, theme, shaper, dpi_scale)
    }

    pub fn handle_canvas_drag_plugin(&mut self, request: CanvasDragRequest) -> CanvasDragResponse {
        let document =
            PresentedDocument { document: self.document, presentation: &self.runtime.presentation };
        self.runtime.plugin.handle_canvas_drag(request, &document)
    }

    pub fn intercept_key(&mut self, key_code: &KeyCode, modifiers: &Modifiers) -> bool {
        let Some(interceptor) = self.runtime.plugin.key_interceptor() else {
            return false;
        };
        let mut document = PresentedDocumentMut {
            document: self.document,
            presentation: &mut self.runtime.presentation,
        };
        interceptor.intercept_key(key_code, modifiers, &mut document)
    }
}

#[cfg(test)]
mod tests {
    use appkit_core::document::DocumentModel;
    use appkit_core::workspace::types::TabIdAllocator;
    use core::buffer::TextBuffer;
    use core::document::DocView;
    use ui::plugin::{PluginQuery, PluginResponse, ViewPlugin};

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

        fn query(&self, query: PluginQuery, document: &dyn DocView) -> PluginResponse {
            match query {
                PluginQuery::ScrollY => PluginResponse::Float(document.scroll_y()),
                _ => PluginResponse::None,
            }
        }
    }

    #[test]
    fn immutable_session_preserves_tab_identity_and_borrows_state() {
        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        let runtime = TabRuntime::new(Box::new(EditorPlugin::new()));

        let session = TabSession::new(id, &document, &runtime);

        assert_eq!(session.id, id);
        assert_eq!(session.document.full_text(), "hello");
        assert_eq!(session.runtime.plugin.name(), ui::plugin::PLUGIN_EDITOR);
    }

    #[test]
    fn plugin_queries_receive_runtime_presentation_viewport() {
        const RUNTIME_SCROLL: f64 = 17.0;

        let id = TabIdAllocator::new().allocate();
        let document = document("hello");
        let mut runtime = TabRuntime::new(Box::new(ScrollProbePlugin));
        runtime.presentation.display.viewport.scroll_top = RUNTIME_SCROLL;

        let session = TabSession::new(id, &document, &runtime);

        assert_eq!(session.query_float(PluginQuery::ScrollY), RUNTIME_SCROLL as f32);
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
    fn mutable_session_takes_advance_cache_from_runtime_presentation() {
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

        {
            let mut session = TabSessionMut::new(id, &mut document, &mut runtime);
            let cache = session.take_advance_cache();

            assert_eq!(cache.len(), 2);
            assert!(session.runtime.presentation.display.advance_cache.is_empty());
        }
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

    #[test]
    fn style_panel_state_is_owned_and_mutated_through_runtime_session() {
        let mut ids = TabIdAllocator::new();
        let id = ids.allocate();
        let mut document = document("# Root");
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        let mut session = TabSessionMut::new(id, &mut document, &mut runtime);

        assert_eq!(
            session.mindmap_style_panel(),
            crate::mindmap_style_panel::MindmapStylePanelSession::Closed
        );

        session.toggle_mindmap_style_panel();
        assert_eq!(
            session.mindmap_style_panel(),
            crate::mindmap_style_panel::MindmapStylePanelSession::Open { presets_expanded: true }
        );

        session.toggle_mindmap_style_presets();
        assert_eq!(
            session.mindmap_style_panel(),
            crate::mindmap_style_panel::MindmapStylePanelSession::Open { presets_expanded: false }
        );

        session.close_mindmap_style_panel();
        assert_eq!(
            session.mindmap_style_panel(),
            crate::mindmap_style_panel::MindmapStylePanelSession::Closed
        );
    }
}
