//! 编辑器输入门与会话状态。

use crate::editor_runtime::{EditorFocus, EditorInputContext};
use crate::mouse_state::{CanvasDragSession, MouseCapture};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreeditUpdate {
    Rejected,
    Unchanged,
    Changed,
}

impl PreeditUpdate {
    pub(crate) fn accepted(self) -> bool {
        !matches!(self, Self::Rejected)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditorInputSession {
    modifiers: winit::keyboard::ModifiersState,
    pointer_capture: MouseCapture,
    canvas_drag_session: Option<CanvasDragSession>,
    preedit_text: String,
    preedit_cursor: Option<(usize, usize)>,
    preferred_x: Option<f32>,
}

impl EditorInputSession {
    pub(crate) fn new() -> Self {
        Self {
            modifiers: winit::keyboard::ModifiersState::default(),
            pointer_capture: MouseCapture::None,
            canvas_drag_session: None,
            preedit_text: String::new(),
            preedit_cursor: None,
            preferred_x: None,
        }
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: winit::keyboard::ModifiersState) {
        self.modifiers = modifiers;
    }

    pub(crate) fn modifiers(&self) -> winit::keyboard::ModifiersState {
        self.modifiers
    }

    pub(crate) fn keyboard_allowed(&self, context: EditorInputContext) -> bool {
        context.focus == EditorFocus::Active && !context.modal_blocked
    }

    pub(crate) fn pointer_allowed(
        &self,
        context: EditorInputContext,
        pointer_inside_editor: bool,
    ) -> bool {
        if context.focus != EditorFocus::Active || context.modal_blocked {
            return false;
        }
        self.pointer_capture != MouseCapture::None || pointer_inside_editor
    }

    pub(crate) fn start_text_selection(&mut self, context: EditorInputContext) -> bool {
        if !self.keyboard_allowed(context) {
            return false;
        }
        self.canvas_drag_session = None;
        self.pointer_capture = MouseCapture::TextSelection;
        true
    }

    pub(crate) fn start_canvas_drag(&mut self, context: EditorInputContext) -> bool {
        if !self.keyboard_allowed(context) {
            return false;
        }
        self.pointer_capture = MouseCapture::CanvasDrag;
        true
    }

    pub(crate) fn start_canvas_drag_session(
        &mut self,
        context: EditorInputContext,
        session: CanvasDragSession,
    ) -> bool {
        if !self.start_canvas_drag(context) {
            return false;
        }
        self.canvas_drag_session = Some(session);
        true
    }

    pub(crate) fn canvas_drag_session_mut(&mut self) -> Option<&mut CanvasDragSession> {
        self.canvas_drag_session.as_mut()
    }

    pub(crate) fn take_canvas_drag_session(&mut self) -> Option<CanvasDragSession> {
        self.canvas_drag_session.take()
    }

    pub(crate) fn end_pointer_capture(&mut self) {
        self.pointer_capture = MouseCapture::None;
        self.canvas_drag_session = None;
    }

    pub(crate) fn pointer_capture(&self) -> MouseCapture {
        self.pointer_capture
    }

    pub(crate) fn set_preferred_x(&mut self, preferred_x: Option<f32>) {
        self.preferred_x = preferred_x;
    }

    pub(crate) fn preferred_x(&self) -> Option<f32> {
        self.preferred_x
    }

    pub(crate) fn update_preedit(
        &mut self,
        context: EditorInputContext,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> PreeditUpdate {
        if !self.keyboard_allowed(context) {
            self.clear_preedit();
            return PreeditUpdate::Rejected;
        }
        if self.preedit_text == text && self.preedit_cursor == cursor {
            return PreeditUpdate::Unchanged;
        }
        self.preedit_text = text;
        self.preedit_cursor = cursor;
        PreeditUpdate::Changed
    }

    pub(crate) fn clear_preedit(&mut self) {
        self.preedit_text.clear();
        self.preedit_cursor = None;
    }

    pub(crate) fn preedit(&self) -> (&str, Option<(usize, usize)>) {
        (&self.preedit_text, self.preedit_cursor)
    }

    pub(crate) fn focus_lost(&mut self) {
        self.pointer_capture = MouseCapture::None;
        self.canvas_drag_session = None;
        self.clear_preedit();
        self.preferred_x = None;
    }
}

impl Default for EditorInputSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_context() -> EditorInputContext {
        EditorInputContext { focus: EditorFocus::Active, modal_blocked: false }
    }

    #[test]
    fn inactive_focus_rejects_keyboard_ime_and_pointer_start() {
        let mut session = EditorInputSession::new();
        let context = EditorInputContext { focus: EditorFocus::Inactive, ..active_context() };

        assert!(!session.keyboard_allowed(context));
        assert!(!session.start_text_selection(context));
        assert_eq!(
            session.update_preedit(context, "拼".to_owned(), Some((0, 3))),
            PreeditUpdate::Rejected
        );
        assert!(session.preedit().0.is_empty());
    }

    #[test]
    fn modal_blocks_editor_without_consuming_product_pointer_hit() {
        let session = EditorInputSession::new();
        let context = EditorInputContext { modal_blocked: true, ..active_context() };

        assert!(!session.pointer_allowed(context, true));
        assert!(!session.pointer_allowed(context, false));
    }

    #[test]
    fn capture_keeps_pointer_route_alive_outside_editor_bounds() {
        let mut session = EditorInputSession::new();
        let context = active_context();

        assert!(session.start_text_selection(context));
        assert_eq!(session.pointer_capture(), MouseCapture::TextSelection);
        assert!(session.pointer_allowed(context, false));
        session.end_pointer_capture();
        assert!(!session.pointer_allowed(context, false));
    }

    #[test]
    fn focus_loss_clears_capture_preedit_and_preferred_x() {
        let mut session = EditorInputSession::new();
        let context = active_context();

        assert!(session.start_canvas_drag(context));
        assert_eq!(
            session.update_preedit(context, "拼音".to_owned(), Some((0, 2))),
            PreeditUpdate::Changed
        );
        session.set_preferred_x(Some(120.0));
        session.focus_lost();

        assert_eq!(session.pointer_capture(), MouseCapture::None);
        assert!(session.preedit().0.is_empty());
        assert!(session.preferred_x().is_none());
    }
}
