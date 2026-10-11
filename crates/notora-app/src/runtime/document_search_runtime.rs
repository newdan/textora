use super::NotoraRuntime;
use crate::action::NotoraAction;
use crate::{FocusTarget, OverlayState};
use ui::document_search_bar::{
    DOCUMENT_SEARCH_BAR_HEIGHT_LOGICAL, DocumentSearchBarAction, DocumentSearchBarInput,
};

impl NotoraRuntime {
    pub(crate) fn open_document_search(&mut self) -> bool {
        if self.state().layout.overlay != OverlayState::None
            || !self.window_runtime.is_focused()
            || !self.active_editor_matches_selection()
            || self.shell_layout().editor_rect == ui::Rect::ZERO
            || !crate::document_search::open(self.document_runtime.editor_mut())
        {
            return false;
        }
        self.dispatch_action(NotoraAction::FocusRequested(FocusTarget::DocumentSearch));
        self.synchronize_document_search();
        self.frame_runtime.document_search_bar.select_query();
        self.apply_shell_effect(appkit_shell::ShellEffect::REDRAW);
        true
    }

    pub(super) fn document_search_is_visible(&self) -> bool {
        self.active_editor_matches_selection()
            && self
                .document_runtime
                .editor()
                .active_tab_id()
                .and_then(|tab_id| self.document_runtime.editor().tab_session(tab_id))
                .is_some_and(|tab| tab.search_state().panel_visible && !tab.is_canvas())
    }

    pub(super) fn synchronize_document_search(&mut self) {
        let layout = self.shell_layout();
        let visible = self.document_search_is_visible() && layout.editor_rect != ui::Rect::ZERO;
        let focused = visible
            && self.window_runtime.is_focused()
            && self.state().layout.focus_target == FocusTarget::DocumentSearch
            && self.state().layout.overlay == OverlayState::None;
        let mut input = DocumentSearchBarInput { visible, focused, ..Default::default() };
        if visible {
            crate::document_search::refresh(self.document_runtime.editor_mut());
            if let Some(tab) = self
                .document_runtime
                .editor()
                .active_tab_id()
                .and_then(|tab_id| self.document_runtime.editor().tab_session(tab_id))
            {
                input.query = tab.search_state().query.clone();
                input.match_count = tab.search_state().match_count();
                input.current_match = tab.search_state().active_match_idx;
            }
        }
        let height = DOCUMENT_SEARCH_BAR_HEIGHT_LOGICAL * layout.dpi;
        let rect = if visible {
            ui::Rect::new(
                layout.editor_body_rect.x,
                layout.editor_body_rect.y - height,
                layout.editor_body_rect.w,
                height,
            )
        } else {
            ui::Rect::ZERO
        };
        self.frame_runtime.synchronize_document_search(input, rect, layout.dpi);
    }

    pub(super) fn route_document_search_event(&mut self, event: &ui::Event) -> bool {
        if self.state().layout.overlay != OverlayState::None
            || self.frame_runtime.editor_popup_is_open()
        {
            return false;
        }
        if self.state().layout.focus_target == FocusTarget::Editor
            && self.document_search_is_visible()
            && matches!(event, ui::Event::KeyDown(ui::KeyCode::Escape, _))
        {
            self.apply_document_search_action(DocumentSearchBarAction::Close);
            return true;
        }
        self.synchronize_document_search();
        let routed = self.frame_runtime.route_document_search_event(event, self.shell_layout().dpi);
        if let Some(action) = routed.action {
            self.apply_document_search_action(action);
        }
        if routed.consumed {
            self.apply_shell_effect(appkit_shell::ShellEffect::REDRAW);
        }
        routed.consumed
    }

    fn apply_document_search_action(&mut self, action: DocumentSearchBarAction) {
        match action {
            DocumentSearchBarAction::QueryChanged(query) => {
                crate::document_search::set_query(self.document_runtime.editor_mut(), query);
            }
            DocumentSearchBarAction::Next | DocumentSearchBarAction::Prev => {
                crate::document_search::navigate(
                    self.document_runtime.editor_mut(),
                    action == DocumentSearchBarAction::Prev,
                );
            }
            DocumentSearchBarAction::Close => {
                crate::document_search::close(self.document_runtime.editor_mut());
                self.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
            }
            DocumentSearchBarAction::FocusRequested => {
                self.dispatch_action(NotoraAction::FocusRequested(FocusTarget::DocumentSearch));
            }
            DocumentSearchBarAction::AppearanceChanged => {}
        }
        self.synchronize_document_search();
    }
}
