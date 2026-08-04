//! 产品无关的编辑器指针命中与选择会话。

use ui::plugin::EditHitTarget;

use super::{EditorInputContext, EditorOutcome, EditorRuntime, MouseCapture};
use crate::event::ShellEffect;
use crate::tab_session::TabSessionMut;

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
                if !self.begin_text_selection(context)
                    || !self.place_pointer_selection(context.editor_rect, position, false)
                {
                    self.end_pointer_capture();
                    return EditorOutcome::default();
                }
                redraw_outcome()
            }
            PointerPhase::Move
                if self.pointer_capture() == MouseCapture::TextSelection
                    && self.pointer_input_allowed(context, position) =>
            {
                self.place_pointer_selection(context.editor_rect, position, true)
                    .then(redraw_outcome)
                    .unwrap_or_default()
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
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        if tab.runtime.plugin.handles_own_rendering() {
            return place_plugin_selection(&mut tab, editor_rect, position, dpi, extend);
        }
        let byte_offset = hit_test_text_byte(&tab, editor_rect, position, &settings, &metrics)
            .unwrap_or_else(|| tab.document.buffer_len());
        place_text_caret(&mut tab, byte_offset, extend);
        true
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

fn place_plugin_selection(
    tab: &mut TabSessionMut<'_>,
    editor_rect: ui::Rect,
    position: (f32, f32),
    dpi: f32,
    extend: bool,
) -> bool {
    let bounds = super::editor_painter::plugin_bounds(editor_rect, dpi, tab.is_canvas());
    match tab.hit_test_edit_target(position.0, position.1, bounds.x, bounds.y) {
        Some(Some(EditHitTarget::TextCaret { byte_offset, .. })) => {
            place_text_caret(tab, byte_offset, extend);
            true
        }
        Some(Some(EditHitTarget::SourceObject { source_range })) if !extend => {
            tab.document.cursor_mut().selection_anchor = Some(source_range.start);
            tab.document.set_cursor_offset_synced(source_range.end);
            true
        }
        Some(Some(EditHitTarget::ClearFocus)) if !extend => {
            tab.document.cursor_mut().selection_anchor = None;
            tab.document.set_cursor_offset_synced(tab.document.buffer_len());
            true
        }
        Some(Some(EditHitTarget::CanvasControl { .. })) | Some(None) => false,
        Some(Some(EditHitTarget::SourceObject { .. } | EditHitTarget::ClearFocus)) => false,
        None => tab
            .hit_test_byte(position.0, position.1, bounds.x, bounds.y)
            .map(|byte_offset| place_text_caret(tab, byte_offset, extend))
            .is_some(),
    }
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
