use crate::action::{DocumentLoadRequest, MetadataMutation, MetadataMutationOutcome, NotoraAction};
use crate::product::{
    EncryptedDocumentUnlockOrigin, UnlockedWorkspaceDocument, WorkspaceCompletion,
};
use notora_core::{DocumentIdentity, NoteEditorMetadata, NoteId, TagSummary};

use super::NotoraRuntime;
use super::metadata_mutation_note_id;

impl NotoraRuntime {
    pub(super) fn apply_workspace_completion(&mut self, completion: WorkspaceCompletion) {
        match completion {
            WorkspaceCompletion::ConflictCopyCompleted { identity, result } => match result {
                Ok(()) => self.dispatch_action(NotoraAction::SaveConflictResolved { identity }),
                Err(message) => self.dispatch_action(NotoraAction::NoteCommandFailed(message)),
            },
            WorkspaceCompletion::NoteCommandCompleted { result } => {
                self.synchronize_open_note_path(&result);
                self.complete_pending_title_seed(&result);
                self.dispatch_action(NotoraAction::NoteCommandCompleted(result.clone()));
                self.install_created_encrypted_note(&result);
            }
            WorkspaceCompletion::NoteCommandFailed { message } => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            WorkspaceCompletion::DirectoryCommandCompleted { result } => {
                self.dispatch_action(NotoraAction::DirectoryCreationCompleted {
                    relative_path: result.relative_path,
                });
            }
            WorkspaceCompletion::DirectoryCommandFailed { message } => {
                self.dispatch_action(NotoraAction::DirectoryCreationFailed(message));
            }
            WorkspaceCompletion::MetadataMutationCompleted {
                mutation,
                note_id,
                metadata,
                tags,
                outcome,
            } => {
                self.apply_metadata_completion(mutation, note_id, metadata, tags, outcome);
            }
            WorkspaceCompletion::MetadataMutationFailed { mutation, message } => {
                self.document_runtime
                    .complete_metadata_mutation(&mutation, metadata_mutation_note_id(&mutation));
                self.dispatch_action(NotoraAction::MetadataMutationFailed(message));
            }
            WorkspaceCompletion::CatalogBackupCompleted { .. } => {}
            WorkspaceCompletion::CatalogBackupFailed { message } => {
                self.dispatch_action(NotoraAction::MetadataMutationFailed(format!(
                    "元数据已保存，但目录索引备份失败：{message}"
                )));
            }
            WorkspaceCompletion::CatalogRecoveryNotified { message } => {
                self.dispatch_action(NotoraAction::CatalogRecoveryNotified(message));
            }
            WorkspaceCompletion::TrashOperationCompleted { operation } => {
                self.complete_trash_operation(operation);
                self.schedule_catalog_backup();
                self.request_navigation_tree();
                self.dispatch_action(NotoraAction::TrashOperationCompleted);
            }
            WorkspaceCompletion::TrashOperationFailed { failure } => {
                self.dispatch_action(NotoraAction::TrashOperationFailed(failure));
            }
            WorkspaceCompletion::DocumentLoaded { request, document, metadata, tags } => {
                self.install_loaded_preview(request, document);
                self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
                    request,
                    metadata,
                    tags,
                });
            }
            WorkspaceCompletion::EncryptedDocumentUnlockRequired {
                request,
                title,
                metadata,
                tags,
            } if self.selection_matches(request) => {
                if let DocumentIdentity::Note(note_id) = request.identity {
                    self.document_runtime.discard_cached_unlocked_workspace_note_session(note_id);
                }
                self.dispatch_action(NotoraAction::EncryptedNoteUnlockRequired {
                    request,
                    title,
                    metadata,
                    tags,
                });
            }
            WorkspaceCompletion::EncryptedDocumentUnlockRequired { .. } => {}
            WorkspaceCompletion::EncryptedDocumentUnlocked { unlocked } => {
                self.apply_unlocked_workspace_document(unlocked);
            }
            WorkspaceCompletion::EncryptedDocumentUnlockFailed { request, generation, message }
                if self.selection_matches(request)
                    && self.action_runtime.has_pending_unlock(request, generation) =>
            {
                self.dispatch_action(NotoraAction::NoteCommandFailed(if message.is_empty() {
                    "密码错误".to_owned()
                } else {
                    message
                }));
            }
            WorkspaceCompletion::EncryptedDocumentUnlockFailed { .. } => {}
            WorkspaceCompletion::DocumentLoadFailed { request, message }
                if self.selection_matches(request) =>
            {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            WorkspaceCompletion::DocumentLoadFailed { .. } => {}
            WorkspaceCompletion::WorkspaceScanCompleted { .. } => {
                self.document_runtime.record_catalog_reconciliation(false);
                self.request_navigation_tree();
                self.dispatch_action(NotoraAction::CatalogReindexed);
            }
            WorkspaceCompletion::WorkspaceIndexFailed { message } => {
                self.document_runtime.record_catalog_reconciliation(true);
                self.dispatch_action(NotoraAction::NavigationTreeFailed(message));
            }
            WorkspaceCompletion::CardQueryCompleted { query, page }
                if query
                    .search_generation
                    .is_none_or(|generation| self.accepts_search_generation(generation)) =>
            {
                self.dispatch_action(NotoraAction::CardQueryCompleted { query, page });
            }
            WorkspaceCompletion::CardQueryCompleted { .. } => {}
            WorkspaceCompletion::CardQueryFailed { query, message }
                if query
                    .search_generation
                    .is_none_or(|generation| self.accepts_search_generation(generation)) =>
            {
                self.dispatch_action(NotoraAction::CardQueryFailed { query, message });
            }
            WorkspaceCompletion::CardQueryFailed { .. } => {}
            WorkspaceCompletion::NavigationTreeLoaded { tree } => {
                self.dispatch_action(NotoraAction::NavigationTreeLoaded(tree));
            }
            WorkspaceCompletion::NavigationTreeFailed { message } => {
                self.dispatch_action(NotoraAction::NavigationTreeFailed(message));
            }
            WorkspaceCompletion::WorkspaceChanged { note_relocations, .. } => {
                self.synchronize_external_note_relocations(note_relocations);
            }
        }
    }

    fn apply_metadata_completion(
        &mut self,
        mutation: MetadataMutation,
        note_id: NoteId,
        metadata: NoteEditorMetadata,
        tags: Vec<TagSummary>,
        outcome: MetadataMutationOutcome,
    ) {
        let Some(selection_generation) =
            self.document_runtime.complete_metadata_mutation(&mutation, note_id)
        else {
            return;
        };
        self.apply_title_initialization_outcome(
            &mutation,
            outcome,
            note_id,
            metadata.title_revision,
        );
        let request =
            DocumentLoadRequest { identity: DocumentIdentity::Note(note_id), selection_generation };
        if !self.selection_matches(request) {
            return;
        }
        self.schedule_catalog_backup();
        self.request_navigation_tree();
        self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
            request,
            metadata: metadata.clone(),
            tags,
        });
        self.dispatch_action(NotoraAction::MetadataMutationCompleted {
            note_id,
            metadata,
            selection_generation,
        });
    }

    fn apply_unlocked_workspace_document(&mut self, unlocked: UnlockedWorkspaceDocument) {
        let UnlockedWorkspaceDocument { request, document, metadata, tags, session, origin } =
            unlocked;
        let selection = self.document_selection();
        let password_unlock = match origin {
            EncryptedDocumentUnlockOrigin::Password { generation } => {
                if !self.action_runtime.has_pending_unlock(request, generation) {
                    return;
                }
                true
            }
            EncryptedDocumentUnlockOrigin::CachedSession => false,
        };
        if !super::document_runtime::DocumentRuntime::selection_matches(request, selection) {
            return;
        }
        let outcome = self
            .document_runtime
            .install_created_encrypted_note(request, document, session, selection);
        self.apply_document_outcome(outcome);
        self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded { request, metadata, tags });
        if password_unlock {
            self.dispatch_action(NotoraAction::EncryptedNoteUnlockCompleted);
        }
    }
}
