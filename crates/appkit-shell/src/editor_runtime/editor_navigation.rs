//! Shared visual keyboard navigation and plugin caret synchronization.

use appkit_core::{document::DocumentModel, workspace::types::TabId};
use textora_core::types::ByteIndex;
use ui::plugin::{
    CursorAffinity, EditHitTarget, MoveDirection, PluginMessage, PluginQuery, PluginResponse,
    VisualCaretPosition,
};

use super::{EditorOutcome, EditorRuntime};
use crate::event::ShellEffect;
use crate::tab_session::{TabSession, TabSessionMut};
use appkit_core::edit_command::EditCommand;

fn wysiwyg_cursor_x(tab: &TabSession<'_>, byte: usize) -> Option<f32> {
    tab.query_cursor_screen_rect(byte).map(|(x, _y, _w, _h)| x)
}

fn query_visual_caret_move(
    tab: &TabSession<'_>,
    current_byte: usize,
    direction: MoveDirection,
    target_x: Option<f32>,
) -> Option<VisualCaretPosition> {
    if let PluginResponse::VisualCaretPosition(position) =
        tab.query(PluginQuery::VisualMoveCaret { current_byte, direction, target_x })
    {
        return position;
    }
    tab.visual_move_byte(current_byte, direction, target_x).map(|byte_offset| VisualCaretPosition {
        byte_offset,
        affinity: CursorAffinity::Downstream,
    })
}

pub fn query_visual_caret_hit(
    tab: &TabSession<'_>,
    x: f32,
    y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<VisualCaretPosition> {
    if let PluginResponse::VisualCaretPosition(position) =
        tab.query(PluginQuery::HitTestVisualCaret { x, y, offset_x, offset_y })
    {
        return position;
    }
    tab.hit_test_byte(x, y, offset_x, offset_y).map(|byte_offset| VisualCaretPosition {
        byte_offset,
        affinity: CursorAffinity::Downstream,
    })
}

impl EditorRuntime {
    /// Execute visual navigation using the host's viewport heights in physical pixels.
    pub fn execute_visual_navigation(
        &mut self,
        command: &EditCommand,
        reveal_viewport_height: f32,
        page_viewport_height: f32,
    ) -> ShellEffect {
        let Some(tab_id) = self.active_tab_id() else {
            return ShellEffect::NONE;
        };
        let extend_selection = matches!(
            command,
            EditCommand::ExtendLeft
                | EditCommand::ExtendRight
                | EditCommand::ExtendUp
                | EditCommand::ExtendDown
                | EditCommand::ExtendToLineStart
                | EditCommand::ExtendToLineEnd
                | EditCommand::ExtendToDocStart
                | EditCommand::ExtendToDocEnd
        );
        let direction = match command {
            EditCommand::MoveLeft | EditCommand::ExtendLeft => MoveDirection::Left,
            EditCommand::MoveRight | EditCommand::ExtendRight => MoveDirection::Right,
            EditCommand::MoveUp | EditCommand::ExtendUp => MoveDirection::Up,
            EditCommand::MoveDown | EditCommand::ExtendDown => MoveDirection::Down,
            EditCommand::MoveToLineStart | EditCommand::ExtendToLineStart => {
                MoveDirection::LineStart
            }
            EditCommand::MoveToLineEnd | EditCommand::ExtendToLineEnd => MoveDirection::LineEnd,
            EditCommand::MoveToDocStart | EditCommand::ExtendToDocStart => {
                return self.navigate_visual_doc_boundary(
                    tab_id,
                    0,
                    extend_selection,
                    reveal_viewport_height,
                );
            }
            EditCommand::MoveToDocEnd | EditCommand::ExtendToDocEnd => {
                let Some(tab) = self.tab_session(tab_id) else {
                    return ShellEffect::NONE;
                };
                let end = tab.document.buffer_len();
                return self.navigate_visual_doc_boundary(
                    tab_id,
                    end,
                    extend_selection,
                    reveal_viewport_height,
                );
            }
            EditCommand::PageUp => {
                return self.page_visual_editor(
                    tab_id,
                    -page_viewport_height,
                    page_viewport_height,
                );
            }
            EditCommand::PageDown => {
                return self.page_visual_editor(tab_id, page_viewport_height, page_viewport_height);
            }
            _ => return ShellEffect::NONE,
        };
        self.move_visual_caret(tab_id, direction, extend_selection, reveal_viewport_height)
    }

    fn move_visual_caret(
        &mut self,
        tab_id: TabId,
        direction: MoveDirection,
        extend_selection: bool,
        reveal_viewport_height: f32,
    ) -> ShellEffect {
        let vertical_navigation = matches!(direction, MoveDirection::Up | MoveDirection::Down);
        let Some(tab) = self.tab_session(tab_id) else {
            return ShellEffect::NONE;
        };

        let current_byte = tab.document.cursor_offset().to_usize();
        let vertical_anchor_x = if vertical_navigation {
            self.preferred_x().or_else(|| wysiwyg_cursor_x(&tab, current_byte))
        } else {
            None
        };
        let selection_anchor = extend_selection
            .then(|| tab.document.cursor().selection_anchor.unwrap_or(current_byte));
        let semantic_target = tab.move_edit_target(current_byte, direction, vertical_anchor_x);
        let visual_target = if semantic_target.is_none() {
            query_visual_caret_move(&tab, current_byte, direction, vertical_anchor_x)
        } else {
            None
        };

        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return ShellEffect::NONE;
        };
        if let Some(target) = semantic_target {
            apply_edit_hit_target(&mut tab, target);
        } else if let Some(position) = visual_target {
            set_wysiwyg_visual_caret_and_selection(&mut tab, position, selection_anchor);
        } else {
            return ShellEffect::NONE;
        }
        record_visual_navigation(&mut tab);
        let cursor_after = tab.document.cursor_offset().to_usize();

        let resolved_anchor_x = if vertical_navigation {
            vertical_anchor_x.or_else(|| {
                self.tab_session(tab_id).and_then(|tab| wysiwyg_cursor_x(&tab, cursor_after))
            })
        } else {
            None
        };
        self.set_preferred_x(resolved_anchor_x);
        self.reveal_visual_caret(reveal_viewport_height);

        ShellEffect::REDRAW
    }

    /// Ask the active plugin to reveal its caret after layout, with a geometry fallback.
    pub fn reveal_visual_caret(&mut self, viewport_height: f32) {
        let Some(mut tab) = self.active_tab_id().and_then(|tab_id| self.tab_session_mut(tab_id))
        else {
            return;
        };
        if !tab.handles_own_rendering() {
            return;
        }
        let cursor_byte = tab.document.cursor_offset().to_usize();
        if tab.send_message(PluginMessage::RevealCursor(cursor_byte)) {
            return;
        }
        let cursor_rect = TabSession::new(tab.id, &*tab.document, &*tab.runtime)
            .query_cursor_screen_rect(cursor_byte);
        let Some((_, cursor_y, _, cursor_height)) = cursor_rect else {
            return;
        };
        if let Some(delta) = super::editor_painter::plugin_cursor_visibility_scroll_delta(
            cursor_y,
            cursor_height,
            viewport_height,
        ) {
            tab.send_message(PluginMessage::Scroll { delta, viewport_h: viewport_height });
        }
    }

    fn navigate_visual_doc_boundary(
        &mut self,
        tab_id: TabId,
        byte: usize,
        extend_selection: bool,
        viewport_height: f32,
    ) -> ShellEffect {
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return ShellEffect::NONE;
        };
        let current_byte = tab.document.cursor_offset().to_usize();
        let selection_anchor = extend_selection
            .then(|| tab.document.cursor().selection_anchor.unwrap_or(current_byte));
        set_wysiwyg_cursor_and_selection(&mut tab, byte, selection_anchor);
        record_visual_navigation(&mut tab);
        self.set_preferred_x(None);
        self.reveal_visual_caret(viewport_height);
        ShellEffect::REDRAW
    }

    fn page_visual_editor(
        &mut self,
        tab_id: TabId,
        delta: f32,
        viewport_height: f32,
    ) -> ShellEffect {
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return ShellEffect::NONE;
        };
        tab.send_message(PluginMessage::Scroll { delta, viewport_h: viewport_height });

        let Some(tab) = self.tab_session(tab_id) else {
            return ShellEffect::NONE;
        };
        let position = query_visual_caret_hit(&tab, 0.0, 0.0, 0.0, 0.0).unwrap_or_else(|| {
            VisualCaretPosition {
                byte_offset: tab.document.cursor_offset().to_usize(),
                affinity: tab.document.tb().cursor_affinity(),
            }
        });
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return ShellEffect::NONE;
        };
        set_wysiwyg_visual_caret_and_selection(&mut tab, position, None);
        record_visual_navigation(&mut tab);
        self.set_preferred_x(None);
        ShellEffect::REDRAW
    }

    pub(super) fn navigate_active_visual_editor_for_key(
        &mut self,
        key: ui::KeyCode,
        modifiers: ui::core::Modifiers,
    ) -> Option<EditorOutcome> {
        let command = visual_navigation_command(key, modifiers)?;
        let editor_bounds = self.painted_editor_bounds()?;
        let tab = self.tab_session(self.active_tab_id()?)?;
        if !tab.allows_editing() || !tab.handles_own_rendering() || tab.is_canvas() {
            return None;
        }

        if matches!(command, EditCommand::PageUp | EditCommand::PageDown)
            && tab.query_cursor_screen_rect(tab.document.cursor_offset().to_usize()).is_none()
        {
            return None;
        }

        let viewport_height =
            super::editor_painter::plugin_bounds(editor_bounds, self.scale_factor() as f32, false)
                .h;
        if viewport_height <= 0.0 {
            return None;
        }
        let shell_effect =
            self.execute_visual_navigation(&command, viewport_height, viewport_height);
        shell_effect.redraw.then_some(EditorOutcome { shell_effect, ..EditorOutcome::default() })
    }
}

/// Matches the source editor's key precedence; word motion remains source-based.
fn visual_navigation_command(
    key: ui::KeyCode,
    modifiers: ui::core::Modifiers,
) -> Option<EditCommand> {
    use ui::KeyCode;

    let command = match (key, modifiers.shift, modifiers.cmd || modifiers.ctrl, modifiers.alt) {
        (KeyCode::Left | KeyCode::Right, _, _, true) => return None,
        (KeyCode::Left, true, true, _) => EditCommand::ExtendToLineStart,
        (KeyCode::Right, true, true, _) => EditCommand::ExtendToLineEnd,
        (KeyCode::Up, true, true, _) => EditCommand::ExtendToDocStart,
        (KeyCode::Down, true, true, _) => EditCommand::ExtendToDocEnd,
        (KeyCode::Left, true, false, _) => EditCommand::ExtendLeft,
        (KeyCode::Right, true, false, _) => EditCommand::ExtendRight,
        (KeyCode::Up, true, false, _) => EditCommand::ExtendUp,
        (KeyCode::Down, true, false, _) => EditCommand::ExtendDown,
        (KeyCode::Left, false, true, _) | (KeyCode::Home, false, _, _) => {
            EditCommand::MoveToLineStart
        }
        (KeyCode::Right, false, true, _) | (KeyCode::End, false, _, _) => {
            EditCommand::MoveToLineEnd
        }
        (KeyCode::Up, false, true, _) => EditCommand::MoveToDocStart,
        (KeyCode::Down, false, true, _) => EditCommand::MoveToDocEnd,
        (KeyCode::Left, false, false, _) => EditCommand::MoveLeft,
        (KeyCode::Right, false, false, _) => EditCommand::MoveRight,
        (KeyCode::Up, false, false, _) => EditCommand::MoveUp,
        (KeyCode::Down, false, false, _) => EditCommand::MoveDown,
        (KeyCode::PageUp, false, _, _) => EditCommand::PageUp,
        (KeyCode::PageDown, false, _, _) => EditCommand::PageDown,
        _ => return None,
    };
    Some(command)
}

fn record_visual_navigation(tab: &mut TabSessionMut<'_>) {
    tab.document.break_edit_merge();
    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
}

pub fn set_wysiwyg_cursor_and_selection(
    tab: &mut TabSessionMut<'_>,
    requested_cursor_byte: usize,
    selection_anchor: Option<usize>,
) -> usize {
    tab.document.cursor_move_to_offset(requested_cursor_byte);
    let snapped_cursor_byte = tab.document.cursor_offset().to_usize();
    tab.document.cursor_mut().selection_anchor = selection_anchor;
    tab.send_message(PluginMessage::SetCursorByte(snapped_cursor_byte));
    tab.send_message(PluginMessage::SetVisualCaret(VisualCaretPosition {
        byte_offset: snapped_cursor_byte,
        affinity: CursorAffinity::Downstream,
    }));
    tab.send_message(PluginMessage::SetSelAnchorByte(selection_anchor));
    tab.send_message(PluginMessage::SetSelCursorByte(
        selection_anchor.map(|_| snapped_cursor_byte),
    ));
    snapped_cursor_byte
}

pub fn set_wysiwyg_visual_caret_and_selection(
    tab: &mut TabSessionMut<'_>,
    position: VisualCaretPosition,
    selection_anchor: Option<usize>,
) {
    let snapped_byte =
        set_wysiwyg_cursor_and_selection(tab, position.byte_offset, selection_anchor);
    if snapped_byte != position.byte_offset {
        return;
    }
    tab.document.tb.set_cursor_affinity(position.affinity);
    tab.send_message(PluginMessage::SetVisualCaret(position));
}

pub fn apply_target_to_document(doc: &mut DocumentModel, target: EditHitTarget) {
    match target {
        EditHitTarget::TextCaret { byte_offset, .. } => {
            doc.cursor_move_to_offset(byte_offset);
            doc.cursor_mut().selection_anchor = None;
        }
        EditHitTarget::SourceObject { source_range } => {
            let Some(source_range) = validated_source_object_range(doc, source_range) else {
                return;
            };
            doc.cursor_move_to_offset(source_range.end);
            doc.cursor_mut().selection_anchor = Some(source_range.start);
        }
        EditHitTarget::CanvasControl { .. } => {}
        EditHitTarget::ClearFocus => {
            doc.cursor_move_to_offset(doc.buffer_len());
            doc.cursor_mut().selection_anchor = None;
        }
    }
}

fn validated_source_object_range(
    doc: &DocumentModel,
    source_range: std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    if source_range.start > source_range.end || source_range.end > doc.buffer_len() {
        return None;
    }

    let source = doc.full_text();
    if !source.is_char_boundary(source_range.start) || !source.is_char_boundary(source_range.end) {
        return None;
    }

    let start = ByteIndex(source_range.start);
    let end = ByteIndex(source_range.end);
    if !doc.tb().is_grapheme_boundary(start) || !doc.tb().is_grapheme_boundary(end) {
        return None;
    }

    Some(source_range)
}

pub fn apply_edit_hit_target(tab: &mut TabSessionMut<'_>, target: EditHitTarget) {
    let clears_edit_focus = matches!(target, EditHitTarget::ClearFocus);
    apply_target_to_document(tab.document, target);
    let cursor_byte = tab.document.cursor_offset().to_usize();
    let selection_anchor = tab.document.cursor().selection_anchor;
    tab.send_message(PluginMessage::SetSelAnchorByte(selection_anchor));
    tab.send_message(PluginMessage::SetSelCursorByte(Some(cursor_byte)));
    tab.send_message(PluginMessage::SetCursorByte(cursor_byte));
    if clears_edit_focus {
        tab.send_message(PluginMessage::ClearEditFocus);
    }
}
