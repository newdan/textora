//! 编辑器输入门与会话状态。

use std::ops::Range;

use appkit_core::tab_set::types::TabId;

use crate::editor_runtime::{EditorFocus, EditorInputContext};
use crate::pointer_capture::{CanvasDragSession, MouseCapture};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreeditUpdate {
    Rejected,
    Unchanged,
    Changed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextSelectionGranularity {
    Grapheme,
    Word,
    SourceLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TextSelectionSession {
    Grapheme,
    VisualCaret { scope: Option<Range<usize>> },
    SourceRows { granularity: TextSelectionGranularity, pending_anchor: Option<usize> },
    AwaitingGroupedRange(TextSelectionGranularity),
    Grouped { granularity: TextSelectionGranularity, initial_range: Range<usize> },
}

impl TextSelectionSession {
    fn new(granularity: TextSelectionGranularity) -> Self {
        match granularity {
            TextSelectionGranularity::Grapheme => Self::Grapheme,
            TextSelectionGranularity::Word | TextSelectionGranularity::SourceLine => {
                Self::AwaitingGroupedRange(granularity)
            }
        }
    }

    fn granularity(&self) -> TextSelectionGranularity {
        match self {
            Self::Grapheme | Self::VisualCaret { .. } => TextSelectionGranularity::Grapheme,
            Self::AwaitingGroupedRange(granularity)
            | Self::Grouped { granularity, .. }
            | Self::SourceRows { granularity, .. } => *granularity,
        }
    }

    fn initial_range(&self) -> Option<Range<usize>> {
        match self {
            Self::Grouped { initial_range, .. } => Some(initial_range.clone()),
            Self::Grapheme
            | Self::VisualCaret { .. }
            | Self::SourceRows { .. }
            | Self::AwaitingGroupedRange(_) => None,
        }
    }
}

impl PreeditUpdate {
    pub(crate) fn accepted(self) -> bool {
        !matches!(self, Self::Rejected)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CapturedCanvasDrag {
    pub tab_id: TabId,
    pub bounds: ui::Rect,
    pub gesture: CanvasDragSession,
}

#[derive(Debug, Clone, PartialEq)]
enum PointerSession {
    Idle,
    TextSelection(TextSelectionSession),
    CanvasDrag(CapturedCanvasDrag),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditorInputSession {
    modifiers: winit::keyboard::ModifiersState,
    pointer: PointerSession,
    preedit_text: String,
    preedit_cursor: Option<(usize, usize)>,
    preferred_x: Option<f32>,
}

impl EditorInputSession {
    pub(crate) fn new() -> Self {
        Self {
            modifiers: winit::keyboard::ModifiersState::default(),
            pointer: PointerSession::Idle,
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
        !matches!(self.pointer, PointerSession::Idle) || pointer_inside_editor
    }

    pub(crate) fn start_text_selection(
        &mut self,
        context: EditorInputContext,
        granularity: TextSelectionGranularity,
    ) -> bool {
        if !self.keyboard_allowed(context) {
            return false;
        }
        self.pointer = PointerSession::TextSelection(TextSelectionSession::new(granularity));
        true
    }

    pub(crate) fn start_canvas_drag_session(
        &mut self,
        context: EditorInputContext,
        session: CapturedCanvasDrag,
    ) -> bool {
        if !self.keyboard_allowed(context) {
            return false;
        }
        self.pointer = PointerSession::CanvasDrag(session);
        true
    }

    pub(crate) fn canvas_drag_session_mut(&mut self) -> Option<&mut CapturedCanvasDrag> {
        match &mut self.pointer {
            PointerSession::CanvasDrag(drag) => Some(drag),
            _ => None,
        }
    }

    pub(crate) fn take_canvas_drag_session(&mut self) -> Option<CapturedCanvasDrag> {
        match std::mem::replace(&mut self.pointer, PointerSession::Idle) {
            PointerSession::CanvasDrag(drag) => Some(drag),
            previous => {
                self.pointer = previous;
                None
            }
        }
    }

    pub(crate) fn end_pointer_capture(&mut self) {
        self.pointer = PointerSession::Idle;
    }

    pub(crate) fn pointer_capture(&self) -> MouseCapture {
        match &self.pointer {
            PointerSession::Idle => MouseCapture::None,
            PointerSession::CanvasDrag(_) => MouseCapture::CanvasDrag,
            PointerSession::TextSelection(_) => MouseCapture::TextSelection,
        }
    }

    fn text_selection(&self) -> Option<&TextSelectionSession> {
        match &self.pointer {
            PointerSession::TextSelection(selection) => Some(selection),
            _ => None,
        }
    }

    pub(crate) fn text_selection_granularity(&self) -> Option<TextSelectionGranularity> {
        self.text_selection().map(TextSelectionSession::granularity)
    }

    pub(crate) fn set_visual_caret_scope(&mut self, scope: Option<Range<usize>>) {
        if let PointerSession::TextSelection(selection) = &mut self.pointer {
            *selection = TextSelectionSession::VisualCaret { scope };
        }
    }

    pub(crate) fn visual_caret_scope(&self) -> Option<&Range<usize>> {
        match self.text_selection() {
            Some(TextSelectionSession::VisualCaret { scope }) => scope.as_ref(),
            _ => None,
        }
    }

    pub(super) fn set_source_row_anchor(&mut self, pending_anchor: Option<usize>) {
        if let PointerSession::TextSelection(selection) = &mut self.pointer {
            *selection = TextSelectionSession::SourceRows {
                granularity: selection.granularity(),
                pending_anchor,
            };
        }
    }

    pub(super) fn take_source_row_anchor(&mut self) -> Option<usize> {
        match &mut self.pointer {
            PointerSession::TextSelection(TextSelectionSession::SourceRows {
                pending_anchor,
                ..
            }) => pending_anchor.take(),
            _ => None,
        }
    }

    pub(crate) fn set_initial_text_selection_range(&mut self, initial_range: Range<usize>) {
        let PointerSession::TextSelection(selection) = &mut self.pointer else {
            return;
        };
        let TextSelectionSession::AwaitingGroupedRange(granularity) = selection else {
            return;
        };
        *selection = TextSelectionSession::Grouped { granularity: *granularity, initial_range };
    }

    pub(crate) fn initial_text_selection_range(&self) -> Option<Range<usize>> {
        self.text_selection().and_then(TextSelectionSession::initial_range)
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
        self.end_pointer_capture();
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
        assert!(!session.start_text_selection(context, TextSelectionGranularity::Grapheme));
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

        assert!(session.start_text_selection(context, TextSelectionGranularity::Grapheme));
        assert_eq!(session.pointer_capture(), MouseCapture::TextSelection);
        assert!(session.pointer_allowed(context, false));
        session.end_pointer_capture();
        assert!(!session.pointer_allowed(context, false));
    }

    #[test]
    fn focus_loss_clears_capture_preedit_and_preferred_x() {
        let mut session = EditorInputSession::new();
        let context = active_context();

        let drag = CapturedCanvasDrag {
            tab_id: appkit_core::tab_set::types::TabIdAllocator::new().allocate(),
            bounds: ui::Rect::new(0.0, 0.0, 200.0, 100.0),
            gesture: CanvasDragSession {
                source_range: 0..1,
                pressed_at: (50.0, 50.0),
                source_generation: 1,
                started: true,
            },
        };
        assert!(session.start_canvas_drag_session(context, drag));
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

    #[test]
    fn text_selection_granularity_follows_pointer_capture_lifetime() {
        let mut session = EditorInputSession::new();
        let context = active_context();

        assert!(session.start_text_selection(context, TextSelectionGranularity::Word));
        assert_eq!(session.text_selection_granularity(), Some(TextSelectionGranularity::Word));
        session.set_initial_text_selection_range(6..11);
        assert_eq!(session.initial_text_selection_range(), Some(6..11));

        session.end_pointer_capture();
        assert_eq!(session.text_selection_granularity(), None);
        assert_eq!(session.initial_text_selection_range(), None);

        assert!(session.start_text_selection(context, TextSelectionGranularity::SourceLine));
        assert_eq!(
            session.text_selection_granularity(),
            Some(TextSelectionGranularity::SourceLine)
        );

        session.focus_lost();
        assert_eq!(session.text_selection_granularity(), None);
        assert_eq!(session.initial_text_selection_range(), None);
    }
}
