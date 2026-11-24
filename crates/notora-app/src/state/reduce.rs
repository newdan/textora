use notora_core::{DocumentIdentity, NavigationScope};

use crate::action::{CardQuery, NotoraAction, NotoraEffect, move_note_command};
use crate::effect_executor::ExternalOpenRequest;

use super::{
    CompactContent, CompactNavigation, DirectoryCreationState, EncryptedConflictCopyState,
    EncryptedNoteUnlockState, FocusTarget, NavigationPaneVisibility, NotoraState, OverlayState,
    ResponsiveLayoutMode, SaveConflict, WorkspaceCreationState, WorkspaceTransitionState,
};

impl NotoraState {
    pub fn reduce(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::NavigationSelected(..)
            | NotoraAction::SearchTextChanged(..)
            | NotoraAction::SearchCommitted { .. }
            | NotoraAction::NavigationTreeLoaded(..)
            | NotoraAction::NavigationTreeFailed(..)
            | NotoraAction::WorkspaceRootExpansionToggled
            | NotoraAction::TagRootExpansionToggled
            | NotoraAction::NavigationExpansionToggled(..)
            | NotoraAction::CompactNavigationRequested
            | NotoraAction::NavigationPaneVisibilityToggled
            | NotoraAction::CompactBackRequested
            | NotoraAction::SplitterDragged { .. } => self.reduce_navigation(action),
            NotoraAction::CardQueryCompleted { .. }
            | NotoraAction::CardQueryFailed { .. }
            | NotoraAction::CardListScrolled { .. }
            | NotoraAction::CardSelected(..)
            | NotoraAction::CardActivated(..)
            | NotoraAction::MetadataMutationRequested(..)
            | NotoraAction::MetadataMutationCompleted { .. }
            | NotoraAction::MetadataMutationFailed(..)
            | NotoraAction::CatalogReindexed
            | NotoraAction::TrashOperationRequested(..)
            | NotoraAction::TrashPermanentDeletionConfirmed
            | NotoraAction::TrashRestoreWithRenamedPathConfirmed
            | NotoraAction::TrashOperationCompleted
            | NotoraAction::TrashOperationFailed(..) => self.reduce_library_cards(action),
            NotoraAction::CatalogRecoveryNotified(..)
            | NotoraAction::BeginDirectoryCreation { .. }
            | NotoraAction::DirectoryCreationTextChanged(..)
            | NotoraAction::DirectoryCreationCommitRequested
            | NotoraAction::DirectoryCreationCancelled
            | NotoraAction::DirectoryCreationCompleted { .. }
            | NotoraAction::DirectoryCreationFailed(..)
            | NotoraAction::OpenWorkspaceCreationRequested
            | NotoraAction::WorkspaceCreationNameChanged(..)
            | NotoraAction::WorkspaceCreationLocationRequested
            | NotoraAction::WorkspaceCreationLocationSelected(..)
            | NotoraAction::WorkspaceCreationCommitRequested
            | NotoraAction::WorkspaceTransitionConfirmed(..)
            | NotoraAction::WorkspaceTransitionApplying
            | NotoraAction::WorkspaceTransitionCompleted
            | NotoraAction::WorkspaceTransitionFailed(..)
            | NotoraAction::WorkspaceRootSelectionRequested => {
                self.reduce_workspace_lifecycle(action)
            }
            NotoraAction::OpenExternalFileDialogRequested
            | NotoraAction::ExternalPathsReceived(..)
            | NotoraAction::ExternalFileOpened(..)
            | NotoraAction::ExternalFileCloseRequested(..)
            | NotoraAction::ExternalFileCloseCompleted(..)
            | NotoraAction::ExternalFileCloseFailed(..)
            | NotoraAction::ExternalFilesClearRequested
            | NotoraAction::ExternalFilesClearCompleted { .. } => {
                self.reduce_external_files(action)
            }
            NotoraAction::DocumentSearchRequested
            | NotoraAction::ActiveEditorMetadataLoaded { .. }
            | NotoraAction::ActiveEditorSaved { .. }
            | NotoraAction::PromotePreviewRequested
            | NotoraAction::TitleTextChanged(..)
            | NotoraAction::TitleCommitRequested(..)
            | NotoraAction::ToggleSourceViewRequested
            | NotoraAction::ToggleInspectorPanelRequested
            | NotoraAction::InspectorPanel(..)
            | NotoraAction::SemanticEditRequested(..)
            | NotoraAction::MoveDialogRequested(..)
            | NotoraAction::NoteCommandCompleted(..)
            | NotoraAction::NoteCommandFailed(..)
            | NotoraAction::MoveRequested { .. }
            | NotoraAction::SaveConflictDetected { .. }
            | NotoraAction::SaveConflictResolutionRequested(..)
            | NotoraAction::SaveConflictResolved { .. }
            | NotoraAction::FocusRequested(..) => self.reduce_editor(action),
            NotoraAction::OpenNewDocumentMenu
            | NotoraAction::CreateRequested(..)
            | NotoraAction::BeginEncryptedNoteCreation
            | NotoraAction::EncryptedNotePasswordChanged(..)
            | NotoraAction::EncryptedNoteConfirmationChanged(..)
            | NotoraAction::EncryptedNoteDialogSubmitRequested
            | NotoraAction::EncryptedNoteUnlockRequired { .. }
            | NotoraAction::EncryptedNoteUnlockCompleted
            | NotoraAction::EncryptedConflictCopyRequired { .. }
            | NotoraAction::OpenSettings
            | NotoraAction::ProductSettingsUpdateRequested(..)
            | NotoraAction::RetryProductSettingsPersistence
            | NotoraAction::SettingsViewChanged
            | NotoraAction::OverlayDismissed
            | NotoraAction::EscapePressed => self.reduce_dialogs(action),
        }
    }

    fn reduce_navigation(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::NavigationSelected(scope) => self.select_navigation_scope(scope),
            NotoraAction::SearchTextChanged(query) => {
                self.library.search_text = query;
                self.layout.focus_target = FocusTarget::NavigationSearch;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SearchCommitted { query, search_generation } => {
                self.commit_search(query, search_generation)
            }
            NotoraAction::NavigationTreeLoaded(tree) => self.apply_navigation_tree(tree),
            NotoraAction::NavigationTreeFailed(message) => {
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceRootExpansionToggled => {
                self.library.navigation_tree.workspace_root_expanded =
                    !self.library.navigation_tree.workspace_root_expanded;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::TagRootExpansionToggled => {
                self.library.navigation_tree.tag_root_expanded =
                    !self.library.navigation_tree.tag_root_expanded;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::NavigationExpansionToggled(relative_path) => {
                self.toggle_navigation_expansion(relative_path)
            }
            NotoraAction::CompactNavigationRequested => {
                self.layout.compact_navigation = CompactNavigation::Visible;
                self.layout.focus_target = FocusTarget::NavigationTree;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::NavigationPaneVisibilityToggled => {
                if self.layout.responsive_mode != ResponsiveLayoutMode::ThreePane {
                    self.layout.compact_navigation = match self.layout.compact_navigation {
                        CompactNavigation::Hidden => CompactNavigation::Visible,
                        CompactNavigation::Visible => CompactNavigation::Hidden,
                    };
                    self.layout.focus_target = match self.layout.compact_navigation {
                        CompactNavigation::Visible => FocusTarget::NavigationTree,
                        CompactNavigation::Hidden => match self.layout.compact_content {
                            CompactContent::CardList => FocusTarget::CardList,
                            CompactContent::Editor => FocusTarget::Editor,
                        },
                    };
                    return vec![NotoraEffect::Redraw];
                }
                self.layout.navigation_pane_visibility =
                    match self.layout.navigation_pane_visibility {
                        NavigationPaneVisibility::Expanded => NavigationPaneVisibility::Collapsed,
                        NavigationPaneVisibility::Collapsed => NavigationPaneVisibility::Expanded,
                    };
                self.layout.focus_target = match self.layout.navigation_pane_visibility {
                    NavigationPaneVisibility::Expanded => FocusTarget::NavigationTree,
                    NavigationPaneVisibility::Collapsed => FocusTarget::CardList,
                };
                vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]
            }
            NotoraAction::CompactBackRequested => {
                self.layout.compact_content = CompactContent::CardList;
                self.layout.compact_navigation = CompactNavigation::Hidden;
                self.layout.focus_target = FocusTarget::CardList;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SplitterDragged { pane, logical_width } => {
                self.set_pane_width(pane, logical_width);
                vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]
            }
            _ => unreachable!("reduce routes only navigation actions here"),
        }
    }

    fn reduce_library_cards(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::CardQueryCompleted { query, page } => self.apply_card_page(query, page),
            NotoraAction::CardQueryFailed { query, message } => {
                self.apply_card_query_failure(query, message)
            }
            NotoraAction::CardListScrolled { offset_px, near_end } => {
                self.library.card_scroll_offset_px = offset_px;
                if near_end {
                    return self.request_next_card_page();
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CardSelected(identity) => {
                let request = self.select_document(identity);
                self.layout.focus_target = FocusTarget::CardList;
                self.layout.compact_content = CompactContent::CardList;
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::CardActivated(identity) => {
                if self.library.selected_card != Some(identity) {
                    return vec![NotoraEffect::Redraw];
                }
                self.layout.focus_target = FocusTarget::Editor;
                self.layout.compact_content = CompactContent::Editor;
                vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
            }
            NotoraAction::MetadataMutationRequested(mutation) => {
                self.library.last_command_error = None;
                vec![NotoraEffect::ExecuteMetadataMutation(mutation), NotoraEffect::Redraw]
            }
            NotoraAction::MetadataMutationCompleted { note_id, metadata, selection_generation } => {
                self.apply_metadata_mutation_completion(note_id, metadata, selection_generation);
                self.library.last_command_error = None;
                if self.library.navigation_scope == NavigationScope::ExternalFiles {
                    return vec![NotoraEffect::Redraw];
                }
                self.request_card_query(CardQuery::from(self.library.navigation_scope.clone()))
            }
            NotoraAction::MetadataMutationFailed(message) => {
                self.library.pending_title_commit = None;
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CatalogReindexed => {
                if self.library.navigation_scope == NavigationScope::ExternalFiles {
                    return vec![NotoraEffect::Redraw];
                }
                self.refresh_card_query(CardQuery::from(self.library.navigation_scope.clone()))
            }
            NotoraAction::TrashOperationRequested(operation) => {
                if matches!(
                    operation,
                    crate::action::TrashOperation::PermanentlyDelete { .. }
                        | crate::action::TrashOperation::Empty
                ) {
                    self.layout.overlay =
                        OverlayState::TrashPermanentDeletionConfirmation { operation };
                    self.layout.focus_target = FocusTarget::Overlay;
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::ExecuteTrashOperation(operation), NotoraEffect::Redraw]
            }
            NotoraAction::TrashPermanentDeletionConfirmed => {
                let OverlayState::TrashPermanentDeletionConfirmation { operation } =
                    self.layout.overlay
                else {
                    return vec![NotoraEffect::Redraw];
                };
                self.layout.overlay = OverlayState::None;
                self.layout.focus_target = FocusTarget::NavigationTree;
                self.library.last_command_error = None;
                vec![NotoraEffect::ExecuteTrashOperation(operation), NotoraEffect::Redraw]
            }
            NotoraAction::TrashRestoreWithRenamedPathConfirmed => {
                let OverlayState::TrashRestoreConflictConfirmation { note_id } =
                    self.layout.overlay
                else {
                    return vec![NotoraEffect::Redraw];
                };
                self.layout.overlay = OverlayState::None;
                self.layout.focus_target = FocusTarget::NavigationTree;
                self.library.last_command_error = None;
                vec![
                    NotoraEffect::ExecuteTrashOperation(
                        crate::action::TrashOperation::RestoreWithRenamedPath { note_id },
                    ),
                    NotoraEffect::Redraw,
                ]
            }
            NotoraAction::TrashOperationCompleted => {
                self.library.last_command_error = None;
                if self.library.navigation_scope == NavigationScope::Trash {
                    self.invalidate_document_selection();
                }
                self.request_card_query(CardQuery::from(self.library.navigation_scope.clone()))
            }
            NotoraAction::TrashOperationFailed(failure) => match failure {
                crate::action::TrashOperationFailure::RestoreConflict { note_id } => {
                    self.layout.overlay =
                        OverlayState::TrashRestoreConflictConfirmation { note_id };
                    self.layout.focus_target = FocusTarget::Overlay;
                    vec![NotoraEffect::Redraw]
                }
                crate::action::TrashOperationFailure::Message(message) => {
                    self.library.last_command_error = Some(message);
                    vec![NotoraEffect::Redraw]
                }
            },
            _ => unreachable!("reduce routes only library cards actions here"),
        }
    }

    fn reduce_workspace_lifecycle(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::CatalogRecoveryNotified(message) => {
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::BeginDirectoryCreation { parent_relative_path } => {
                self.begin_directory_creation(parent_relative_path)
            }
            NotoraAction::DirectoryCreationTextChanged(draft_name) => {
                self.update_directory_creation_text(draft_name)
            }
            NotoraAction::DirectoryCreationCommitRequested => self.commit_directory_creation(),
            NotoraAction::DirectoryCreationCancelled => {
                self.directory_creation = DirectoryCreationState::Inactive;
                self.library.last_command_error = None;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::DirectoryCreationCompleted { relative_path } => {
                self.complete_directory_creation(relative_path)
            }
            NotoraAction::DirectoryCreationFailed(message) => self.fail_directory_creation(message),
            NotoraAction::OpenWorkspaceCreationRequested => {
                if self.workspace_transition != WorkspaceTransitionState::Idle {
                    return vec![NotoraEffect::Redraw];
                }
                self.workspace_creation =
                    WorkspaceCreationState::Editing { name: String::new(), parent_directory: None };
                self.library.last_command_error = None;
                self.layout.overlay = OverlayState::NewWorkspace;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationNameChanged(name) => {
                if let WorkspaceCreationState::Editing { name: current_name, .. } =
                    &mut self.workspace_creation
                {
                    *current_name = name;
                    self.library.last_command_error = None;
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationLocationRequested => {
                if !matches!(self.workspace_creation, WorkspaceCreationState::Editing { .. }) {
                    return vec![NotoraEffect::Redraw];
                }
                vec![NotoraEffect::ChooseWorkspaceCreationLocation, NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationLocationSelected(parent_directory) => {
                if let WorkspaceCreationState::Editing {
                    parent_directory: current_parent, ..
                } = &mut self.workspace_creation
                {
                    *current_parent = Some(parent_directory);
                    self.library.last_command_error = None;
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationCommitRequested => self.commit_workspace_creation(),
            NotoraAction::WorkspaceTransitionConfirmed(request) => {
                if self.workspace_transition != WorkspaceTransitionState::Idle {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                self.workspace_transition =
                    WorkspaceTransitionState::AwaitingDirtySaves { request: request.clone() };
                vec![NotoraEffect::PrepareWorkspaceTransition(request), NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceTransitionApplying => {
                let WorkspaceTransitionState::AwaitingDirtySaves { request } =
                    &self.workspace_transition
                else {
                    return vec![NotoraEffect::Redraw];
                };
                self.workspace_transition =
                    WorkspaceTransitionState::Applying { request: request.clone() };
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceTransitionCompleted => {
                self.workspace_transition = WorkspaceTransitionState::Idle;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceTransitionFailed(message) => {
                self.workspace_transition = WorkspaceTransitionState::Idle;
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceRootSelectionRequested => {
                if self.workspace_transition != WorkspaceTransitionState::Idle {
                    return vec![NotoraEffect::Redraw];
                }
                vec![NotoraEffect::ChooseWorkspaceRoot, NotoraEffect::Redraw]
            }
            _ => unreachable!("reduce routes only workspace lifecycle actions here"),
        }
    }

    fn reduce_external_files(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::OpenExternalFileDialogRequested => {
                vec![NotoraEffect::OpenExternalFiles(ExternalOpenRequest::ShowFileDialog)]
            }
            NotoraAction::ExternalPathsReceived(paths) => {
                vec![NotoraEffect::OpenExternalFiles(ExternalOpenRequest::Paths(paths))]
            }
            NotoraAction::ExternalFileOpened(identity) => {
                let request = self.select_opened_external_file(identity);
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFileCloseRequested(external_file_id) => {
                if self.external_files.session(external_file_id).is_none() {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::CloseExternalFile(external_file_id), NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFileCloseCompleted(external_file_id) => {
                let identity = DocumentIdentity::ExternalFile(external_file_id);
                let _ = self.external_files.remove(external_file_id);
                if self.library.selected_card == Some(identity) {
                    self.invalidate_document_selection();
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFileCloseFailed(message) => {
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFilesClearRequested => {
                if self.external_files.sessions().is_empty() {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::CloseAllExternalFiles, NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFilesClearCompleted {
                closed_external_file_ids,
                blocked_count,
            } => {
                let selected_external_file_id =
                    self.library.selected_card.and_then(|identity| match identity {
                        DocumentIdentity::ExternalFile(external_file_id) => Some(external_file_id),
                        DocumentIdentity::Note(_) => None,
                    });
                let selected_record_was_closed = selected_external_file_id
                    .is_some_and(|selected| closed_external_file_ids.contains(&selected));
                for external_file_id in closed_external_file_ids {
                    let _ = self.external_files.remove(external_file_id);
                }
                if selected_record_was_closed {
                    self.invalidate_document_selection();
                }
                self.library.last_command_error = (blocked_count > 0).then(|| {
                    format!("有 {blocked_count} 个文件未清空：请先保存未保存内容或取消固定")
                });
                vec![NotoraEffect::Redraw]
            }
            _ => unreachable!("reduce routes only external files actions here"),
        }
    }

    fn reduce_editor(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::DocumentSearchRequested => vec![NotoraEffect::Redraw],
            NotoraAction::ActiveEditorMetadataLoaded { request, metadata, tags } => {
                self.apply_active_editor_metadata(request, metadata, tags)
            }
            NotoraAction::ActiveEditorSaved { identity, saved_at } => {
                self.apply_active_editor_saved(identity, saved_at)
            }
            NotoraAction::PromotePreviewRequested => {
                vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
            }
            NotoraAction::TitleTextChanged(title) => {
                self.library.title_draft = Some(title);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::TitleCommitRequested(title) => self.request_title_commit(title),
            NotoraAction::ToggleSourceViewRequested => {
                self.layout.focus_target = FocusTarget::Editor;
                vec![NotoraEffect::ToggleEditorView]
            }
            NotoraAction::ToggleInspectorPanelRequested => {
                vec![NotoraEffect::ToggleInspectorPanel, NotoraEffect::Redraw]
            }
            NotoraAction::InspectorPanel(action) => {
                vec![NotoraEffect::DispatchInspectorPanel(action), NotoraEffect::Redraw]
            }
            NotoraAction::SemanticEditRequested(command) => {
                self.layout.focus_target = FocusTarget::Editor;
                vec![NotoraEffect::ExecuteSemanticEdit(command), NotoraEffect::Redraw]
            }
            NotoraAction::MoveDialogRequested(note_id) => {
                vec![NotoraEffect::ChooseNoteMoveDirectory(note_id), NotoraEffect::Redraw]
            }
            NotoraAction::NoteCommandCompleted(result) => {
                self.apply_note_command_completion(result)
            }
            NotoraAction::NoteCommandFailed(message) => {
                if self.fail_encrypted_note_creation(message.clone()) {
                    return vec![NotoraEffect::Redraw];
                }
                if self.fail_encrypted_note_unlock(message.clone()) {
                    return vec![NotoraEffect::Redraw];
                }
                if self.fail_encrypted_conflict_copy(message.clone()) {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::MoveRequested { note_id, target_directory } => {
                self.library.last_command_error = None;
                vec![
                    NotoraEffect::ExecuteNoteCommand(move_note_command(note_id, target_directory)),
                    NotoraEffect::Redraw,
                ]
            }
            NotoraAction::SaveConflictDetected { identity, content_revision } => {
                self.library.save_conflict = Some(SaveConflict { identity, content_revision });
                self.layout.overlay = OverlayState::SaveConflict;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SaveConflictResolutionRequested(resolution) => {
                self.resolve_save_conflict(resolution)
            }
            NotoraAction::SaveConflictResolved { identity } => {
                if self.library.save_conflict.map(|conflict| conflict.identity) == Some(identity) {
                    self.library.save_conflict = None;
                    self.encrypted_conflict_copy = EncryptedConflictCopyState::Inactive;
                    self.layout.overlay = OverlayState::None;
                    self.layout.focus_target = FocusTarget::Editor;
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::FocusRequested(focus_target) => {
                self.layout.focus_target = if self.layout.overlay == OverlayState::None {
                    focus_target
                } else {
                    FocusTarget::Overlay
                };
                vec![NotoraEffect::Redraw]
            }
            _ => unreachable!("reduce routes only editor actions here"),
        }
    }

    fn reduce_dialogs(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::OpenNewDocumentMenu => self.open_new_document_menu(),
            NotoraAction::CreateRequested(kind) => self.request_note_creation(kind),
            NotoraAction::BeginEncryptedNoteCreation => self.begin_encrypted_note_creation(),
            NotoraAction::EncryptedNotePasswordChanged(password) => {
                self.update_encrypted_note_password(password)
            }
            NotoraAction::EncryptedNoteConfirmationChanged(confirmation) => {
                self.update_encrypted_note_confirmation(confirmation)
            }
            NotoraAction::EncryptedNoteDialogSubmitRequested => self.submit_encrypted_note_dialog(),
            NotoraAction::EncryptedNoteUnlockRequired { request, title, metadata, tags } => {
                self.require_encrypted_note_unlock(request, title, metadata, tags)
            }
            NotoraAction::EncryptedNoteUnlockCompleted => {
                self.encrypted_note_unlock = EncryptedNoteUnlockState::Inactive;
                self.layout.focus_target = FocusTarget::Editor;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::EncryptedConflictCopyRequired { identity, target_path } => {
                self.require_encrypted_conflict_copy(identity, target_path)
            }
            NotoraAction::OpenSettings => {
                self.layout.overlay = OverlayState::Settings;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::ProductSettingsUpdateRequested(update) => {
                vec![NotoraEffect::ApplyProductSettingsUpdate(update), NotoraEffect::Redraw]
            }
            NotoraAction::RetryProductSettingsPersistence => {
                vec![NotoraEffect::PersistProductSettings, NotoraEffect::Redraw]
            }
            NotoraAction::SettingsViewChanged => vec![NotoraEffect::Redraw],
            NotoraAction::OverlayDismissed => self.dismiss_overlay(),
            NotoraAction::EscapePressed => self.handle_escape(),
            _ => unreachable!("reduce routes only dialogs actions here"),
        }
    }
}
