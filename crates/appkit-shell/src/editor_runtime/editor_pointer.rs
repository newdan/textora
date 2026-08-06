//! 产品无关的编辑器指针命中与选择会话。

use ui::plugin::EditHitTarget;
use ui::plugin::{CanvasDragPhase, CanvasDragRequest, CanvasDragResponse};

use super::{EditorInputContext, EditorOutcome, EditorRuntime, MouseCapture};
use crate::event::ShellEffect;
use crate::mouse_state::{CanvasDragEligibility, CanvasDragSession};
use crate::tab_session::TabSessionMut;

const CANVAS_DRAG_THRESHOLD_PX: f32 = 5.0;

impl EditorRuntime {
    pub fn handle_pointer_event(
        &mut self,
        context: EditorInputContext,
        event: &ui::Event,
    ) -> EditorOutcome {
        let Some((position, phase)) = pointer_event(event) else {
            return EditorOutcome::default();
        };
        match phase {
            PointerPhase::Press if self.pointer_input_allowed(context, position) => {
                if self.begin_canvas_source_drag(context, position) {
                    return redraw_outcome();
                }
                if !self.begin_text_selection(context)
                    || !self.place_pointer_selection(context.editor_rect, position, false)
                {
                    self.end_pointer_capture();
                    return EditorOutcome::default();
                }
                redraw_outcome()
            }
            PointerPhase::Move
                if self.pointer_capture() == MouseCapture::CanvasDrag
                    && self.pointer_input_allowed(context, position) =>
            {
                self.update_canvas_source_drag(context.editor_rect, position)
            }
            PointerPhase::Move
                if self.pointer_capture() == MouseCapture::TextSelection
                    && self.pointer_input_allowed(context, position) =>
            {
                self.place_pointer_selection(context.editor_rect, position, true)
                    .then(redraw_outcome)
                    .unwrap_or_default()
            }
            PointerPhase::Release if self.pointer_capture() == MouseCapture::CanvasDrag => {
                self.finish_canvas_source_drag(context.editor_rect, position)
            }
            PointerPhase::Release if self.pointer_capture() != MouseCapture::None => {
                self.end_pointer_capture();
                self.clear_collapsed_selection();
                redraw_outcome()
            }
            PointerPhase::Press | PointerPhase::Move | PointerPhase::Release => {
                EditorOutcome::default()
            }
        }
    }

    fn begin_canvas_source_drag(
        &mut self,
        context: EditorInputContext,
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
            if !tab.is_canvas() || !tab.runtime.plugin.handles_own_rendering() {
                return false;
            }
            let bounds = super::editor_painter::plugin_bounds(context.editor_rect, dpi, true);
            let Some(Some(EditHitTarget::SourceObject { source_range })) =
                tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y)
            else {
                return false;
            };
            let source_generation = tab.document.generation();
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
        let (phase, session) = {
            let Some(session) = self.input_session.canvas_drag_session_mut() else {
                return EditorOutcome::default();
            };
            if session.eligibility == CanvasDragEligibility::Disabled {
                return EditorOutcome::default();
            }
            if !session.started {
                let horizontal_distance = position.0 - session.pressed_at.0;
                let vertical_distance = position.1 - session.pressed_at.1;
                let threshold_squared = CANVAS_DRAG_THRESHOLD_PX * CANVAS_DRAG_THRESHOLD_PX;
                if horizontal_distance * horizontal_distance + vertical_distance * vertical_distance
                    <= threshold_squared
                {
                    return EditorOutcome::default();
                }
                session.started = true;
                (CanvasDragPhase::Start, session.clone())
            } else {
                (CanvasDragPhase::Update, session.clone())
            }
        };
        let response = self.dispatch_canvas_drag(editor_rect, position, phase, &session);
        self.canvas_drag_response(phase, response)
    }

    fn finish_canvas_source_drag(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
    ) -> EditorOutcome {
        let session = self.input_session.take_canvas_drag_session();
        self.end_pointer_capture();
        let Some(session) = session.filter(|session| session.started) else {
            return redraw_outcome();
        };
        let response =
            self.dispatch_canvas_drag(editor_rect, position, CanvasDragPhase::Drop, &session);
        let mut outcome = self.canvas_drag_response(CanvasDragPhase::Drop, response);
        outcome.shell_effect = outcome.shell_effect.merge(ShellEffect::REDRAW);
        outcome
    }

    fn dispatch_canvas_drag(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
        phase: CanvasDragPhase,
        session: &CanvasDragSession,
    ) -> CanvasDragResponse {
        let Some(tab_id) = self.active_tab_id() else {
            return CanvasDragResponse::Ignore;
        };
        let dpi = self.scale_factor() as f32;
        let bounds = super::editor_painter::plugin_bounds(editor_rect, dpi, true);
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return CanvasDragResponse::Ignore;
        };
        tab.handle_canvas_drag_plugin(CanvasDragRequest {
            phase,
            source_range: session.source_range.clone(),
            pointer_x: position.0,
            pointer_y: position.1,
            pressed_x: session.pressed_at.0,
            pressed_y: session.pressed_at.1,
            offset_x: bounds.x,
            offset_y: bounds.y,
            source_generation: session.source_generation,
        })
    }

    fn canvas_drag_response(
        &mut self,
        phase: CanvasDragPhase,
        response: CanvasDragResponse,
    ) -> EditorOutcome {
        match response {
            CanvasDragResponse::Ignore => EditorOutcome::default(),
            CanvasDragResponse::Preview(_) => redraw_outcome(),
            CanvasDragResponse::Apply(transaction) if phase == CanvasDragPhase::Drop => self
                .model_session
                .apply_active_edit_transaction(transaction, self.editor_line_height()),
            CanvasDragResponse::Apply(_) => EditorOutcome::default(),
        }
    }

    fn place_pointer_selection(
        &mut self,
        editor_rect: ui::Rect,
        position: (f32, f32),
        extend: bool,
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let dpi = self.scale_factor() as f32;
        let settings = self.settings.clone();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
        let handles_own_rendering =
            self.tab_session(tab_id).is_some_and(|tab| tab.runtime.plugin.handles_own_rendering());
        if handles_own_rendering {
            return self.place_plugin_pointer_selection(tab_id, editor_rect, position, dpi, extend);
        }
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        let byte_offset = hit_test_text_byte(&tab, editor_rect, position, &settings, &metrics)
            .unwrap_or_else(|| tab.document.buffer_len());
        place_text_caret(&mut tab, byte_offset, extend);
        true
    }

    fn place_plugin_pointer_selection(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        editor_rect: ui::Rect,
        position: (f32, f32),
        dpi: f32,
        extend: bool,
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
            tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y)
        };
        match edit_target {
            Some(Some(EditHitTarget::TextCaret { byte_offset, .. })) => {
                let Some(mut tab) = self.tab_session_mut(tab_id) else {
                    return false;
                };
                place_text_caret(&mut tab, byte_offset, extend);
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
            Some(Some(EditHitTarget::CanvasControl { .. })) | Some(None) => false,
            Some(Some(EditHitTarget::SourceObject { .. } | EditHitTarget::ClearFocus)) => false,
            None => self.place_plugin_byte_selection(tab_id, bounds, position, extend),
        }
    }

    fn place_plugin_byte_selection(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        bounds: ui::Rect,
        position: (f32, f32),
        extend: bool,
    ) -> bool {
        let candidate = {
            let Some(tab) = self.tab_session(tab_id) else {
                return false;
            };
            tab.hit_test_byte(position.0, position.1, bounds.x, bounds.y).or_else(|| {
                (position.1 - bounds.y > tab.content_height()).then(|| tab.document.buffer_len())
            })
        };
        let Some(candidate) = candidate else {
            return false;
        };
        let snapped_candidate = {
            let Some(mut tab) = self.tab_session_mut(tab_id) else {
                return false;
            };
            place_text_caret(&mut tab, candidate, extend);
            let snapped_candidate = tab.document.cursor_offset().to_usize();
            tab.send_message(ui::plugin::PluginMessage::SetCursorByte(snapped_candidate));
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
            let expanded_position = expanded_plugin_hit_position(
                position,
                tab.query_cursor_screen_rect(snapped_candidate),
                bounds,
            );
            tab.hit_test_byte(expanded_position.0, expanded_position.1, bounds.x, bounds.y)
                .unwrap_or(snapped_candidate)
        };
        if final_byte != snapped_candidate {
            let Some(mut tab) = self.tab_session_mut(tab_id) else {
                return false;
            };
            place_text_caret(&mut tab, final_byte, false);
            let snapped_final = tab.document.cursor_offset().to_usize();
            tab.send_message(ui::plugin::PluginMessage::SetCursorByte(snapped_final));
        }
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
        let Some(tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        if tab.document.cursor().selection_anchor == Some(tab.document.cursor_offset().to_usize()) {
            tab.document.cursor_mut().selection_anchor = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerPhase {
    Press,
    Move,
    Release,
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

fn hit_test_text_byte(
    tab: &TabSessionMut<'_>,
    editor_rect: ui::Rect,
    position: (f32, f32),
    settings: &ui::settings::Settings,
    metrics: &ui::settings::UiMetrics,
) -> Option<usize> {
    let sub_line_offset = tab.display().viewport.sub_line_pixel_offset(metrics.line_height);
    let adjusted_y = position.1 - editor_rect.y - sub_line_offset;
    if adjusted_y < 0.0 {
        return None;
    }
    let visual_line = (adjusted_y / metrics.line_height) as usize;
    let entry = tab.display().advance_cache.get(visual_line)?;
    let gutter_width = metrics
        .content_left_margin
        .max(settings.gutter_width(tab.document.line_count()) * metrics.dpi);
    let left_margin = editor_rect.x + gutter_width;
    let mut previous_x = left_margin;
    let mut previous_byte = entry.vl_byte_start;
    for &(cluster_end, cluster_x, _) in &entry.clusters {
        let current_byte = entry.vl_byte_start + cluster_end;
        if position.0 <= cluster_x {
            let midpoint = previous_x + (cluster_x - previous_x) * 0.5;
            let line_byte = if position.0 >= midpoint { current_byte } else { previous_byte };
            return tab.document.line_byte_offset(entry.doc_line).map(|start| start + line_byte);
        }
        previous_x = cluster_x;
        previous_byte = current_byte;
    }
    tab.document.line_byte_offset(entry.doc_line).map(|start| start + previous_byte)
}

fn redraw_outcome() -> EditorOutcome {
    EditorOutcome { shell_effect: ShellEffect::REDRAW, ..EditorOutcome::default() }
}
