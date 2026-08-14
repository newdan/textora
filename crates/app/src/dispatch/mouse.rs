use crate::app::App;
use crate::app_effect::AppEffect;
use crate::dispatch::wysiwyg::{apply_edit_hit_target, set_wysiwyg_cursor_and_selection};
use crate::edit_transaction::execute_edit_plan;
use crate::mouse::CanvasDragEligibility;
use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
use core::types::UniCharOffset;
use ui::canvas::CanvasPoint;
use ui::plugin::{
    CanvasDragPhase, CanvasDragRequest, CanvasDragResponse, EditHitTarget, EditPlan,
    EditTransaction, PluginMessage,
};
use winit::event::ElementState;

const PLUGIN_SELECTION_DRAG_THRESHOLD_PX: f32 = 5.0;

fn editor_input_context(app: &App) -> EditorInputContext {
    let editor_focus =
        matches!(app.ui_shell.keyboard_focus(), crate::ui_shell::KeyboardFocusTarget::Editor)
            && app.editor_runtime.window_focused();
    EditorInputContext {
        focus: if editor_focus { EditorFocus::Active } else { EditorFocus::Inactive },
        modal_blocked: app.ui_shell.active_overlay_is_modal(),
    }
}

fn semantic_drag_stays_within_scope(
    start_scope: Option<&std::ops::Range<usize>>,
    current_scope: Option<&std::ops::Range<usize>>,
) -> bool {
    start_scope.is_none_or(|scope| current_scope == Some(scope))
}

fn expanded_wysiwyg_hit_point(
    mouse_x: f32,
    mouse_y: f32,
    cursor_rect: Option<(f32, f32, f32, f32)>,
    offset_x: f32,
    offset_y: f32,
) -> (f32, f32) {
    let Some((cursor_x, cursor_y, _cursor_width, cursor_height)) = cursor_rect else {
        return (mouse_x, mouse_y);
    };
    (offset_x + cursor_x, offset_y + cursor_y + cursor_height * 0.5)
}

fn plugin_selection_drag_started(mouse: &crate::mouse::MouseState, px: f32, py: f32) -> bool {
    let dx = px - mouse.last_click_pos.0;
    let dy = py - mouse.last_click_pos.1;
    let threshold_sq = PLUGIN_SELECTION_DRAG_THRESHOLD_PX * PLUGIN_SELECTION_DRAG_THRESHOLD_PX;
    dx * dx + dy * dy > threshold_sq
}

fn canvas_drag_started(session: &crate::mouse::CanvasDragSession, px: f32, py: f32) -> bool {
    let dx = px - session.pressed_at.0;
    let dy = py - session.pressed_at.1;
    let threshold_sq = PLUGIN_SELECTION_DRAG_THRESHOLD_PX * PLUGIN_SELECTION_DRAG_THRESHOLD_PX;
    dx * dx + dy * dy > threshold_sq
}

impl App {
    pub(crate) fn dispatch_editor_mouse_input(
        &mut self,
        state: ElementState,
        px: f32,
        py: f32,
        hit: Option<(UniCharOffset, usize, usize)>,
    ) -> AppEffect {
        let is_preview = !self.active_allows_editing();
        let is_custom_renderer = self.active_handles_own_rendering();
        if is_preview {
            let render_bounds = self.plugin_render_bounds();
            let (offset_x, offset_y) = (render_bounds.x, render_bounds.y);
            let Some(preview_pos) = self
                .active_tab_session()
                .map(|tab| tab.hit_test_position(px, py, offset_x, offset_y))
            else {
                return AppEffect::NONE;
            };
            let pressed = state.is_pressed();
            if pressed {
                let _ = self.editor_runtime.begin_text_selection(editor_input_context(self));
            } else {
                self.editor_runtime.end_pointer_capture();
            }
            if pressed {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(self.mouse.last_click_time);
                let dx = px - self.mouse.last_click_pos.0;
                let dy = py - self.mouse.last_click_pos.1;
                if elapsed.as_millis() > 500 || dx * dx + dy * dy > 25.0 {
                    self.mouse.click_count = 0;
                }
                self.mouse.click_count = (self.mouse.click_count + 1).min(3);
                self.mouse.last_click_time = now;
                self.mouse.last_click_pos = (px, py);
            }
            self.mouse.is_down = pressed;
            let click_count = self.mouse.click_count;
            let shift_pressed = self.editor_runtime.input_modifiers().shift_key();
            let Some(mut tab) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            if !pressed {
                if !tab.has_selection() {
                    tab.send_message(PluginMessage::ClearSelection);
                }
                return AppEffect::REDRAW;
            }
            if let Some((line_index, cluster_pos)) = preview_pos {
                if shift_pressed {
                    tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                        line_index,
                        cluster_pos,
                    ))));
                } else if click_count == 3 {
                    if let Some((start, end)) = tab.line_range_at_pos(line_index, cluster_pos) {
                        tab.send_message(ui::plugin::PluginMessage::SetSelAnchor(Some(start)));
                        tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some(end)));
                    }
                } else if click_count == 2 {
                    if let Some((start, end)) = tab.word_range_at_pos(line_index, cluster_pos) {
                        tab.send_message(ui::plugin::PluginMessage::SetSelAnchor(Some(start)));
                        tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some(end)));
                    }
                } else {
                    tab.send_message(ui::plugin::PluginMessage::SetSelAnchor(Some((
                        line_index,
                        cluster_pos,
                    ))));
                    tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                        line_index,
                        cluster_pos,
                    ))));
                }
            } else {
                tab.send_message(ui::plugin::PluginMessage::ClearSelection);
            }
            AppEffect::REDRAW
        } else if is_custom_renderer {
            if state.is_pressed() {
                self.editor_runtime.end_pointer_capture();
                let _ = self.cancel_canvas_drag();
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(self.mouse.last_click_time);
                let dx = px - self.mouse.last_click_pos.0;
                let dy = py - self.mouse.last_click_pos.1;
                let dist_sq = dx * dx + dy * dy;
                if elapsed.as_millis() > 500 || dist_sq > 25.0 {
                    self.mouse.click_count = 0;
                }
                self.mouse.click_count = (self.mouse.click_count + 1).min(3);
                self.mouse.last_click_time = now;
                self.mouse.last_click_pos = (px, py);

                self.sync_plugin_state();
                match self.query_plugin_edit_hit_target(px, py) {
                    Some(Some(EditHitTarget::TextCaret { byte_offset, selection_scope })) => {
                        self.mouse.wysiwyg_selection_scope = selection_scope;
                        let click_count = self.mouse.click_count;
                        if let Some(mut tab) = self.active_tab_session_mut() {
                            if click_count == 2 {
                                let (word_start, word_end) =
                                    tab.document.word_select_at(byte_offset);
                                set_wysiwyg_cursor_and_selection(
                                    &mut tab,
                                    word_end,
                                    Some(word_start),
                                );
                            } else {
                                set_wysiwyg_cursor_and_selection(
                                    &mut tab,
                                    byte_offset,
                                    Some(byte_offset),
                                );
                            }
                        }
                        self.editor_runtime.set_preferred_x(None);
                        let _ =
                            self.editor_runtime.begin_text_selection(editor_input_context(self));
                        self.mouse.is_down = true;
                        return AppEffect::REDRAW;
                    }
                    Some(Some(EditHitTarget::CanvasControl { source_range })) => {
                        self.editor_runtime.end_pointer_capture();
                        self.mouse.wysiwyg_selection_scope = None;
                        let source_generation = self
                            .active_tab_session()
                            .map(|tab| tab.document.generation())
                            .unwrap_or_default();
                        let plan = self.active_tab_session().and_then(|tab| {
                            tab.canvas_control_edit_plan(source_range, source_generation)
                        });
                        if let Some(EditPlan::Apply(transaction)) = plan {
                            if let Some(tab) = self.active_tab_session_mut() {
                                let _ = execute_edit_plan(
                                    EditPlan::Apply(transaction),
                                    tab.document,
                                    &[],
                                );
                            }
                            self.sync_plugin_state();
                        }
                        self.mouse.is_down = false;
                        return AppEffect::REDRAW;
                    }
                    Some(Some(EditHitTarget::SourceObject { source_range })) => {
                        let _ = self.editor_runtime.begin_canvas_drag(editor_input_context(self));
                        self.mouse.wysiwyg_selection_scope = None;
                        if let Some(mut tab) = self.active_tab_session_mut() {
                            let source_generation = tab.document.generation();
                            apply_edit_hit_target(
                                &mut tab,
                                EditHitTarget::SourceObject { source_range: source_range.clone() },
                            );
                            self.mouse.canvas_drag = Some(crate::mouse::CanvasDragSession {
                                source_range,
                                pressed_at: (px, py),
                                source_generation,
                                eligibility: CanvasDragEligibility::Enabled,
                                started: false,
                            });
                        }
                        self.editor_runtime.set_preferred_x(None);
                        self.mouse.is_down = true;
                        return AppEffect::REDRAW;
                    }
                    Some(Some(target)) => {
                        let _ =
                            self.editor_runtime.begin_text_selection(editor_input_context(self));
                        self.mouse.wysiwyg_selection_scope = None;
                        if let Some(mut tab) = self.active_tab_session_mut() {
                            apply_edit_hit_target(&mut tab, target);
                        }
                        self.editor_runtime.set_preferred_x(None);
                        self.mouse.is_down = true;
                        return AppEffect::REDRAW;
                    }
                    Some(None) => {
                        self.editor_runtime.end_pointer_capture();
                        self.mouse.wysiwyg_selection_scope = None;
                        self.mouse.is_down = false;
                        return AppEffect::REDRAW;
                    }
                    None => {}
                }

                let Some(byte) = self.set_plugin_cursor_from_point(px, py) else {
                    return AppEffect::NONE;
                };
                self.mouse.wysiwyg_selection_scope = None;
                let _ = self.editor_runtime.begin_text_selection(editor_input_context(self));
                let click_count = self.mouse.click_count;
                if let Some(mut tab) = self.active_tab_session_mut() {
                    if click_count == 2 {
                        let (word_start, word_end) = tab.document.word_select_at(byte);
                        tab.document.cursor_move_to_offset(word_end);
                        let snapped_end = tab.document.cursor_offset().to_usize();
                        tab.document.cursor_mut().selection_anchor = Some(word_start);
                        tab.send_message(ui::plugin::PluginMessage::SetCursorByte(snapped_end));
                        tab.send_message(ui::plugin::PluginMessage::SetSelAnchorByte(Some(
                            word_start,
                        )));
                        tab.send_message(ui::plugin::PluginMessage::SetSelCursorByte(Some(
                            snapped_end,
                        )));
                    } else {
                        set_wysiwyg_cursor_and_selection(&mut tab, byte, Some(byte));
                    }
                }
                self.mouse.is_down = true;
            } else {
                self.editor_runtime.end_pointer_capture();
                self.mouse.is_down = false;
                self.mouse.wysiwyg_selection_scope = None;
                if let Some(session) = self.mouse.canvas_drag.take() {
                    if !session.started {
                        return AppEffect::REDRAW;
                    }
                    let response =
                        self.dispatch_canvas_drag(CanvasDragPhase::Drop, &session, px, py);
                    return self.handle_canvas_drag_response(CanvasDragPhase::Drop, response);
                }
                // Clear empty selection on mouse release (matching code editor
                // path behavior in handle_mouse_input).
                if let Some(mut tab) = self.active_tab_session_mut()
                    && tab.document.cursor().selection_anchor
                        == Some(tab.document.cursor_offset().to_usize())
                {
                    tab.document.cursor_mut().selection_anchor = None;
                    tab.send_message(ui::plugin::PluginMessage::SetSelAnchorByte(None));
                    tab.send_message(ui::plugin::PluginMessage::SetSelCursorByte(None));
                }
            }
            AppEffect::REDRAW
        } else {
            let line_height = self.ui_metrics().line_height;
            let modifiers = self.editor_runtime.input_modifiers();
            if state.is_pressed() {
                let _ = self.editor_runtime.begin_text_selection(editor_input_context(self));
            } else {
                self.editor_runtime.end_pointer_capture();
            }
            if self.handle_editor_mouse_input(state, px, py, modifiers, hit, line_height) {
                return AppEffect::REDRAW;
            }
            AppEffect::NONE
        }
    }

    /// Two-phase WYSIWYG hit test: locates the cursor byte from a mouse click.
    ///
    /// Phase 1 hits the current (possibly folded) layout to get a candidate
    /// byte, then the cursor is placed there and a synchronous layout refresh
    /// expands inline spans. Phase 2 re-hit-tests at the cursor's visual
    /// position on the expanded layout, so the final byte is accurate against
    /// the expanded source maps.
    ///
    /// Also handles the fallback case where the click is below all content
    /// (cursor moves to buffer end).
    fn set_plugin_cursor_from_point(&mut self, px: f32, py: f32) -> Option<usize> {
        // Sync source + cursor to plugin before hit-testing.
        self.sync_plugin_state();

        // Pre-compute mini-render params (immutable borrows must be released
        // before mutable workspace borrow below).
        let font_system = self.editor_runtime.shared_font_system();
        let render_bounds = self.plugin_render_bounds();
        let dpi = self.ui_metrics().dpi;
        let font_size = self.ui_metrics().font_size;

        // Use render_bounds offsets so HitTestByte coordinates match the
        // coordinate system the plugin renders into (centered reading column).
        // preview_offsets() returns gutter_left_margin for x, which mismatches
        // when the reading column is centered.
        let offset_x = render_bounds.x;
        let offset_y = render_bounds.y;

        let snapped_candidate = {
            let Some(mut tab) = self.active_tab_session_mut() else {
                return None;
            };

            // ---- Phase 1: hit-test on current (possibly folded) layout ----
            let candidate = match tab.hit_test_byte(px, py, offset_x, offset_y) {
                Some(byte) => byte,
                None => {
                    if py - offset_y > tab.content_height() {
                        return Some(tab.document.buffer_len());
                    }
                    return None;
                }
            };

            tab.document.cursor_move_to_offset(candidate);
            let snapped_candidate = tab.document.cursor_offset().to_usize();
            tab.send_message(PluginMessage::SetCursorByte(snapped_candidate));
            snapped_candidate
        };

        // Layout is refreshed synchronously so the next frame displays the expanded state.
        if let Some(fs) = font_system {
            let mut shaper = shaping::Shaper::from_shared_font_system(fs, font_size, "");
            let canvas_snapshot = self.prepare_active_canvas_frame(&mut shaper);
            let theme = self.current_theme.clone();
            let mut tab = self.active_tab_session_mut()?;
            let _ = match canvas_snapshot {
                Some(snapshot) => tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi),
                None => tab.render_plugin(render_bounds, &theme, &mut shaper, dpi),
            };
        }

        let tab = self.active_tab_session()?;
        let expanded_cursor_rect = tab.query_cursor_screen_rect(snapped_candidate);
        let (expanded_x, expanded_y) =
            expanded_wysiwyg_hit_point(px, py, expanded_cursor_rect, offset_x, offset_y);

        // ---- Phase 2: hit-test on the NEW layout ----
        let final_byte = tab
            .hit_test_byte(expanded_x, expanded_y, offset_x, offset_y)
            .unwrap_or(snapped_candidate);

        let snapped_final = if final_byte != snapped_candidate {
            let mut tab = self.active_tab_session_mut()?;
            tab.document.cursor_move_to_offset(final_byte);
            let snapped_final = tab.document.cursor_offset().to_usize();
            tab.send_message(PluginMessage::SetCursorByte(snapped_final));
            snapped_final
        } else {
            snapped_candidate
        };

        self.editor_runtime.set_preferred_x(None);
        Some(snapped_final)
    }

    fn hit_test_plugin_byte_from_point(&mut self, px: f32, py: f32) -> Option<usize> {
        let render_bounds = self.plugin_render_bounds();
        let offset_x = render_bounds.x;
        let offset_y = render_bounds.y;
        let tab = self.active_tab_session_mut()?;

        tab.hit_test_byte(px, py, offset_x, offset_y)
            .or_else(|| (py - offset_y > tab.content_height()).then(|| tab.document.buffer_len()))
    }

    pub(crate) fn query_plugin_edit_hit_target(
        &mut self,
        px: f32,
        py: f32,
    ) -> Option<Option<EditHitTarget>> {
        let render_bounds = self.plugin_render_bounds();
        let offset_x = render_bounds.x;
        let offset_y = render_bounds.y;
        let tab = self.active_tab_session_mut()?;

        tab.hit_test_edit_target(px, py, offset_x, offset_y)
    }

    fn dispatch_canvas_drag(
        &mut self,
        phase: CanvasDragPhase,
        session: &crate::mouse::CanvasDragSession,
        pointer_x: f32,
        pointer_y: f32,
    ) -> CanvasDragResponse {
        let render_bounds = self.plugin_render_bounds();
        let Some(mut tab) = self.active_tab_session_mut() else {
            return CanvasDragResponse::Ignore;
        };
        tab.handle_canvas_drag_plugin(CanvasDragRequest {
            phase,
            source_range: session.source_range.clone(),
            pointer_x,
            pointer_y,
            pressed_x: session.pressed_at.0,
            pressed_y: session.pressed_at.1,
            offset_x: render_bounds.x,
            offset_y: render_bounds.y,
            source_generation: session.source_generation,
        })
    }

    fn handle_canvas_drag_response(
        &mut self,
        phase: CanvasDragPhase,
        response: CanvasDragResponse,
    ) -> AppEffect {
        match response {
            CanvasDragResponse::Ignore => AppEffect::NONE,
            CanvasDragResponse::Preview(_) => AppEffect::REDRAW,
            CanvasDragResponse::Apply(transaction) if phase == CanvasDragPhase::Drop => {
                self.apply_canvas_drag_transaction(transaction)
            }
            CanvasDragResponse::Apply(_) => AppEffect::NONE,
        }
    }

    fn apply_canvas_drag_transaction(&mut self, transaction: EditTransaction) -> AppEffect {
        if let Some(tab) = self.active_tab_session_mut() {
            let _ = execute_edit_plan(EditPlan::Apply(transaction), tab.document, &[]);
        }
        self.sync_plugin_state();
        AppEffect::REDRAW
    }

    fn dispatch_canvas_drag_moved(&mut self, px: f32, py: f32) -> AppEffect {
        let (phase, session) = {
            let Some(session) = self.mouse.canvas_drag.as_mut() else {
                return AppEffect::NONE;
            };
            if session.eligibility == CanvasDragEligibility::Disabled {
                return AppEffect::NONE;
            }
            if !session.started {
                if !canvas_drag_started(session, px, py) {
                    return AppEffect::NONE;
                }
                session.started = true;
                (CanvasDragPhase::Start, session.clone())
            } else {
                (CanvasDragPhase::Update, session.clone())
            }
        };
        let response = self.dispatch_canvas_drag(phase, &session, px, py);
        self.handle_canvas_drag_response(phase, response)
    }

    pub(crate) fn cancel_canvas_drag(&mut self) -> AppEffect {
        let Some(session) = self.mouse.canvas_drag.take() else {
            return AppEffect::NONE;
        };
        if !session.started {
            return AppEffect::NONE;
        }
        let response = self.dispatch_canvas_drag(
            CanvasDragPhase::Cancel,
            &session,
            session.pressed_at.0,
            session.pressed_at.1,
        );
        self.handle_canvas_drag_response(CanvasDragPhase::Cancel, response).merge(AppEffect::REDRAW)
    }

    pub(crate) fn dispatch_editor_cursor_moved(
        &mut self,
        px: f32,
        py: f32,
        hit: Option<(UniCharOffset, usize, usize)>,
    ) -> AppEffect {
        let is_preview = !self.active_allows_editing();
        let is_custom_renderer = self.active_handles_own_rendering();
        if is_preview {
            if self.mouse.is_down {
                if !plugin_selection_drag_started(&self.mouse, px, py) {
                    return AppEffect::NONE;
                }
                let render_bounds = self.plugin_render_bounds();
                let (offset_x, offset_y) = (render_bounds.x, render_bounds.y);
                let click_count = self.mouse.click_count;
                if let Some(mut tab) = self.active_tab_session_mut() {
                    let hit_pos = tab.hit_test_position(px, py, offset_x, offset_y);
                    if let Some((li, cp)) = hit_pos {
                        let anchor = tab.selection_cursor().unwrap_or((li, cp));
                        if click_count >= 3 {
                            if let Some((line_start, line_end)) = tab.line_range_at_pos(li, cp) {
                                if (li, cp) >= anchor {
                                    tab.send_message(ui::plugin::PluginMessage::SetSelCursor(
                                        Some(line_end),
                                    ));
                                } else {
                                    tab.send_message(ui::plugin::PluginMessage::SetSelCursor(
                                        Some(line_start),
                                    ));
                                }
                            }
                        } else if click_count >= 2 {
                            if let Some((word_start, word_end)) = tab.word_range_at_pos(li, cp) {
                                if (li, cp) >= anchor {
                                    tab.send_message(ui::plugin::PluginMessage::SetSelCursor(
                                        Some(word_end),
                                    ));
                                } else {
                                    tab.send_message(ui::plugin::PluginMessage::SetSelCursor(
                                        Some(word_start),
                                    ));
                                }
                            }
                        } else {
                            tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                                li, cp,
                            ))));
                        }
                    }
                    return AppEffect::REDRAW;
                }
            }
            AppEffect::NONE
        } else if is_custom_renderer {
            // WYSIWYG drag: query semantic target first, then retain byte hit testing.
            if let Some(mut tab) = self.active_tab_session_mut() {
                tab.send_message(PluginMessage::SetCanvasPointer(Some(CanvasPoint::new(px, py))));
            }
            if self.mouse.canvas_drag.is_some() {
                return self.dispatch_canvas_drag_moved(px, py);
            }
            self.sync_plugin_state();
            if !self.mouse.is_down {
                return AppEffect::REDRAW;
            }
            if self.mouse.is_down {
                if !plugin_selection_drag_started(&self.mouse, px, py) {
                    return AppEffect::NONE;
                }
                let selection_anchor = self
                    .active_tab_session()
                    .and_then(|tab| tab.document.cursor().selection_anchor);
                match self.query_plugin_edit_hit_target(px, py) {
                    Some(Some(EditHitTarget::TextCaret { byte_offset, selection_scope })) => {
                        if !semantic_drag_stays_within_scope(
                            self.mouse.wysiwyg_selection_scope.as_ref(),
                            selection_scope.as_ref(),
                        ) {
                            return AppEffect::NONE;
                        }
                        if let Some(mut tab) = self.active_tab_session_mut() {
                            set_wysiwyg_cursor_and_selection(
                                &mut tab,
                                byte_offset,
                                selection_anchor,
                            );
                            return AppEffect::REDRAW;
                        }
                        return AppEffect::NONE;
                    }
                    Some(Some(EditHitTarget::SourceObject { .. } | EditHitTarget::ClearFocus))
                    | Some(None) => return AppEffect::NONE,
                    Some(Some(EditHitTarget::CanvasControl { .. })) => return AppEffect::NONE,
                    None => {}
                }
                let Some(byte) = self.hit_test_plugin_byte_from_point(px, py) else {
                    return AppEffect::NONE;
                };
                if let Some(mut tab) = self.active_tab_session_mut() {
                    set_wysiwyg_cursor_and_selection(&mut tab, byte, selection_anchor);
                    return AppEffect::REDRAW;
                }
            }
            AppEffect::NONE
        } else {
            if self.handle_editor_cursor_moved(px, py, hit) {
                return AppEffect::REDRAW;
            }
            AppEffect::NONE
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::app_effect::AppEffect;
    use crate::dispatch::wysiwyg::semantic_test_support::{
        RecordedSyncMessage, SemanticPluginState, SemanticQueryResponse, app_with_semantic_plugin,
    };
    use crate::document_view::DocumentView;

    use std::cell::RefCell;
    use std::rc::Rc;
    use ui::canvas::CanvasPoint;
    use ui::plugin::{
        CanvasDragPhase, CanvasDragRequest, CanvasDragResponse, EditHitTarget, EditPlan,
        EditTransaction, PluginMessage, PluginQuery, PluginResponse, ViewPlugin,
    };
    use winit::event::ElementState;

    #[derive(Clone, Debug)]
    struct CanvasDragTestState {
        hit_target: EditHitTarget,
        responses: Vec<(CanvasDragPhase, CanvasDragResponse)>,
        requests: Vec<CanvasDragRequest>,
    }

    impl CanvasDragTestState {
        fn with_response(response: CanvasDragResponse) -> Self {
            Self {
                hit_target: EditHitTarget::SourceObject { source_range: 1..2 },
                responses: vec![(CanvasDragPhase::Drop, response)],
                requests: Vec::new(),
            }
        }

        fn response_for(&self, phase: CanvasDragPhase) -> CanvasDragResponse {
            self.responses
                .iter()
                .find_map(|(configured_phase, response)| {
                    (*configured_phase == phase).then(|| response.clone())
                })
                .unwrap_or(CanvasDragResponse::Ignore)
        }

        fn request_count(&self, phase: CanvasDragPhase) -> usize {
            self.requests.iter().filter(|request| request.phase == phase).count()
        }
    }

    struct CanvasDragTestPlugin {
        state: Rc<RefCell<CanvasDragTestState>>,
        plugin_name: &'static str,
    }

    impl ViewPlugin for CanvasDragTestPlugin {
        fn name(&self) -> &str {
            self.plugin_name
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn allows_editing(&self) -> bool {
            true
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }

        fn query(&self, query: PluginQuery, _doc: &dyn core::document::DocView) -> PluginResponse {
            match query {
                PluginQuery::NeedsSourceUpdate(_) => PluginResponse::Bool(false),
                PluginQuery::HitTestEditTarget { .. } => {
                    PluginResponse::EditHitTarget(Some(self.state.borrow().hit_target.clone()))
                }
                PluginQuery::ContentHeight => PluginResponse::Float(1_000.0),
                _ => PluginResponse::None,
            }
        }

        fn handle_message(
            &mut self,
            _message: PluginMessage,
            _doc: &mut dyn core::document::DocViewMut,
        ) -> bool {
            false
        }

        fn handle_canvas_drag(
            &mut self,
            request: CanvasDragRequest,
            _doc: &dyn core::document::DocView,
        ) -> CanvasDragResponse {
            let mut state = self.state.borrow_mut();
            let response = state.response_for(request.phase);
            state.requests.push(request);
            response
        }
    }

    struct CanvasControlTestState {
        edit_plan: EditPlan,
        hit_target: EditHitTarget,
        planned_control_ranges: Vec<std::ops::Range<usize>>,
        pointer_positions: Vec<Option<CanvasPoint>>,
        drag_requests: Vec<CanvasDragRequest>,
    }

    struct CanvasControlTestPlugin {
        state: Rc<RefCell<CanvasControlTestState>>,
    }

    impl ViewPlugin for CanvasControlTestPlugin {
        fn name(&self) -> &str {
            "canvas_control_test"
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn allows_editing(&self) -> bool {
            true
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }

        fn query(&self, query: PluginQuery, _doc: &dyn core::document::DocView) -> PluginResponse {
            match query {
                PluginQuery::NeedsSourceUpdate(_) => PluginResponse::Bool(false),
                PluginQuery::HitTestEditTarget { .. } => {
                    PluginResponse::EditHitTarget(Some(self.state.borrow().hit_target.clone()))
                }
                PluginQuery::PlanCanvasControl { source_range, .. } => {
                    let mut state = self.state.borrow_mut();
                    state.planned_control_ranges.push(source_range);
                    PluginResponse::EditPlan(state.edit_plan.clone())
                }
                PluginQuery::ContentHeight => PluginResponse::Float(1_000.0),
                _ => PluginResponse::None,
            }
        }

        fn handle_message(
            &mut self,
            message: PluginMessage,
            _doc: &mut dyn core::document::DocViewMut,
        ) -> bool {
            if let PluginMessage::SetCanvasPointer(point) = message {
                self.state.borrow_mut().pointer_positions.push(point);
                return true;
            }
            false
        }

        fn handle_canvas_drag(
            &mut self,
            request: CanvasDragRequest,
            _doc: &dyn core::document::DocView,
        ) -> CanvasDragResponse {
            self.state.borrow_mut().drag_requests.push(request);
            CanvasDragResponse::Ignore
        }
    }

    fn app_with_canvas_drag_plugin(text: &str, state: Rc<RefCell<CanvasDragTestState>>) -> App {
        let mut app = App::new(None);
        let document = DocumentView::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(
            document,
            Box::new(CanvasDragTestPlugin { state, plugin_name: "canvas_drag_test" }),
        );
        app.switch_workspace_for_test(0);
        app
    }

    fn app_with_mindmap_drag_plugin(text: &str, state: Rc<RefCell<CanvasDragTestState>>) -> App {
        let mut app = App::new(None);
        let document = DocumentView::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(
            document,
            Box::new(CanvasDragTestPlugin { state, plugin_name: ui::plugin::PLUGIN_MINDMAP }),
        );
        app.switch_workspace_for_test(0);
        app
    }

    fn app_with_canvas_control_plugin(
        text: &str,
        state: Rc<RefCell<CanvasControlTestState>>,
    ) -> App {
        let mut app = App::new(None);
        let document = DocumentView::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(CanvasControlTestPlugin { state }));
        app.switch_workspace_for_test(0);
        app
    }

    fn active_text(app: &App) -> String {
        app.active_tab_session().expect("active entry").document.full_text()
    }

    #[cfg(feature = "markdown")]
    fn render_mindmap_canvas_for_control_test(app: &mut App) -> ui::core::paint::DrawList {
        let font_size = app.ui_metrics().font_size;
        let dpi = app.ui_metrics().dpi;
        let theme = app.current_theme.clone();
        let mut shaper = app
            .editor_runtime
            .new_shaper(font_size, "")
            .unwrap_or_else(|| shaping::Shaper::new().expect("test shaper should initialize"));
        let snapshot = app
            .sync_and_prepare_canvas_frame(&mut shaper)
            .expect("mindmap source must resolve a canvas viewport");
        let mut tab = app.active_tab_session_mut().expect("active mindmap tab");
        tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi)
    }

    #[test]
    fn canvas_control_press_applies_edit_plan_without_starting_drag() {
        let state = Rc::new(RefCell::new(CanvasControlTestState {
            edit_plan: EditPlan::Apply(EditTransaction::replace(1, 1..2, "TRUE".into(), 5)),
            hit_target: EditHitTarget::CanvasControl { source_range: 1..4 },
            planned_control_ranges: Vec::new(),
            pointer_positions: Vec::new(),
            drag_requests: Vec::new(),
        }));
        let mut app = app_with_canvas_control_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );

        let state = state.borrow();
        assert_eq!(state.planned_control_ranges, vec![1..4]);
        assert!(state.drag_requests.is_empty());
        assert_eq!(active_text(&app), "aTRUEc");
        assert!(!app.mouse.is_down);
        assert!(app.mouse.canvas_drag.is_none());
    }

    #[test]
    fn canvas_control_pointer_move_notifies_plugin() {
        let state = Rc::new(RefCell::new(CanvasControlTestState {
            edit_plan: EditPlan::Consume,
            hit_target: EditHitTarget::CanvasControl { source_range: 1..4 },
            planned_control_ranges: Vec::new(),
            pointer_positions: Vec::new(),
            drag_requests: Vec::new(),
        }));
        let mut app = app_with_canvas_control_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();
        let point = CanvasPoint::new(bounds.x + 12.0, bounds.y + 20.0);

        let effect = app.dispatch_editor_cursor_moved(point.x, point.y, None);

        assert_eq!(state.borrow().pointer_positions, vec![Some(point)]);
        assert_eq!(effect, AppEffect::REDRAW);
    }

    #[test]
    #[cfg(feature = "markdown")]
    fn canvas_control_end_to_end() {
        let source = "# Root\n## Parent\n### Child\n#### Grandchild";
        let tree = textora_markdown::mmf::parser::parse(source).expect("mmap fixture must parse");
        let parent_range = tree.root.children[0].subtree_source_range.clone();
        let root_range = tree.root.subtree_source_range.clone();
        let child_range = tree.root.children[0].children[0].subtree_source_range.clone();

        let mut app = App::new(None);
        let document = DocumentView::new(source.lines().map(str::to_owned).collect(), 80, 10.0);
        app.push_entry_for_test(
            document,
            Box::new(textora_markdown::mindmap_view::MindmapView::new()),
        );
        app.switch_workspace_for_test(0);
        let _ = render_mindmap_canvas_for_control_test(&mut app);

        let bounds = app.plugin_render_bounds();
        let mut parent_control_point = None;
        let mut root_control_seen = false;
        for y in (0..=bounds.h.ceil() as usize).step_by(4) {
            for x in (0..=bounds.w.ceil() as usize).step_by(4) {
                let point = (bounds.x + x as f32, bounds.y + y as f32);
                let target = app.active_tab_session().expect("active mindmap tab").query(
                    PluginQuery::HitTestEditTarget {
                        x: point.0,
                        y: point.1,
                        offset_x: bounds.x,
                        offset_y: bounds.y,
                    },
                );
                let PluginResponse::EditHitTarget(Some(EditHitTarget::CanvasControl {
                    source_range,
                })) = target
                else {
                    continue;
                };
                if source_range == parent_range {
                    parent_control_point = Some(point);
                }
                if source_range == root_range {
                    root_control_seen = true;
                }
            }
        }
        assert!(parent_control_point.is_some(), "parent control must be hit-testable");
        assert!(!root_control_seen, "root must not expose a collapse control");

        let (control_x, control_y) = parent_control_point.expect("parent control point");
        app.dispatch_editor_mouse_input(ElementState::Pressed, control_x, control_y, None);
        let collapsed_source = active_text(&app);
        assert!(collapsed_source.contains("collapsed = true"));

        let draw_list = render_mindmap_canvas_for_control_test(&mut app);
        let rendered_text = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(rendered_text.contains(&"Parent"));
        assert!(rendered_text.contains(&"2"));
        assert!(!rendered_text.contains(&" · 2"));

        let mut child_target_seen = false;
        let bounds = app.plugin_render_bounds();
        for y in (0..=bounds.h.ceil() as usize).step_by(4) {
            for x in (0..=bounds.w.ceil() as usize).step_by(4) {
                let point = (bounds.x + x as f32, bounds.y + y as f32);
                let target = app.active_tab_session().expect("active mindmap tab").query(
                    PluginQuery::HitTestEditTarget {
                        x: point.0,
                        y: point.1,
                        offset_x: bounds.x,
                        offset_y: bounds.y,
                    },
                );
                if matches!(
                    target,
                    PluginResponse::EditHitTarget(Some(EditHitTarget::SourceObject {
                        source_range,
                    })) if source_range == child_range
                ) {
                    child_target_seen = true;
                }
            }
        }
        assert!(!child_target_seen, "collapsed descendants must not be drag candidates");
    }

    #[test]
    fn canvas_drag_starts_after_threshold_and_applies_one_drop_transaction() {
        let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
            CanvasDragResponse::Apply(EditTransaction::replace(1, 1..2, "Z".into(), 2)),
        )));
        let mut app = app_with_mindmap_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        let Some(entry) = app.active_tab_session_mut() else {
            panic!("canvas drag test requires an active entry");
        };
        entry.document.cursor_move_to_offset(2);
        entry.document.cursor_mut().selection_anchor = Some(1);

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 6.0, bounds.y + 6.0, None);
        assert!(state.borrow().requests.is_empty());

        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
        app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
            None,
        );

        assert_eq!(active_text(&app), "aZc");
        assert_eq!(state.borrow().request_count(CanvasDragPhase::Drop), 1);
        let state = state.borrow();
        assert!(state.requests.iter().all(|request| {
            request.pressed_x == bounds.x + 4.0 && request.pressed_y == bounds.y + 4.0
        }));
    }

    #[test]
    fn mindmap_canvas_drag_starts_without_prior_selection() {
        let state =
            Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
        let mut app = app_with_mindmap_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);

        assert_eq!(state.borrow().request_count(CanvasDragPhase::Start), 1);
    }

    #[test]
    fn canvas_drag_does_not_start_for_text_caret_press() {
        let state = Rc::new(RefCell::new(CanvasDragTestState {
            hit_target: EditHitTarget::TextCaret { byte_offset: 1, selection_scope: None },
            responses: Vec::new(),
            requests: Vec::new(),
        }));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );

        assert!(app.mouse.canvas_drag.is_none());
    }

    #[test]
    fn canvas_drag_ignore_does_not_modify_document() {
        let state =
            Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
        app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
            None,
        );

        assert_eq!(active_text(&app), "abc");
    }

    #[test]
    fn canvas_drag_cancel_sends_cancel_without_executing_transaction() {
        let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
            CanvasDragResponse::Apply(EditTransaction::replace(0, 1..2, "Z".into(), 2)),
        )));
        let mut app = app_with_canvas_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
        app.cancel_canvas_drag();

        assert_eq!(active_text(&app), "abc");
        assert_eq!(state.borrow().request_count(CanvasDragPhase::Cancel), 1);
    }

    #[test]
    fn canvas_drag_apply_before_drop_does_not_modify_document() {
        let transaction = EditTransaction::replace(1, 1..2, "Z".into(), 2);
        let state = Rc::new(RefCell::new(CanvasDragTestState {
            hit_target: EditHitTarget::SourceObject { source_range: 1..2 },
            responses: vec![
                (CanvasDragPhase::Start, CanvasDragResponse::Apply(transaction.clone())),
                (CanvasDragPhase::Update, CanvasDragResponse::Apply(transaction.clone())),
                (CanvasDragPhase::Cancel, CanvasDragResponse::Apply(transaction)),
            ],
            requests: Vec::new(),
        }));
        let mut app = app_with_canvas_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
        assert_eq!(active_text(&app), "abc");

        app.dispatch_editor_cursor_moved(bounds.x + 30.0, bounds.y + 30.0, None);
        assert_eq!(active_text(&app), "abc");

        app.cancel_canvas_drag();

        assert_eq!(active_text(&app), "abc");
        let state = state.borrow();
        assert_eq!(state.request_count(CanvasDragPhase::Start), 1);
        assert_eq!(state.request_count(CanvasDragPhase::Update), 1);
        assert_eq!(state.request_count(CanvasDragPhase::Cancel), 1);
    }

    #[test]
    fn canvas_drag_rejects_stale_generation_transaction() {
        let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
            CanvasDragResponse::Apply(EditTransaction::replace(0, 1..2, "Z".into(), 2)),
        )));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
        app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
            None,
        );

        assert_eq!(active_text(&app), "abc");
    }

    #[test]
    fn canvas_drag_release_clears_session() {
        let state =
            Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
        app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
            None,
        );

        assert!(app.mouse.canvas_drag.is_none());
    }

    #[test]
    fn source_object_drag_ignore_keeps_object_selection() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(Some(EditHitTarget::SourceObject {
                source_range: 1..5,
            })),
            hit_test_byte_result: Some(3),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        let bounds = app.plugin_render_bounds();
        let press_x = bounds.x + 10.0;
        let press_y = bounds.y + 10.0;

        app.dispatch_editor_mouse_input(ElementState::Pressed, press_x, press_y, None);
        state.borrow_mut().hit_target_response =
            SemanticQueryResponse::Target(Some(EditHitTarget::TextCaret {
                byte_offset: 4,
                selection_scope: None,
            }));
        app.dispatch_editor_cursor_moved(press_x + 10.0, press_y, None);

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.selection_range(), Some((1, 5)));
        let state = state.borrow();
        assert_eq!(state.queried_operations, ["hit_target"]);
        assert!(state.sync_messages.ends_with(&[
            RecordedSyncMessage::Anchor(Some(1)),
            RecordedSyncMessage::SelectionCursor(Some(5)),
            RecordedSyncMessage::Cursor(5),
        ]));
    }

    #[test]
    fn mouse_semantic_none_consumes_press_without_byte_fallback() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(None),
            hit_test_byte_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        {
            let entry = app.active_tab_session_mut().expect("active entry");
            entry.document.cursor_move_to_offset(3);
            entry.document.cursor_mut().selection_anchor = Some(1);
        }
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 10.0,
            bounds.y + 10.0,
            None,
        );

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.selection_range(), Some((1, 3)));
        let state = state.borrow();
        assert_eq!(state.queried_operations, ["hit_target"]);
        assert!(!state.sync_messages.contains(&RecordedSyncMessage::Cursor(4)));
    }

    #[test]
    fn mouse_uses_byte_hit_fallback_when_semantic_query_is_unsupported() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_test_byte_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 10.0,
            bounds.y + 10.0,
            None,
        );

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.cursor_offset().to_usize(), 4);
        assert_eq!(entry.document.cursor().selection_anchor, Some(4));
        let state = state.borrow();
        assert!(state.queried_operations.starts_with(&["hit_target", "hit_byte"]));
        assert!(state.sync_messages.contains(&RecordedSyncMessage::Cursor(4)));
    }

    #[test]
    fn semantic_text_caret_press_keeps_an_anchor_for_drag_selection() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(Some(EditHitTarget::TextCaret {
                byte_offset: 1,
                selection_scope: None,
            })),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state);
        let bounds = app.plugin_render_bounds();

        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 10.0,
            bounds.y + 10.0,
            None,
        );

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.cursor_offset().to_usize(), 1);
        assert_eq!(entry.document.cursor().selection_anchor, Some(1));
    }

    #[test]
    fn semantic_text_caret_double_click_selects_its_source_word() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(Some(EditHitTarget::TextCaret {
                byte_offset: 1,
                selection_scope: None,
            })),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("title other", state);
        let bounds = app.plugin_render_bounds();
        let x = bounds.x + 10.0;
        let y = bounds.y + 10.0;

        app.dispatch_editor_mouse_input(ElementState::Pressed, x, y, None);
        app.dispatch_editor_mouse_input(ElementState::Released, x, y, None);
        app.dispatch_editor_mouse_input(ElementState::Pressed, x, y, None);

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.selection_range(), Some((0, "title".len())));
    }

    #[test]
    fn expanded_wysiwyg_hit_point_uses_cursor_rect_when_available() {
        let hit_point = super::expanded_wysiwyg_hit_point(
            40.0,
            12.0,
            Some((120.0, 24.0, 2.0, 18.0)),
            10.0,
            30.0,
        );

        assert_eq!(hit_point, (130.0, 63.0));
    }

    #[test]
    fn expanded_wysiwyg_hit_point_falls_back_to_mouse_point_without_cursor_rect() {
        let hit_point = super::expanded_wysiwyg_hit_point(40.0, 12.0, None, 10.0, 30.0);

        assert_eq!(hit_point, (40.0, 12.0));
    }
}
