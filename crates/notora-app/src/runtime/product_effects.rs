use std::time::Instant;

use appkit_shell::ShellEffect;
use notora_core::NavigationScope;

use crate::action::{NotoraAction, NotoraEffect};

use super::{NotoraRuntime, action_requires_session_persistence};

impl NotoraRuntime {
    pub(super) fn reduce_action(&mut self, action: NotoraAction) {
        let should_persist_session = action_requires_session_persistence(&action);
        let reduction = self.action_runtime.reduce(action, Instant::now(), should_persist_session);
        for effect in reduction.effects {
            let redraw = matches!(effect, NotoraEffect::Redraw | NotoraEffect::ToggleEditorView);
            for follow_up_action in self.execute_product_effect(effect) {
                self.action_runtime.enqueue(follow_up_action);
            }
            if redraw {
                self.apply_shell_effect(ShellEffect::REDRAW);
            }
        }
        self.synchronize_product_focus();
        for follow_up_action in reduction.follow_up_actions {
            self.action_runtime.enqueue(follow_up_action);
        }
        if reduction.should_persist_session {
            self.schedule_session_persistence();
        }
    }

    fn execute_product_effect(&mut self, effect: NotoraEffect) -> Vec<NotoraAction> {
        match effect {
            NotoraEffect::QueryCards(query) => return self.query_cards(query),
            NotoraEffect::RequestNoteCreation { kind, target } => {
                return self.request_note_creation(kind, target);
            }
            NotoraEffect::ExecuteNoteCommand(command) => self.execute_note_command(command),
            NotoraEffect::ExecuteDirectoryCommand(command) => {
                self.execute_directory_command(command)
            }
            NotoraEffect::ChooseWorkspaceCreationLocation => {
                return (self.workspace_directory_chooser)()
                    .map(NotoraAction::WorkspaceCreationLocationSelected)
                    .into_iter()
                    .collect();
            }
            NotoraEffect::PrepareWorkspaceTransition(request) => {
                self.prepare_workspace_transition(request);
            }
            NotoraEffect::CommitTitle(title) => self.commit_title(title),
            NotoraEffect::ToggleEditorView => self.toggle_editor_view(),
            NotoraEffect::ToggleInspectorPanel => self.toggle_inspector_panel(),
            NotoraEffect::DispatchInspectorPanel(action) => {
                self.dispatch_inspector_panel(action);
            }
            NotoraEffect::ExecuteSemanticEdit(command) => self.execute_semantic_edit(command),
            NotoraEffect::ExecuteMetadataMutation(mutation) => {
                return self.execute_metadata_mutation(mutation);
            }
            NotoraEffect::ExecuteTrashOperation(operation) => {
                self.execute_trash_operation(operation)
            }
            NotoraEffect::ChooseNoteMoveDirectory(note_id) => {
                return self.choose_note_move_directory(note_id);
            }
            NotoraEffect::PrepareDocument(request) => return self.prepare_document(request),
            NotoraEffect::UnlockEncryptedNote(request) => {
                let trashed =
                    self.action_runtime.state().library.navigation_scope == NavigationScope::Trash;
                if let Err(error) =
                    self.workspace_controller.unlock_encrypted_document(request, trashed)
                {
                    return vec![NotoraAction::NoteCommandFailed(error.to_string())];
                }
            }
            NotoraEffect::SaveEncryptedConflictCopy(request) => {
                let outcome = self.document_runtime.prepare_encrypted_conflict_copy(
                    request.identity,
                    request.target_path,
                    request.password,
                );
                self.apply_document_outcome(outcome);
            }
            NotoraEffect::PromoteActivePreview => self.promote_active_preview(),
            NotoraEffect::ChooseWorkspaceRoot => self.choose_workspace_root(),
            NotoraEffect::OpenExternalFiles(request) => self.open_external_files(request),
            NotoraEffect::CreateUntitledExternal(kind) => {
                return self.create_untitled_external(kind);
            }
            NotoraEffect::CloseExternalFile(external_file_id) => {
                return self.close_external_file(external_file_id);
            }
            NotoraEffect::CloseAllExternalFiles => return self.close_all_external_files(),
            NotoraEffect::ResolveSaveConflict(request) => self.resolve_save_conflict(request),
            NotoraEffect::ApplyProductSettingsUpdate(update) => {
                self.apply_product_settings_update(update)
            }
            NotoraEffect::PersistProductSettings => self.persist_product_settings(),
            NotoraEffect::PersistLayout => self.persist_layout(),
            NotoraEffect::Redraw => {}
        }
        Vec::new()
    }
}
