//! 产品无关的编辑器指针命中与选择会话。

use ui::canvas::CanvasPoint;
use ui::core::widget::PointerClickKind;
use ui::plugin::{CanvasDragPhase, CanvasDragRequest, CanvasDragResponse, EditHitTarget, EditPlan};

use super::input_session::TextSelectionGranularity;
use super::{EditorInputContext, EditorOutcome, EditorPointerOutcome, EditorRuntime, MouseCapture};
use crate::cursor_motion::{hit_test_row_byte, hit_test_visual_row};
use crate::event::ShellEffect;
use crate::pointer_capture::{CanvasDragEligibility, CanvasDragSession};
use crate::tab_session::{TabSession, TabSessionMut};

const CANVAS_DRAG_THRESHOLD_PX: f32 = 5.0;

impl EditorRuntime {
    pub fn handle_pointer_event(
        &mut self,
        context: EditorInputContext,
        event: &ui::Event,
    ) -> EditorPointerOutcome {
        let Some((position, phase)) = pointer_event(event) else {
            return EditorPointerOutcome::default();
        };
        let Some(editor_bounds) = self.painted_editor_bounds() else {
            if phase == PointerPhase::Release {
                self.end_pointer_capture();
            }
            let editor =
                self.clear_active_canvas_pointer().then(redraw_outcome).unwrap_or_default();
            return EditorPointerOutcome::from_editor(editor, None);
        };
        let canvas_pointer_outcome = match phase {
            PointerPhase::Move
                if self.update_active_canvas_pointer(context, editor_bounds, position) =>
            {
                redraw_outcome()
            }
            PointerPhase::Press | PointerPhase::Move | PointerPhase::Release => {
                EditorOutcome::default()
            }
        };
        let editor = match phase {
            PointerPhase::Press if self.pointer_input_allowed(context, position) => {
                if let Some(outcome) = self.activate_canvas_control(position) {
                    self.pointer_click_tracker.reset();
                    let cursor_icon = self.pointer_cursor_icon(context, editor_bounds, position);
                    return EditorPointerOutcome::from_editor(outcome, cursor_icon);
                }
                if self.begin_canvas_source_drag(context, editor_bounds, position) {
                    self.pointer_click_tracker.reset();
                    let cursor_icon = self.pointer_cursor_icon(context, editor_bounds, position);
                    return EditorPointerOutcome::from_editor(redraw_outcome(), cursor_icon);
                }
                let click_kind = self.pointer_click_tracker.record_press(position);
                let granularity = selection_granularity(click_kind);
                let extend = self.input_modifiers().shift_key();
                let selection_started =
                    self.begin_text_selection_with_granularity(context, granularity);
                let selection_placed = selection_started
                    && self.place_pointer_selection(
                        editor_bounds,
                        position,
                        extend,
                        click_kind,
                        None,
                    );
                if !selection_placed {
                    self.pointer_click_tracker.reset();
                    self.end_pointer_capture();
                    let cursor_icon = self.pointer_cursor_icon(context, editor_bounds, position);
                    return EditorPointerOutcome::from_editor(
                        EditorOutcome::default(),
                        cursor_icon,
                    );
                }
                if let Some(initial_range) = self.current_text_selection_range() {
                    self.set_initial_text_selection_range(initial_range);
                }
                redraw_outcome()
            }
            PointerPhase::Move
                if self.pointer_capture() == MouseCapture::CanvasDrag
                    && self.pointer_input_allowed(context, position) =>
            {
                canvas_pointer_outcome
                    .merge(self.update_canvas_source_drag(editor_bounds, position))
            }
            PointerPhase::Move
                if self.pointer_capture() == MouseCapture::TextSelection
                    && self.pointer_input_allowed(context, position) =>
            {
                let click_kind = self
                    .text_selection_granularity()
                    .map(pointer_click_kind)
                    .unwrap_or(PointerClickKind::Single);
                let initial_range = self.initial_text_selection_range();
                let selection_outcome = self
                    .place_pointer_selection(
                        editor_bounds,
                        position,
                        true,
                        click_kind,
                        initial_range.as_ref(),
                    )
                    .then(redraw_outcome)
                    .unwrap_or_default();
                canvas_pointer_outcome.merge(selection_outcome)
            }
            PointerPhase::Release if self.pointer_capture() == MouseCapture::CanvasDrag => {
                self.finish_canvas_source_drag(editor_bounds, position)
            }
            PointerPhase::Release if self.pointer_capture() != MouseCapture::None => {
                self.end_pointer_capture();
                self.clear_collapsed_selection();
                redraw_outcome()
            }
            PointerPhase::Move => canvas_pointer_outcome,
            PointerPhase::Press => {
                self.pointer_click_tracker.reset();
                EditorOutcome::default()
            }
            PointerPhase::Release => EditorOutcome::default(),
        };
        let cursor_icon = self.pointer_cursor_icon(context, editor_bounds, position);
        EditorPointerOutcome::from_editor(editor, cursor_icon)
    }

    fn pointer_cursor_icon(
        &self,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> Option<winit::window::CursorIcon> {
        if context.modal_blocked
            || (self.pointer_capture() == MouseCapture::None
                && !editor_bounds.contains(position.0, position.1))
        {
            return None;
        }
        if self.pointer_capture() == MouseCapture::CanvasDrag {
            return Some(winit::window::CursorIcon::Grabbing);
        }

        let Some(tab_id) = self.active_tab_id() else {
            return Some(winit::window::CursorIcon::Default);
        };
        let Some(tab) = self.tab_session(tab_id) else {
            return Some(winit::window::CursorIcon::Default);
        };
        if !tab.handles_own_rendering() {
            return Some(winit::window::CursorIcon::Text);
        }
        if !tab.allows_editing() {
            return Some(winit::window::CursorIcon::Default);
        }

        if tab.hit_test_canvas_control(CanvasPoint::new(position.0, position.1)).is_some() {
            return Some(winit::window::CursorIcon::Pointer);
        }
        let dpi = self.scale_factor() as f32;
        let bounds = super::editor_painter::plugin_bounds(editor_bounds, dpi, tab.is_canvas());
        let target = tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y);
        Some(match target {
            Some(Some(EditHitTarget::TextCaret { .. })) => winit::window::CursorIcon::Text,
            Some(Some(EditHitTarget::SourceObject { .. })) => winit::window::CursorIcon::Grab,
            Some(Some(EditHitTarget::ClearFocus)) | Some(None) | None => {
                winit::window::CursorIcon::Default
            }
        })
    }

    fn update_active_canvas_pointer(
        &mut self,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> bool {
        let pointer_is_visible = !context.modal_blocked
            && (self.pointer_capture() != MouseCapture::None
                || editor_bounds.contains(position.0, position.1));
        let pointer =
            pointer_is_visible.then(|| ui::canvas::CanvasPoint::new(position.0, position.1));
        self.set_active_canvas_pointer(pointer)
    }

    fn set_active_canvas_pointer(&mut self, pointer: Option<ui::canvas::CanvasPoint>) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        if !tab.is_canvas() || !tab.handles_own_rendering() {
            return false;
        }
        tab.set_canvas_pointer(pointer)
    }

    pub(crate) fn clear_active_canvas_pointer(&mut self) -> bool {
        self.set_active_canvas_pointer(None)
    }

    fn activate_canvas_control(&mut self, position: (f32, f32)) -> Option<EditorOutcome> {
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        let source_range = tab.hit_test_canvas_control(CanvasPoint::new(position.0, position.1))?;
        Some(self.handle_canvas_control(source_range))
    }

    /// Apply the control identified by a product's semantic pointer hit test.
    pub fn handle_canvas_control(&mut self, source_range: std::ops::Range<usize>) -> EditorOutcome {
        self.end_pointer_capture();
        let plan = self.active_tab_id().and_then(|tab_id| {
            let tab = self.tab_session(tab_id)?;
            tab.canvas_control_edit_plan(source_range, tab.document.generation())
        });
        let Some(EditPlan::Apply(transaction)) = plan else {
            return redraw_outcome();
        };
        self.model_session
            .apply_active_edit_transaction(transaction, self.editor_line_height())
            .merge(redraw_outcome())
    }

    fn begin_canvas_source_drag(
        &mut self,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let dpi = self.scale_factor() as f32;
        let (source_range, source_generation) = {
            let Some(tab) = self.tab_session_mut(tab_id) else {
                return false;
            };
            if !tab.is_canvas() || !tab.handles_own_rendering() || !tab.allows_editing() {
                return false;
            }
            let bounds = super::editor_painter::plugin_bounds(editor_bounds, dpi, true);
            let Some(Some(EditHitTarget::SourceObject { source_range })) =
                tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y)
            else {
                return false;
            };
            let source_generation = tab.document.generation();
            // Pressing a source object moves the caret: split any ongoing
            // undo coalescing run.
            tab.document.break_edit_merge();
            tab.document.cursor_mut().selection_anchor = Some(source_range.start);
            tab.document.set_cursor_offset_synced(source_range.end);
            (source_range, source_generation)
        };

        self.input_session.start_canvas_drag_session(
            context,
            CanvasDragSession {
                source_range,
                pressed_at: position,
                source_generation,
                eligibility: CanvasDragEligibility::Enabled,
                started: false,
            },
        )
    }

    fn update_canvas_source_drag(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
    ) -> EditorOutcome {
        let bounds =
            super::editor_painter::plugin_bounds(editor_rect, self.scale_factor() as f32, true);
        let request = self
            .input_session
            .canvas_drag_session_mut()
            .and_then(|session| session.move_request(position, bounds));
        request.map(|request| self.handle_canvas_drag_request(request)).unwrap_or_default()
    }

    fn finish_canvas_source_drag(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
    ) -> EditorOutcome {
        let session = self.input_session.take_canvas_drag_session();
        self.end_pointer_capture();
        let bounds =
            super::editor_painter::plugin_bounds(editor_rect, self.scale_factor() as f32, true);
        let Some(request) = session.and_then(|session| session.release_request(position, bounds))
        else {
            return redraw_outcome();
        };
        self.handle_canvas_drag_request(request)
    }

    /// Route a captured drag to the active plugin and apply edits on drop.
    pub fn handle_canvas_drag_request(&mut self, request: CanvasDragRequest) -> EditorOutcome {
        let phase = request.phase;
        let response = self
            .active_tab_id()
            .and_then(|tab_id| self.tab_session_mut(tab_id))
            .map(|mut tab| tab.handle_canvas_drag_plugin(request))
            .unwrap_or(CanvasDragResponse::Ignore);
        match (phase, response) {
            (CanvasDragPhase::Drop, CanvasDragResponse::Apply(transaction)) => self
                .model_session
                .apply_active_edit_transaction(transaction, self.editor_line_height())
                .merge(redraw_outcome()),
            (CanvasDragPhase::Drop | CanvasDragPhase::Cancel, _)
            | (_, CanvasDragResponse::Preview(_)) => redraw_outcome(),
            _ => EditorOutcome::default(),
        }
    }

    fn place_pointer_selection(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
        extend: bool,
        click_kind: PointerClickKind,
        initial_range: Option<&std::ops::Range<usize>>,
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        // A pointer gesture is user-driven caret movement: split any ongoing
        // undo coalescing run even if the caret lands back on the byte where
        // the last edit ended.
        if let Some(tab) = self.tab_session_mut(tab_id) {
            tab.document.break_edit_merge();
        }
        let dpi = self.scale_factor() as f32;
        let settings = self.settings.clone();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
        let handles_own_rendering =
            self.tab_session(tab_id).is_some_and(|tab| tab.handles_own_rendering());
        if handles_own_rendering {
            return self.place_plugin_pointer_selection(
                tab_id,
                editor_rect,
                position,
                dpi,
                extend,
                click_kind,
                initial_range,
            );
        }
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        let byte_offset = hit_test_text_byte(&tab, editor_rect, position, &settings, &metrics)
            .unwrap_or_else(|| tab.document.buffer_len());
        place_text_pointer_selection(&mut tab, byte_offset, extend, click_kind, initial_range);
        true
    }

    fn place_plugin_pointer_selection(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        editor_rect: ui::Rect,
        position: (f32, f32),
        dpi: f32,
        extend: bool,
        click_kind: PointerClickKind,
        initial_range: Option<&std::ops::Range<usize>>,
    ) -> bool {
        let bounds = {
            let Some(tab) = self.tab_session(tab_id) else {
                return false;
            };
            super::editor_painter::plugin_bounds(editor_rect, dpi, tab.is_canvas())
        };
        let edit_target = {
            let Some(tab) = self.tab_session(tab_id) else {
                return false;
            };
            if tab.hit_test_canvas_control(CanvasPoint::new(position.0, position.1)).is_some() {
                return false;
            }
            tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y)
        };
        match edit_target {
            Some(Some(EditHitTarget::TextCaret { byte_offset, .. })) => {
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                place_text_pointer_selection(
                    &mut tab,
                    byte_offset,
                    extend,
                    click_kind,
                    initial_range,
                );
                let cursor_byte = tab.document.cursor_offset().to_usize();
                tab.set_cursor_byte(cursor_byte);
                true
            }
            Some(Some(EditHitTarget::SourceObject { source_range })) if !extend => {
                let Some(tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                tab.document.cursor_mut().selection_anchor = Some(source_range.start);
                tab.document.set_cursor_offset_synced(source_range.end);
                true
            }
            Some(Some(EditHitTarget::ClearFocus)) if !extend => {
                let Some(tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                tab.document.cursor_mut().selection_anchor = None;
                tab.document.set_cursor_offset_synced(tab.document.buffer_len());
                true
            }
            Some(None) => false,
            Some(Some(EditHitTarget::SourceObject { .. } | EditHitTarget::ClearFocus)) => false,
            None => self.place_plugin_byte_selection(
                tab_id,
                bounds,
                position,
                extend,
                click_kind,
                initial_range,
            ),
        }
    }

    fn place_plugin_byte_selection(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        bounds: ui::Rect,
        position: (f32, f32),
        extend: bool,
        click_kind: PointerClickKind,
        initial_range: Option<&std::ops::Range<usize>>,
    ) -> bool {
        let candidate = {
            let Some(tab) = self.tab_session(tab_id) else {
                return false;
            };
            tab.hit_test_plugin_byte_or_end(position, bounds)
        };
        let Some(candidate) = candidate else {
            return false;
        };
        let snapped_candidate = {
            let Some(mut tab) = self.tab_session_mut(tab_id) else {
                return false;
            };
            if extend {
                place_text_pointer_selection(&mut tab, candidate, true, click_kind, initial_range);
            } else {
                place_text_caret(&mut tab, candidate, false);
            }
            let snapped_candidate = tab.document.cursor_offset().to_usize();
            tab.set_cursor_byte(snapped_candidate);
            snapped_candidate
        };
        if extend {
            return true;
        }

        self.refresh_plugin_layout_for_pointer_hit(tab_id, bounds);
        let final_byte = {
            let Some(tab) = self.tab_session(tab_id) else {
                return false;
            };
            tab.hit_test_expanded_plugin_byte(snapped_candidate, position, bounds)
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        place_text_pointer_selection(&mut tab, final_byte, extend, click_kind, initial_range);
        let snapped_final = tab.document.cursor_offset().to_usize();
        tab.set_cursor_byte(snapped_final);
        self.input_session.set_preferred_x(None);
        true
    }

    fn refresh_plugin_layout_for_pointer_hit(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        bounds: ui::Rect,
    ) {
        let metrics =
            ui::settings::UiMetrics::from_settings(&self.settings, self.scale_factor() as f32);
        let Some(mut shaper) = self.new_shaper(metrics.font_size, &self.settings.font_family)
        else {
            return;
        };
        let theme = self.theme.clone();
        let dpi = metrics.dpi;
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        let _ = tab.render_plugin(bounds, &theme, &mut shaper, dpi);
    }

    fn clear_collapsed_selection(&mut self) {
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        tab.clear_collapsed_pointer_selection();
    }

    fn current_text_selection_range(&self) -> Option<std::ops::Range<usize>> {
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        let selection_anchor = tab.document.cursor().selection_anchor?;
        let cursor_offset = tab.document.cursor_offset().to_usize();
        Some(selection_anchor.min(cursor_offset)..selection_anchor.max(cursor_offset))
    }
}

impl CanvasDragSession {
    pub fn move_request(
        &mut self,
        position: (f32, f32),
        bounds: ui::Rect,
    ) -> Option<CanvasDragRequest> {
        if self.eligibility == CanvasDragEligibility::Disabled {
            return None;
        }
        if self.started {
            return Some(self.request(CanvasDragPhase::Update, position, bounds));
        }

        let horizontal_distance = position.0 - self.pressed_at.0;
        let vertical_distance = position.1 - self.pressed_at.1;
        let distance_squared =
            horizontal_distance * horizontal_distance + vertical_distance * vertical_distance;
        if distance_squared > CANVAS_DRAG_THRESHOLD_PX * CANVAS_DRAG_THRESHOLD_PX {
            self.started = true;
            return Some(self.request(CanvasDragPhase::Start, position, bounds));
        }
        None
    }

    pub fn release_request(
        &self,
        position: (f32, f32),
        bounds: ui::Rect,
    ) -> Option<CanvasDragRequest> {
        self.started.then(|| self.request(CanvasDragPhase::Drop, position, bounds))
    }

    pub fn cancel_request(&self, bounds: ui::Rect) -> Option<CanvasDragRequest> {
        self.started.then(|| self.request(CanvasDragPhase::Cancel, self.pressed_at, bounds))
    }

    fn request(
        &self,
        phase: CanvasDragPhase,
        position: (f32, f32),
        bounds: ui::Rect,
    ) -> CanvasDragRequest {
        CanvasDragRequest {
            phase,
            source_range: self.source_range.clone(),
            pointer_x: position.0,
            pointer_y: position.1,
            pressed_x: self.pressed_at.0,
            pressed_y: self.pressed_at.1,
            offset_x: bounds.x,
            offset_y: bounds.y,
            source_generation: self.source_generation,
        }
    }
}

impl TabSession<'_> {
    pub fn hit_test_plugin_byte_or_end(
        &self,
        position: (f32, f32),
        bounds: ui::Rect,
    ) -> Option<usize> {
        self.hit_test_byte(position.0, position.1, bounds.x, bounds.y).or_else(|| {
            (position.1 - bounds.y > self.content_height()).then(|| self.document.buffer_len())
        })
    }

    pub fn hit_test_expanded_plugin_byte(
        &self,
        candidate: usize,
        position: (f32, f32),
        bounds: ui::Rect,
    ) -> usize {
        let expanded_position = expanded_plugin_hit_position(
            position,
            self.query_cursor_screen_rect(candidate),
            bounds,
        );
        self.hit_test_byte(expanded_position.0, expanded_position.1, bounds.x, bounds.y)
            .unwrap_or(candidate)
    }
}

impl TabSessionMut<'_> {
    pub fn clear_collapsed_pointer_selection(&mut self) -> bool {
        let cursor = self.document.cursor();
        if cursor.selection_anchor != Some(cursor.offset.to_usize()) {
            return false;
        }
        self.document.cursor_mut().selection_anchor = None;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerPhase {
    Press,
    Move,
    Release,
}

fn selection_granularity(click_kind: PointerClickKind) -> TextSelectionGranularity {
    match click_kind {
        PointerClickKind::Single => TextSelectionGranularity::Grapheme,
        PointerClickKind::Double => TextSelectionGranularity::Word,
        PointerClickKind::Triple => TextSelectionGranularity::SourceLine,
    }
}

fn pointer_click_kind(granularity: TextSelectionGranularity) -> PointerClickKind {
    match granularity {
        TextSelectionGranularity::Grapheme => PointerClickKind::Single,
        TextSelectionGranularity::Word => PointerClickKind::Double,
        TextSelectionGranularity::SourceLine => PointerClickKind::Triple,
    }
}

fn pointer_event(event: &ui::Event) -> Option<((f32, f32), PointerPhase)> {
    match event {
        ui::Event::MouseDown { px, py, button: ui::MouseButton::Left } => {
            Some(((*px, *py), PointerPhase::Press))
        }
        ui::Event::MouseMove { px, py } => Some(((*px, *py), PointerPhase::Move)),
        ui::Event::MouseUp { px, py, button: ui::MouseButton::Left } => {
            Some(((*px, *py), PointerPhase::Release))
        }
        _ => None,
    }
}

fn expanded_plugin_hit_position(
    pointer_position: (f32, f32),
    cursor_rect: Option<(f32, f32, f32, f32)>,
    bounds: ui::Rect,
) -> (f32, f32) {
    let Some((cursor_x, cursor_y, _cursor_width, cursor_height)) = cursor_rect else {
        return pointer_position;
    };
    (bounds.x + cursor_x, bounds.y + cursor_y + cursor_height * 0.5)
}

fn place_text_caret(tab: &mut TabSessionMut<'_>, byte_offset: usize, extend: bool) {
    let byte_offset = byte_offset.min(tab.document.buffer_len());
    if !extend {
        tab.document.cursor_mut().selection_anchor = Some(byte_offset);
    }
    tab.document.set_cursor_offset_synced(byte_offset);
    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
}

fn place_text_pointer_selection(
    tab: &mut TabSessionMut<'_>,
    byte_offset: usize,
    extend: bool,
    click_kind: PointerClickKind,
    initial_range: Option<&std::ops::Range<usize>>,
) {
    let byte_offset = byte_offset.min(tab.document.buffer_len());
    if extend {
        tab.document.ensure_selection_active();
    }
    match click_kind {
        PointerClickKind::Single => place_text_caret(tab, byte_offset, extend),
        PointerClickKind::Double => {
            let (selection_start, selection_end) = tab.document.word_select_at(byte_offset);
            place_text_range_selection(
                tab,
                selection_start,
                selection_end,
                extend,
                byte_offset,
                initial_range,
            );
        }
        PointerClickKind::Triple => {
            tab.document.set_cursor_offset_synced(byte_offset);
            let document_line = tab.document.cursor_line();
            let range = tab
                .document
                .source_line_selection_range(document_line)
                .unwrap_or_else(|| 0..tab.document.buffer_len());
            place_text_range_selection(
                tab,
                range.start,
                range.end,
                extend,
                byte_offset,
                initial_range,
            );
        }
    }
    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
}

fn place_text_range_selection(
    tab: &mut TabSessionMut<'_>,
    selection_start: usize,
    selection_end: usize,
    extend: bool,
    hit_byte: usize,
    initial_range: Option<&std::ops::Range<usize>>,
) {
    if !extend {
        tab.document.cursor_mut().selection_anchor = Some(selection_start);
        tab.document.set_cursor_offset_synced(selection_end);
        return;
    }
    if let Some(initial_range) = initial_range {
        place_grouped_drag_selection(tab, selection_start..selection_end, initial_range);
        return;
    }
    let anchor = tab
        .document
        .cursor()
        .selection_anchor
        .unwrap_or_else(|| tab.document.cursor_offset().to_usize());
    if hit_byte >= anchor {
        tab.document.set_cursor_offset_synced(selection_end);
    } else {
        tab.document.set_cursor_offset_synced(selection_start);
    }
}

fn place_grouped_drag_selection(
    tab: &mut TabSessionMut<'_>,
    current_range: std::ops::Range<usize>,
    initial_range: &std::ops::Range<usize>,
) {
    let (selection_anchor, cursor_offset) = if current_range.end <= initial_range.start {
        (initial_range.end, current_range.start)
    } else if current_range.start >= initial_range.end {
        (initial_range.start, current_range.end)
    } else {
        (initial_range.start, initial_range.end)
    };
    tab.document.cursor_mut().selection_anchor = Some(selection_anchor);
    tab.document.set_cursor_offset_synced(cursor_offset);
}

fn hit_test_text_byte(
    tab: &TabSessionMut<'_>,
    editor_rect: ui::Rect,
    position: (f32, f32),
    settings: &ui::settings::Settings,
    metrics: &ui::settings::UiMetrics,
) -> Option<usize> {
    let display = tab.display();
    let sub_line_offset = display.viewport.sub_line_pixel_offset(metrics.line_height);
    let (_, entry) = hit_test_visual_row(
        position.1,
        editor_rect.y,
        sub_line_offset,
        metrics.line_height,
        &display.advance_cache,
    )?;
    let gutter_width = metrics
        .content_left_margin
        .max(settings.gutter_width(tab.document.line_count()) * metrics.dpi);
    let line_byte = hit_test_row_byte(position.0, editor_rect.x + gutter_width, entry);
    tab.document.line_byte_offset(entry.doc_line).map(|start| start + line_byte)
}

fn redraw_outcome() -> EditorOutcome {
    EditorOutcome { shell_effect: ShellEffect::REDRAW, ..EditorOutcome::default() }
}

#[cfg(test)]
mod tests {
    use super::expanded_plugin_hit_position;

    #[test]
    fn expanded_plugin_hit_position_uses_cursor_rect_when_available() {
        let bounds = ui::Rect::new(10.0, 30.0, 400.0, 500.0);
        let hit_position =
            expanded_plugin_hit_position((40.0, 12.0), Some((120.0, 24.0, 2.0, 18.0)), bounds);

        assert_eq!(hit_position, (130.0, 63.0));
    }

    #[test]
    fn expanded_plugin_hit_position_uses_pointer_when_cursor_rect_is_unavailable() {
        let bounds = ui::Rect::new(10.0, 30.0, 400.0, 500.0);
        let hit_position = expanded_plugin_hit_position((40.0, 12.0), None, bounds);

        assert_eq!(hit_position, (40.0, 12.0));
    }
}
