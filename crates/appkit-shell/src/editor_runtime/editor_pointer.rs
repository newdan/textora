//! 产品无关的编辑器指针命中与选择会话。

use appkit_core::tab_set::types::TabId;
use ui::canvas::CanvasPoint;
use ui::core::widget::{PointerClickKind, PointerClickTiming, PointerClickTracker};
use ui::plugin::{
    CanvasDragPhase, CanvasDragRequest, CanvasDragResponse, CursorAffinity, EditHitTarget,
    EditPlan, PluginMode, VisualCaretPosition,
};

use super::editor_navigation::validated_source_object_range;
use super::input_session::{CapturedCanvasDrag, TextSelectionGranularity};
use super::{EditorInputContext, EditorOutcome, EditorPointerOutcome, EditorRuntime, MouseCapture};
use super::{
    apply_edit_hit_target, set_wysiwyg_cursor_and_selection, set_wysiwyg_visual_caret_and_selection,
};
use crate::cursor_motion::{hit_test_row_byte, hit_test_visual_row};
use crate::event::ShellEffect;
use crate::pointer_capture::CanvasDragSession;
use crate::tab_session::{TabSession, TabSessionMut};

const CANVAS_DRAG_THRESHOLD_PX: f32 = 5.0;
const PLUGIN_SELECTION_DRAG_THRESHOLD_LOGICAL_PX: f32 = 5.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum PointerClickFocusLoss {
    #[default]
    Reset,
    Preserve,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginPointerPolicy {
    #[default]
    GroupedSelection,
    VisualCaret,
}

#[derive(Clone, Copy)]
pub(super) enum SelectionMotion {
    Press { extend: bool },
    Drag,
}

impl SelectionMotion {
    fn extends(self) -> bool {
        match self {
            Self::Press { extend } => extend,
            Self::Drag => true,
        }
    }
}

struct PluginPointerHit {
    bounds: ui::Rect,
    target: Option<Option<EditHitTarget>>,
}

impl EditorRuntime {
    pub fn configure_pointer_clicks(
        &mut self,
        timing: PointerClickTiming,
        focus_loss: PointerClickFocusLoss,
    ) {
        self.pointer_click_tracker = PointerClickTracker::with_timing(timing);
        self.pointer_click_focus_loss = focus_loss;
    }

    pub fn configure_plugin_pointer(&mut self, policy: PluginPointerPolicy) {
        self.plugin_pointer_policy = policy;
    }

    fn record_special_pointer_press(&mut self, position: (f32, f32)) {
        match self.plugin_pointer_policy {
            PluginPointerPolicy::GroupedSelection => self.pointer_click_tracker.reset(),
            PluginPointerPolicy::VisualCaret => {
                self.record_pointer_press(position);
            }
        }
    }

    fn record_pointer_press(&mut self, position: (f32, f32)) -> PointerClickKind {
        self.pointer_click_tracker.record_press(position)
    }

    fn last_pointer_press(&self) -> Option<((f32, f32), PointerClickKind)> {
        self.pointer_click_tracker.last_press()
    }

    fn plugin_selection_drag_started(&self, position: (f32, f32)) -> bool {
        let Some((press, _)) = self.last_pointer_press() else {
            return false;
        };
        let dx = position.0 - press.0;
        let dy = position.1 - press.1;
        let threshold = PLUGIN_SELECTION_DRAG_THRESHOLD_LOGICAL_PX * self.scale_factor() as f32;
        dx * dx + dy * dy > threshold * threshold
    }

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
                return EditorPointerOutcome::from_editor(self.cancel_pointer_capture(), None);
            }
            let editor =
                self.clear_active_canvas_pointer().then(redraw_outcome).unwrap_or_default();
            return EditorPointerOutcome::from_editor(editor, None);
        };
        if let Some(tab_id) = self.active_tab_id()
            && self
                .tab_session(tab_id)
                .is_some_and(|tab| tab.plugin().mode() == PluginMode::Reading)
        {
            let editor =
                self.handle_reading_pointer(tab_id, context, editor_bounds, position, phase);
            let cursor_icon = self.pointer_cursor_icon(context, editor_bounds, position, None);
            return EditorPointerOutcome::from_editor(editor, cursor_icon);
        }
        let (editor, plugin_hit) = match phase {
            PointerPhase::Press => {
                self.handle_editor_pointer_press(context, editor_bounds, position)
            }
            PointerPhase::Move => self.handle_editor_pointer_move(context, editor_bounds, position),
            PointerPhase::Release => {
                (self.handle_editor_pointer_release(editor_bounds, position), None)
            }
        };
        let cursor_icon =
            self.pointer_cursor_icon(context, editor_bounds, position, plugin_hit.as_ref());
        EditorPointerOutcome::from_editor(editor, cursor_icon)
    }

    fn handle_editor_pointer_press(
        &mut self,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> (EditorOutcome, Option<PluginPointerHit>) {
        if !self.pointer_input_allowed(context, position)
            || !editor_bounds.contains(position.0, position.1)
        {
            self.pointer_click_tracker.reset();
            return (EditorOutcome::default(), None);
        }
        let cancel = self.cancel_pointer_capture();
        if let Some(outcome) = self.activate_canvas_control(position) {
            self.record_special_pointer_press(position);
            return (cancel.merge(outcome), None);
        }
        let plugin_hit = self.hit_test_plugin_pointer(editor_bounds, position);
        if let Some(hit) = &plugin_hit
            && self.begin_canvas_source_drag(context, position, hit)
        {
            self.record_special_pointer_press(position);
            return (cancel.merge(redraw_outcome()), plugin_hit);
        }

        let click_kind = self.record_pointer_press(position);
        let granularity = selection_granularity(click_kind);
        let extend = self.input_modifiers().shift_key();
        let began_selection = self.begin_text_selection_with_granularity(context, granularity);
        let placed_selection = began_selection
            && self.place_pointer_selection(
                editor_bounds,
                position,
                SelectionMotion::Press { extend },
                plugin_hit.as_ref(),
                click_kind,
                None,
            );
        if !placed_selection {
            if self.plugin_pointer_policy == PluginPointerPolicy::GroupedSelection {
                self.pointer_click_tracker.reset();
            }
            self.end_pointer_capture();
            return (cancel, plugin_hit);
        }
        if let Some(initial_range) = self.current_text_selection_range() {
            self.set_initial_text_selection_range(initial_range);
        }
        (cancel.merge(redraw_outcome()), plugin_hit)
    }

    fn handle_editor_pointer_move(
        &mut self,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> (EditorOutcome, Option<PluginPointerHit>) {
        let hover = if self.update_active_canvas_pointer(context, editor_bounds, position) {
            redraw_outcome()
        } else {
            EditorOutcome::default()
        };
        if !self.pointer_input_allowed(context, position) {
            return (hover, None);
        }
        match self.pointer_capture() {
            MouseCapture::CanvasDrag => {
                (hover.merge(self.update_canvas_source_drag(editor_bounds, position)), None)
            }
            MouseCapture::TextSelection => {
                let click_kind = self
                    .text_selection_granularity()
                    .map(pointer_click_kind)
                    .unwrap_or(PointerClickKind::Single);
                let initial_range = self.initial_text_selection_range();
                let plugin_hit = self.hit_test_plugin_pointer(editor_bounds, position);
                let selection = self
                    .place_pointer_selection(
                        editor_bounds,
                        position,
                        SelectionMotion::Drag,
                        plugin_hit.as_ref(),
                        click_kind,
                        initial_range.as_ref(),
                    )
                    .then(redraw_outcome)
                    .unwrap_or_default();
                (hover.merge(selection), plugin_hit)
            }
            MouseCapture::None => (hover, None),
        }
    }

    fn handle_editor_pointer_release(
        &mut self,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> EditorOutcome {
        match self.pointer_capture() {
            MouseCapture::CanvasDrag => self.finish_canvas_source_drag(editor_bounds, position),
            MouseCapture::TextSelection => {
                self.end_pointer_capture();
                self.clear_collapsed_selection();
                redraw_outcome()
            }
            MouseCapture::None => EditorOutcome::default(),
        }
    }

    fn handle_reading_pointer(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
        phase: PointerPhase,
    ) -> EditorOutcome {
        let bounds = self.plugin_input_bounds(editor_bounds, false);
        match phase {
            PointerPhase::Press if self.pointer_input_allowed(context, position) => {
                let click_kind = self.record_pointer_press(position);
                if !self.begin_text_selection_with_granularity(
                    context,
                    selection_granularity(click_kind),
                ) {
                    return EditorOutcome::default();
                }
                let extend = self.input_modifiers().shift_key();
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return EditorOutcome::default();
                };
                place_reading_selection(&mut tab, position, bounds, click_kind, extend);
            }
            PointerPhase::Move
                if self.pointer_capture() == MouseCapture::TextSelection
                    && self.pointer_input_allowed(context, position) =>
            {
                if !self.plugin_selection_drag_started(position) {
                    return EditorOutcome::default();
                }
                let Some((_, click_kind)) = self.last_pointer_press() else {
                    return EditorOutcome::default();
                };
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return EditorOutcome::default();
                };
                extend_reading_selection(&mut tab, position, bounds, click_kind);
            }
            PointerPhase::Release if self.pointer_capture() == MouseCapture::TextSelection => {
                self.end_pointer_capture();
                if let Some(mut tab) = self.tab_session_mut(tab_id)
                    && !tab.plugin_has_selection()
                {
                    tab.clear_plugin_selection();
                }
            }
            PointerPhase::Press | PointerPhase::Move | PointerPhase::Release => {
                return EditorOutcome::default();
            }
        }
        redraw_outcome()
    }

    fn hit_test_plugin_pointer(
        &self,
        editor_bounds: ui::Rect,
        position: (f32, f32),
    ) -> Option<PluginPointerHit> {
        let tab = self.active_tab_id().and_then(|id| self.tab_session(id))?;
        if !tab.handles_own_rendering() {
            return None;
        }
        let bounds = self.plugin_input_bounds(editor_bounds, tab.is_canvas());
        let target = tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y);
        Some(PluginPointerHit { bounds, target })
    }

    fn pointer_cursor_icon(
        &self,
        context: EditorInputContext,
        editor_bounds: ui::Rect,
        position: (f32, f32),
        plugin_hit: Option<&PluginPointerHit>,
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
            if self.source_pointer_policy == super::SourcePointerPolicy::VisualRows
                && self.source_input_left().is_some_and(|text_left| position.0 < text_left)
            {
                return Some(winit::window::CursorIcon::Default);
            }
            return Some(winit::window::CursorIcon::Text);
        }
        if !tab.allows_editing() {
            if tab.plugin().mode() == PluginMode::Reading
                && self.plugin_pointer_policy == PluginPointerPolicy::VisualCaret
            {
                return Some(winit::window::CursorIcon::Text);
            }
            return Some(winit::window::CursorIcon::Default);
        }

        if tab.hit_test_canvas_control(CanvasPoint::new(position.0, position.1)).is_some() {
            return Some(winit::window::CursorIcon::Pointer);
        }
        let resolved;
        let hit = if let Some(hit) = plugin_hit {
            hit
        } else {
            resolved = self.hit_test_plugin_pointer(editor_bounds, position)?;
            &resolved
        };
        if self.plugin_pointer_policy == PluginPointerPolicy::VisualCaret
            && !tab.is_canvas()
            && hit.target.is_none()
            && hit.bounds.contains(position.0, position.1)
        {
            return Some(winit::window::CursorIcon::Text);
        }
        Some(plugin_hit_cursor_icon(hit.target.as_ref().and_then(Option::as_ref)))
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
        position: (f32, f32),
        hit: &PluginPointerHit,
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let Some(tab) = self.tab_session(tab_id) else {
            return false;
        };
        if !tab.handles_own_rendering()
            || !tab.allows_editing()
            || (!tab.is_canvas()
                && self.plugin_pointer_policy == PluginPointerPolicy::GroupedSelection)
        {
            return false;
        }
        let Some(Some(EditHitTarget::SourceObject { source_range })) = &hit.target else {
            return false;
        };
        let Some(source_range) = validated_source_object_range(tab.document, source_range.clone())
        else {
            return false;
        };
        let source_generation = tab.document.generation();
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        tab.document.break_edit_merge();
        tab.document.cursor_move_to_offset(source_range.end);
        tab.document.cursor_mut().selection_anchor = Some(source_range.start);
        tab.set_plugin_selection_anchor_byte(Some(source_range.start));
        tab.set_plugin_selection_cursor_byte(Some(source_range.end));
        tab.set_cursor_byte(source_range.end);

        self.input_session.set_preferred_x(None);
        self.input_session.start_canvas_drag_session(
            context,
            CapturedCanvasDrag {
                tab_id,
                bounds: hit.bounds,
                gesture: CanvasDragSession {
                    source_range,
                    pressed_at: position,
                    source_generation,
                    started: false,
                },
            },
        )
    }

    fn update_canvas_source_drag(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
    ) -> EditorOutcome {
        let active_tab = self.active_tab_id();
        let bounds = self.active_plugin_drag_bounds(editor_rect);
        let Some(drag) = self.input_session.canvas_drag_session_mut() else {
            return EditorOutcome::default();
        };
        if Some(drag.tab_id) != active_tab {
            return self.cancel_pointer_capture();
        }
        let tab_id = drag.tab_id;
        drag.bounds = bounds.unwrap_or(drag.bounds);
        drag.gesture
            .move_request(position, drag.bounds)
            .map_or_else(EditorOutcome::default, |request| {
                self.dispatch_canvas_drag_request(tab_id, request)
            })
    }

    fn finish_canvas_source_drag(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
    ) -> EditorOutcome {
        let bounds = self.active_plugin_drag_bounds(editor_rect);
        let drag = self.input_session.take_canvas_drag_session();
        self.end_pointer_capture();
        let Some(drag) = drag else {
            return redraw_outcome();
        };
        let request = if Some(drag.tab_id) == self.active_tab_id() {
            drag.gesture.release_request(position, bounds.unwrap_or(drag.bounds))
        } else {
            drag.gesture.cancel_request(drag.bounds)
        };
        request.map_or_else(redraw_outcome, |request| {
            self.dispatch_canvas_drag_request(drag.tab_id, request)
        })
    }

    fn active_plugin_drag_bounds(&self, editor_rect: ui::Rect) -> Option<ui::Rect> {
        let tab = self.active_tab_id().and_then(|id| self.tab_session(id))?;
        Some(self.plugin_input_bounds(editor_rect, tab.is_canvas()))
    }

    /// 结束当前手势。已启动的画布拖拽会通知发起它的标签。
    pub fn cancel_pointer_capture(&mut self) -> EditorOutcome {
        let capture = self.pointer_capture();
        let drag = self.input_session.take_canvas_drag_session();
        self.end_pointer_capture();
        if let Some(drag) = drag
            && let Some(request) = drag.gesture.cancel_request(drag.bounds)
        {
            return self.dispatch_canvas_drag_request(drag.tab_id, request);
        }
        if capture == MouseCapture::TextSelection {
            self.clear_collapsed_selection();
        }
        EditorOutcome::default()
    }

    fn dispatch_canvas_drag_request(
        &mut self,
        tab_id: TabId,
        request: CanvasDragRequest,
    ) -> EditorOutcome {
        let phase = request.phase;
        let response = self
            .tab_session_mut(tab_id)
            .map(|mut tab| tab.handle_canvas_drag_plugin(request))
            .unwrap_or(CanvasDragResponse::Ignore);
        match (phase, response) {
            (CanvasDragPhase::Drop, CanvasDragResponse::Apply(transaction))
                if self.active_tab_id() == Some(tab_id) =>
            {
                self.model_session
                    .apply_active_edit_transaction(transaction, self.editor_line_height())
                    .merge(redraw_outcome())
            }
            (CanvasDragPhase::Drop | CanvasDragPhase::Cancel, _)
            | (_, CanvasDragResponse::Preview(_)) => redraw_outcome(),
            _ => EditorOutcome::default(),
        }
    }

    fn place_pointer_selection(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
        motion: SelectionMotion,
        plugin_hit: Option<&PluginPointerHit>,
        click_kind: PointerClickKind,
        initial_range: Option<&std::ops::Range<usize>>,
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        // A press breaks the typing undo run even when the caret does not move.
        if matches!(motion, SelectionMotion::Press { .. })
            && let Some(tab) = self.tab_session_mut(tab_id)
        {
            tab.document.break_edit_merge();
        }
        if let Some(hit) = plugin_hit {
            return self.place_plugin_pointer_selection(
                tab_id,
                hit,
                position,
                motion,
                click_kind,
                initial_range,
            );
        }
        if self.source_pointer_policy == super::SourcePointerPolicy::VisualRows {
            return self.place_source_row_selection(
                tab_id,
                editor_rect,
                position,
                motion,
                click_kind,
            );
        }
        let dpi = self.scale_factor() as f32;
        let settings = self.settings.clone();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
        let painted_text_left = self.source_input_left();
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        let text_left = painted_text_left.unwrap_or_else(|| {
            editor_rect.x
                + metrics
                    .content_left_margin
                    .max(settings.gutter_width(tab.document.line_count()) * metrics.dpi)
        });
        let byte_offset = hit_test_text_byte(&tab, editor_rect.y, text_left, position, &metrics)
            .unwrap_or_else(|| tab.document.buffer_len());
        place_text_pointer_selection(
            &mut tab,
            byte_offset,
            motion.extends(),
            click_kind,
            initial_range,
        );
        true
    }

    fn place_plugin_pointer_selection(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
        hit: &PluginPointerHit,
        position: (f32, f32),
        motion: SelectionMotion,
        click_kind: PointerClickKind,
        initial_range: Option<&std::ops::Range<usize>>,
    ) -> bool {
        let bounds = hit.bounds;
        let visual_caret = self.plugin_pointer_policy == PluginPointerPolicy::VisualCaret;
        if visual_caret
            && matches!(motion, SelectionMotion::Drag)
            && !self.plugin_selection_drag_started(position)
        {
            return false;
        }
        let extend = motion.extends();
        let edit_target = {
            let Some(tab) = self.tab_session(tab_id) else {
                return false;
            };
            if tab.hit_test_canvas_control(CanvasPoint::new(position.0, position.1)).is_some() {
                return false;
            }
            &hit.target
        };
        match edit_target {
            Some(Some(EditHitTarget::TextCaret { byte_offset, selection_scope }))
                if visual_caret =>
            {
                self.place_visual_plugin_caret(
                    tab_id,
                    bounds,
                    position,
                    *byte_offset,
                    selection_scope.clone(),
                    motion,
                    click_kind,
                )
            }
            Some(Some(EditHitTarget::TextCaret { byte_offset, .. })) => {
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                place_text_pointer_selection(
                    &mut tab,
                    *byte_offset,
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
            Some(Some(EditHitTarget::ClearFocus))
                if matches!(motion, SelectionMotion::Press { .. }) =>
            {
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                apply_edit_hit_target(&mut tab, EditHitTarget::ClearFocus);
                self.input_session.set_preferred_x(None);
                true
            }
            Some(None) => false,
            Some(Some(EditHitTarget::SourceObject { .. } | EditHitTarget::ClearFocus)) => false,
            None => self.place_plugin_byte_selection(
                tab_id,
                bounds,
                position,
                motion,
                click_kind,
                initial_range,
            ),
        }
    }

    fn place_plugin_byte_selection(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
        bounds: ui::Rect,
        position: (f32, f32),
        motion: SelectionMotion,
        click_kind: PointerClickKind,
        initial_range: Option<&std::ops::Range<usize>>,
    ) -> bool {
        let visual_caret = self.plugin_pointer_policy == PluginPointerPolicy::VisualCaret;
        let extend = motion.extends();
        let Some(tab) = self.tab_session(tab_id) else {
            return false;
        };
        let candidate = tab.hit_test_plugin_byte_or_end(position, bounds);
        let Some(candidate) = candidate else {
            return false;
        };
        if visual_caret && matches!(motion, SelectionMotion::Drag) {
            let Some(mut tab) = self.tab_session_mut(tab_id) else {
                return false;
            };
            let anchor = tab.document.cursor().selection_anchor;
            set_wysiwyg_cursor_and_selection(&mut tab, candidate, anchor);
            return true;
        }
        let snapped_candidate = {
            let Some(mut tab) = self.tab_session_mut(tab_id) else {
                return false;
            };
            if !visual_caret && extend {
                place_text_pointer_selection(&mut tab, candidate, true, click_kind, initial_range);
            } else {
                place_text_caret(&mut tab, candidate, false);
            }
            let snapped_candidate = tab.document.cursor_offset().to_usize();
            tab.set_cursor_byte(snapped_candidate);
            snapped_candidate
        };
        if !visual_caret && extend {
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
        if visual_caret {
            tab.document.cursor_move_to_offset(final_byte);
            let snapped_byte = tab.document.cursor_offset().to_usize();
            place_visual_plugin_press(
                &mut tab,
                VisualCaretPosition {
                    byte_offset: snapped_byte,
                    affinity: CursorAffinity::Downstream,
                },
                click_kind,
            );
        } else {
            place_text_pointer_selection(&mut tab, final_byte, extend, click_kind, initial_range);
            let snapped_final = tab.document.cursor_offset().to_usize();
            tab.set_cursor_byte(snapped_final);
        }
        self.input_session.set_preferred_x(None);
        true
    }

    fn place_visual_plugin_caret(
        &mut self,
        tab_id: TabId,
        bounds: ui::Rect,
        point: (f32, f32),
        byte_offset: usize,
        selection_scope: Option<std::ops::Range<usize>>,
        motion: SelectionMotion,
        click_kind: PointerClickKind,
    ) -> bool {
        let Some(tab) = self.tab_session(tab_id) else {
            return false;
        };
        let position = tab
            .hit_test_caret(point.0, point.1, bounds.x, bounds.y)
            .filter(|position| position.byte_offset == byte_offset)
            .unwrap_or(VisualCaretPosition { byte_offset, affinity: CursorAffinity::Downstream });
        if matches!(motion, SelectionMotion::Drag)
            && self
                .input_session
                .visual_caret_scope()
                .is_some_and(|scope| Some(scope) != selection_scope.as_ref())
        {
            return false;
        }

        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        match motion {
            SelectionMotion::Drag => {
                let anchor = tab.document.cursor().selection_anchor;
                set_wysiwyg_visual_caret_and_selection(&mut tab, position, anchor);
            }
            SelectionMotion::Press { .. } => {
                place_visual_plugin_press(&mut tab, position, click_kind);
                self.input_session.set_visual_caret_scope(selection_scope);
                self.input_session.set_preferred_x(None);
            }
        }
        true
    }

    fn refresh_plugin_layout_for_pointer_hit(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
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
        if tab.is_canvas()
            && let Some(snapshot) = tab
                .prepare_canvas_plugin(&theme, &mut shaper, dpi)
                .and_then(|metrics| tab.prepare_canvas_viewport(metrics, bounds, dpi))
        {
            let _ = tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi);
        } else {
            let _ = tab.render_plugin(bounds, &theme, &mut shaper, dpi);
        }
    }

    fn clear_collapsed_selection(&mut self) {
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        if tab.clear_collapsed_pointer_selection() && tab.handles_own_rendering() {
            tab.set_plugin_selection_anchor_byte(None);
            tab.set_plugin_selection_cursor_byte(None);
        }
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

pub(super) fn selection_granularity(click_kind: PointerClickKind) -> TextSelectionGranularity {
    match click_kind {
        PointerClickKind::Single => TextSelectionGranularity::Grapheme,
        PointerClickKind::Double => TextSelectionGranularity::Word,
        PointerClickKind::Triple => TextSelectionGranularity::SourceLine,
    }
}

pub(super) fn pointer_click_kind(granularity: TextSelectionGranularity) -> PointerClickKind {
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

fn plugin_hit_cursor_icon(target: Option<&EditHitTarget>) -> winit::window::CursorIcon {
    match target {
        Some(EditHitTarget::TextCaret { .. }) => winit::window::CursorIcon::Text,
        Some(EditHitTarget::SourceObject { .. }) => winit::window::CursorIcon::Grab,
        Some(EditHitTarget::ClearFocus) | None => winit::window::CursorIcon::Default,
    }
}

fn place_reading_selection(
    tab: &mut TabSessionMut<'_>,
    point: (f32, f32),
    bounds: ui::Rect,
    click_kind: PointerClickKind,
    extend: bool,
) {
    let Some(position) = tab.hit_test_position(point.0, point.1, bounds.x, bounds.y) else {
        tab.clear_plugin_selection();
        return;
    };
    if extend {
        tab.set_plugin_selection_cursor(Some(position));
        return;
    }

    if let Some(range) = reading_selection_range(tab, position, click_kind) {
        tab.set_plugin_selection_anchor(Some(range.start));
        tab.set_plugin_selection_cursor(Some(range.end));
    }
}

fn extend_reading_selection(
    tab: &mut TabSessionMut<'_>,
    point: (f32, f32),
    bounds: ui::Rect,
    click_kind: PointerClickKind,
) {
    let Some(position) = tab.hit_test_position(point.0, point.1, bounds.x, bounds.y) else {
        return;
    };
    let Some(range) = reading_selection_range(tab, position, click_kind) else {
        return;
    };
    let previous = tab.selection_cursor().unwrap_or(position);
    let endpoint = if position >= previous { range.end } else { range.start };
    tab.set_plugin_selection_cursor(Some(endpoint));
}

fn reading_selection_range(
    tab: &TabSessionMut<'_>,
    position: (usize, usize),
    click_kind: PointerClickKind,
) -> Option<std::ops::Range<(usize, usize)>> {
    let (start, end) = match click_kind {
        PointerClickKind::Single => Some((position, position)),
        PointerClickKind::Double => tab.word_range_at_pos(position.0, position.1),
        PointerClickKind::Triple => tab.line_range_at_pos(position.0, position.1),
    }?;
    Some(start..end)
}

fn place_visual_plugin_press(
    tab: &mut TabSessionMut<'_>,
    position: VisualCaretPosition,
    click_kind: PointerClickKind,
) {
    let (position, anchor) = if click_kind == PointerClickKind::Double {
        let (start, end) = tab.document.word_select_at(position.byte_offset);
        let affinity = if end == position.byte_offset {
            position.affinity
        } else {
            CursorAffinity::Downstream
        };
        (VisualCaretPosition { byte_offset: end, affinity }, start)
    } else {
        (position, position.byte_offset)
    };
    set_wysiwyg_visual_caret_and_selection(tab, position, Some(anchor));
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
    text_top: f32,
    text_left: f32,
    position: (f32, f32),
    metrics: &ui::settings::UiMetrics,
) -> Option<usize> {
    let display = tab.display();
    let sub_line_offset = display.viewport.sub_line_pixel_offset(metrics.line_height);
    let (_, entry) = hit_test_visual_row(
        position.1,
        text_top,
        sub_line_offset,
        metrics.line_height,
        &display.advance_cache,
    )?;
    let line_byte = hit_test_row_byte(position.0, text_left, entry);
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
