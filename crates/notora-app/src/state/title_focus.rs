use notora_core::note_command::NoteCommandOutcome;

use super::{
    ConflictResolution, DirectoryCreationState, DocumentIdentity, FocusTarget, NavigationScope,
    NotoraAction, NotoraState, OverlayState, WorkspaceCreationState, WorkspaceTransitionState,
};

impl NotoraState {
    /// Whether reducing the action would leave the current title draft's editor.
    pub(crate) fn will_leave_title_focus(&self, action: &NotoraAction) -> bool {
        if self.layout.focus_target != FocusTarget::EditorTitle
            || self.library.title_draft.is_none()
        {
            return false;
        }

        use NotoraAction::*;

        match action {
            NavigationSelected(_)
            | SearchTextChanged(_)
            | SearchCommitted { .. }
            | CardSelected(_)
            | CompactNavigationRequested
            | NavigationPaneVisibilityToggled
            | CompactBackRequested
            | ExternalFileOpened(_)
            | EncryptedNoteUnlockCompleted
            | ToggleSourceViewRequested
            | SemanticEditRequested(_)
            | SaveConflictDetected { .. }
            | OpenSettings => true,
            CardActivated(identity) => self.library.selected_card == Some(*identity),
            NavigationTreeLoaded(tree) => !self.navigation_scope_exists(tree),
            BeginDirectoryCreation { parent_relative_path } => {
                self.can_begin_directory_creation(parent_relative_path)
            }
            DirectoryCreationCompleted { relative_path } => {
                self.completed_directory_parent(relative_path).is_some()
            }
            DirectoryCreationFailed(_) => {
                matches!(self.directory_creation, DirectoryCreationState::Submitting { .. })
            }
            OpenWorkspaceCreationRequested => {
                self.workspace_transition == WorkspaceTransitionState::Idle
            }
            WorkspaceCreationCommitRequested => {
                matches!(
                    &self.workspace_creation,
                    WorkspaceCreationState::Editing { name, parent_directory: Some(_) }
                        if notora_core::validate_workspace_directory_name(name).is_ok()
                )
            }
            ExternalFileCloseCompleted(id) => {
                self.library.selected_card == Some(DocumentIdentity::ExternalFile(*id))
            }
            ExternalFilesClearCompleted { closed_external_file_ids, .. } => {
                matches!(self.library.selected_card,
                    Some(DocumentIdentity::ExternalFile(id)) if closed_external_file_ids.contains(&id))
            }
            OpenNewDocumentMenu => self.can_open_new_document_menu(),
            CreateRequested(_) => {
                self.can_request_note_creation()
                    && self.layout.overlay == OverlayState::NewDocumentMenu
            }
            BeginEncryptedNoteCreation => self.can_begin_encrypted_note_creation(),
            EncryptedNoteUnlockRequired { request, .. } => self.is_current_document(*request),
            EncryptedConflictCopyRequired { identity, .. } | SaveConflictResolved { identity } => {
                self.library.save_conflict.is_some_and(|conflict| conflict.identity == *identity)
            }
            NoteCommandCompleted(result) => match result.outcome {
                NoteCommandOutcome::Created => {
                    self.library.selected_card != Some(DocumentIdentity::Note(result.note.note_id))
                }
                NoteCommandOutcome::TitleUpdated => false,
                NoteCommandOutcome::Moved => true,
            },
            TrashOperationRequested(operation) => matches!(
                operation,
                crate::action::TrashOperation::PermanentlyDelete { .. }
                    | crate::action::TrashOperation::Empty
            ),
            TrashPermanentDeletionConfirmed => {
                matches!(
                    self.layout.overlay,
                    OverlayState::TrashPermanentDeletionConfirmation { .. }
                )
            }
            TrashRestoreWithRenamedPathConfirmed => {
                matches!(self.layout.overlay, OverlayState::TrashRestoreConflictConfirmation { .. })
            }
            TrashOperationCompleted => {
                self.library.navigation_scope == NavigationScope::Trash
                    && self.library.selected_card.is_some()
            }
            TrashOperationFailed(failure) => {
                matches!(failure, crate::action::TrashOperationFailure::RestoreConflict { .. })
            }
            SaveConflictResolutionRequested(resolution) => {
                self.library.save_conflict.is_some() && *resolution == ConflictResolution::Cancel
            }
            FocusRequested(target) => {
                *target != FocusTarget::EditorTitle || self.layout.overlay != OverlayState::None
            }
            OverlayDismissed => self.layout.overlay != OverlayState::None,
            EscapePressed => {
                self.layout.overlay != OverlayState::None
                    || matches!(self.library.navigation_scope, NavigationScope::Search { .. })
                    || self.library.search_text.is_empty()
            }
            DocumentSearchRequested
            | CardQueryCompleted { .. }
            | CardQueryFailed { .. }
            | NavigationTreeFailed(_)
            | CatalogReindexed
            | CatalogRecoveryNotified(_)
            | WorkspaceRootExpansionToggled
            | TagRootExpansionToggled
            | NavigationExpansionToggled(_)
            | DirectoryCreationTextChanged(_)
            | DirectoryCreationCommitRequested
            | DirectoryCreationCancelled
            | WorkspaceCreationNameChanged(_)
            | WorkspaceCreationLocationRequested
            | WorkspaceCreationLocationSelected(_)
            | WorkspaceTransitionConfirmed(_)
            | WorkspaceTransitionApplying
            | WorkspaceTransitionCompleted
            | WorkspaceTransitionFailed(_)
            | CardListScrolled { .. }
            | ActiveEditorMetadataLoaded { .. }
            | ActiveEditorSaved { .. }
            | OpenExternalFileDialogRequested
            | ExternalPathsReceived(_)
            | ExternalFileCloseRequested(_)
            | ExternalFileCloseFailed(_)
            | ExternalFilesClearRequested
            | PromotePreviewRequested
            | WorkspaceRootSelectionRequested
            | EncryptedNotePasswordChanged(_)
            | EncryptedNoteConfirmationChanged(_)
            | EncryptedNoteDialogSubmitRequested
            | TitleTextChanged(_)
            | TitleCommitRequested(_)
            | ToggleInspectorPanelRequested
            | InspectorPanel(_)
            | MoveDialogRequested(_)
            | MoveRequested { .. }
            | MetadataMutationRequested(_)
            | MetadataMutationCompleted { .. }
            | MetadataMutationFailed(_)
            | NoteCommandFailed(_)
            | SplitterDragged { .. }
            | ProductSettingsUpdateRequested(_)
            | RetryProductSettingsPersistence
            | SettingsViewChanged => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::state::{SaveConflict, WorkspaceRootState};
    use notora_core::{CatalogNavigationTree, DocumentKind};

    #[test]
    fn title_focus_prediction_matches_reducer_on_focus_and_selection_changes() {
        let selected = DocumentIdentity::Note(notora_core::NoteId::generate());
        let mut state = NotoraState::default();
        state.layout.focus_target = FocusTarget::EditorTitle;
        state.library.selected_card = Some(selected);
        state.library.title_draft = Some("draft".to_owned());
        let empty_tree = CatalogNavigationTree { directories: Vec::new(), tags: Vec::new() };
        let actions = [
            NotoraAction::CardActivated(selected),
            NotoraAction::CardActivated(DocumentIdentity::Note(notora_core::NoteId::generate())),
            NotoraAction::CardSelected(selected),
            NotoraAction::CatalogReindexed,
            NotoraAction::NavigationTreeLoaded(empty_tree),
            NotoraAction::FocusRequested(FocusTarget::EditorTitle),
            NotoraAction::FocusRequested(FocusTarget::Editor),
            NotoraAction::EscapePressed,
            NotoraAction::OverlayDismissed,
            NotoraAction::OpenNewDocumentMenu,
            NotoraAction::CreateRequested(DocumentKind::Markdown),
            NotoraAction::BeginDirectoryCreation { parent_relative_path: PathBuf::new() },
            NotoraAction::DirectoryCreationCompleted { relative_path: PathBuf::from("archive") },
            NotoraAction::OpenWorkspaceCreationRequested,
            NotoraAction::WorkspaceCreationCommitRequested,
            NotoraAction::SaveConflictResolved { identity: selected },
            NotoraAction::SaveConflictResolutionRequested(ConflictResolution::Cancel),
            NotoraAction::TrashOperationCompleted,
        ];

        assert_matches_reducer(&state, &actions);
        state.workspace_root = WorkspaceRootState::Active;
        state.layout.overlay = OverlayState::NewDocumentMenu;
        state.library.navigation_scope =
            NavigationScope::Directory { relative_path: PathBuf::from("old") };
        state.directory_creation = DirectoryCreationState::Submitting {
            parent_relative_path: PathBuf::new(),
            directory_name: "archive".to_owned(),
        };
        state.workspace_creation = WorkspaceCreationState::Editing {
            name: "workspace".to_owned(),
            parent_directory: Some(PathBuf::from("/tmp")),
        };
        assert_matches_reducer(&state, &actions);

        state.layout.overlay = OverlayState::None;
        state.library.navigation_scope = NavigationScope::Trash;
        state.library.search_text = "still visible".to_owned();
        state.library.save_conflict =
            Some(SaveConflict { identity: selected, content_revision: 1 });
        assert_matches_reducer(&state, &actions);
    }

    fn assert_matches_reducer(state: &NotoraState, actions: &[NotoraAction]) {
        for action in actions {
            let mut next = state.clone();
            let _ = next.reduce(action.clone());
            let expected = next.layout.focus_target != FocusTarget::EditorTitle
                || next.library.selected_card != state.library.selected_card;
            assert_eq!(state.will_leave_title_focus(action), expected, "{action:?}");
        }
    }
}
