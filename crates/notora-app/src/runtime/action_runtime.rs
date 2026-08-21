use std::time::Instant;

use appkit_shell::{DrainStart, EventPump};
use notora_core::WorkspaceId;

use crate::action::{NotoraAction, NotoraEffect};
use crate::external_files::{CanonicalExternalPath, ExternalFileSession, SaveExternalFileAs};
use crate::search_controller::{SearchController, SearchGeneration, SearchRequest};
use crate::{NotoraState, WorkspaceRootState};

/// 一次 reducer 调用产生的有序 effect 与运行时后续工作。
pub(super) struct ActionReduction {
    pub(super) effects: Vec<NotoraEffect>,
    pub(super) follow_up_actions: Vec<NotoraAction>,
    pub(super) should_persist_session: bool,
}

/// `NotoraState`、Action FIFO 与搜索去抖状态的唯一所有者。
pub(super) struct ActionRuntime {
    #[cfg(not(test))]
    state: NotoraState,
    #[cfg(test)]
    pub(super) state: NotoraState,
    event_pump: EventPump<NotoraAction>,
    search_controller: SearchController,
}

pub(super) enum ExternalSaveAsApplication {
    Updated,
    PathAlreadyOpen,
    SessionClosed,
}

impl ActionRuntime {
    pub(super) fn new(state: NotoraState) -> Self {
        Self {
            state,
            event_pump: EventPump::default(),
            search_controller: SearchController::default(),
        }
    }

    pub(super) fn state(&self) -> &NotoraState {
        &self.state
    }

    pub(super) fn enqueue(&mut self, action: NotoraAction) {
        self.event_pump.enqueue(action);
    }

    pub(super) fn start_draining(&mut self) -> DrainStart {
        self.event_pump.start_draining()
    }

    pub(super) fn next_action(&mut self) -> Option<NotoraAction> {
        self.event_pump.next_action()
    }

    pub(super) fn finish_draining(&mut self) {
        self.event_pump.finish_draining();
    }

    pub(super) fn reduce(
        &mut self,
        action: NotoraAction,
        now: Instant,
        should_persist_session: bool,
    ) -> ActionReduction {
        let committed_without_workspace = match &action {
            NotoraAction::SearchTextChanged(query) => {
                !self.search_controller.schedule_committed_query(query.clone(), now)
            }
            _ => false,
        };
        let effects = self.state.reduce(action);
        let follow_up_actions = committed_without_workspace
            .then(|| NotoraAction::SearchCommitted {
                query: self.state.library.search_text.clone(),
                search_generation: None,
            })
            .into_iter()
            .collect();
        ActionReduction { effects, follow_up_actions, should_persist_session }
    }

    pub(super) fn set_active_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        workspace_root: std::path::PathBuf,
    ) {
        self.state.activate_workspace();
        self.state.workspace_root_path = Some(workspace_root);
        self.search_controller.set_active_workspace(workspace_id, workspace_generation);
    }

    pub(super) fn clear_active_workspace(&mut self) {
        self.state.workspace_root = WorkspaceRootState::Missing;
        self.state.workspace_root_path = None;
        self.search_controller.clear_active_workspace();
    }

    pub(super) fn take_due_search_request(&mut self, now: Instant) -> Option<SearchRequest> {
        self.search_controller.take_due_request(now)
    }

    pub(super) fn next_search_deadline(&self) -> Option<Instant> {
        self.search_controller.next_deadline()
    }

    pub(super) fn accepts_search_generation(&self, generation: SearchGeneration) -> bool {
        self.search_controller.accepts_generation(generation)
    }

    pub(super) fn record_command_error(&mut self, message: String) {
        self.state.library.last_command_error = Some(message);
    }

    pub(super) fn set_responsive_mode(&mut self, mode: crate::ResponsiveLayoutMode) {
        self.state.layout.responsive_mode = mode;
    }

    pub(super) fn invalidate_document_selection(&mut self) {
        self.state.invalidate_document_selection();
    }

    pub(super) fn restore_navigation_expansion(
        &mut self,
        workspace_root_expanded: bool,
        tag_root_expanded: bool,
        directories: impl IntoIterator<Item = std::path::PathBuf>,
    ) {
        self.state.library.navigation_tree.workspace_root_expanded = workspace_root_expanded;
        self.state.library.navigation_tree.tag_root_expanded = tag_root_expanded;
        self.state.library.navigation_tree.expanded_directories.extend(directories);
    }

    pub(super) fn apply_external_save_as(
        &mut self,
        external_file_id: notora_core::ExternalFileId,
        canonical_path: CanonicalExternalPath,
    ) -> ExternalSaveAsApplication {
        match self.state.external_files.save_as(external_file_id, canonical_path) {
            Some(SaveExternalFileAs::Updated(_)) => ExternalSaveAsApplication::Updated,
            Some(SaveExternalFileAs::PathAlreadyOpen(_)) => {
                ExternalSaveAsApplication::PathAlreadyOpen
            }
            None => ExternalSaveAsApplication::SessionClosed,
        }
    }

    pub(super) fn create_untitled_external(
        &mut self,
        kind: notora_core::DocumentKind,
    ) -> notora_core::DocumentIdentity {
        self.state.external_files.create_untitled(kind)
    }

    pub(super) fn external_file_session(
        &self,
        external_file_id: notora_core::ExternalFileId,
    ) -> Option<ExternalFileSession> {
        self.state.external_files.session(external_file_id).cloned()
    }

    pub(super) fn open_existing_external(
        &mut self,
        canonical_path: CanonicalExternalPath,
    ) -> notora_core::DocumentIdentity {
        self.state.external_files.open_existing(canonical_path).identity()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::action::NotoraAction;

    use super::ActionRuntime;

    #[test]
    fn reducer_follow_up_actions_stay_behind_the_current_action_effects() {
        let mut runtime = ActionRuntime::new(crate::NotoraState::default());
        runtime.enqueue(NotoraAction::SearchTextChanged("queued".to_owned()));
        assert_eq!(runtime.start_draining(), appkit_shell::DrainStart::Started);

        let action = runtime.next_action().expect("queued action should be available");
        let reduction = runtime.reduce(action, Instant::now(), false);
        for follow_up_action in reduction.follow_up_actions {
            runtime.enqueue(follow_up_action);
        }

        assert!(matches!(runtime.next_action(), Some(NotoraAction::SearchCommitted { .. })));
        runtime.finish_draining();
    }

    #[test]
    fn navigation_expansion_restore_applies_root_and_directory_states_together() {
        let mut runtime = ActionRuntime::new(crate::NotoraState::default());

        runtime.restore_navigation_expansion(false, true, [std::path::PathBuf::from("plans")]);

        assert!(!runtime.state().library.navigation_tree.workspace_root_expanded);
        assert!(runtime.state().library.navigation_tree.tag_root_expanded);
        assert_eq!(
            runtime.state().library.navigation_tree.expanded_directories,
            [std::path::PathBuf::from("plans")].into_iter().collect()
        );
    }
}
