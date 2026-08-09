use crate::action::{DocumentLoadRequest, MetadataMutation, NotoraAction};
use crate::product::WorkspaceCompletion;

use super::product_event_coordinator::WorkspaceCompletionTarget;

pub(super) struct WorkspaceCompletionInterpreter;

impl WorkspaceCompletionInterpreter {
    pub(super) fn apply<T: WorkspaceCompletionTarget>(
        target: &mut T,
        completion: WorkspaceCompletion,
    ) {
        match completion {
            WorkspaceCompletion::ConflictCopyCompleted { identity, result } => match result {
                Ok(()) => target.dispatch_action(NotoraAction::SaveConflictResolved { identity }),
                Err(message) => target.dispatch_action(NotoraAction::NoteCommandFailed(message)),
            },
            WorkspaceCompletion::NoteCommandCompleted { result } => {
                target.synchronize_open_note_path(&result);
                target.complete_pending_title_seed(&result);
                target.dispatch_action(NotoraAction::NoteCommandCompleted(result));
            }
            WorkspaceCompletion::NoteCommandFailed { message } => {
                target.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            WorkspaceCompletion::MetadataMutationCompleted {
                mutation,
                note_id,
                metadata,
                tags,
                outcome,
            } => {
                let Some(selection_generation) =
                    target.complete_metadata_mutation(&mutation, note_id)
                else {
                    return;
                };
                target.apply_title_initialization_outcome(
                    &mutation,
                    outcome,
                    note_id,
                    metadata.title_revision,
                );
                let (selected_identity, selected_generation) = target.selected_document();
                if selected_identity != Some(notora_core::DocumentIdentity::Note(note_id))
                    || selected_generation != selection_generation
                {
                    return;
                }
                target.schedule_catalog_backup();
                target.request_navigation_tree();
                target.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
                    request: DocumentLoadRequest {
                        identity: notora_core::DocumentIdentity::Note(note_id),
                        selection_generation,
                    },
                    metadata: metadata.clone(),
                    tags,
                });
                target.dispatch_action(NotoraAction::MetadataMutationCompleted {
                    note_id,
                    metadata,
                    selection_generation,
                });
            }
            WorkspaceCompletion::MetadataMutationFailed { mutation, message } => {
                target.complete_metadata_mutation(&mutation, metadata_mutation_note_id(&mutation));
                target.dispatch_action(NotoraAction::MetadataMutationFailed(message));
            }
            WorkspaceCompletion::CatalogBackupCompleted { .. } => {}
            WorkspaceCompletion::CatalogBackupFailed { message } => {
                target.dispatch_action(NotoraAction::MetadataMutationFailed(format!(
                    "元数据已保存，但目录索引备份失败：{message}"
                )));
            }
            WorkspaceCompletion::CatalogRecoveryNotified { message } => {
                target.dispatch_action(NotoraAction::CatalogRecoveryNotified(message));
            }
            WorkspaceCompletion::TrashOperationCompleted { operation } => {
                target.complete_trash_operation(operation);
                target.schedule_catalog_backup();
                target.request_navigation_tree();
                target.dispatch_action(NotoraAction::TrashOperationCompleted);
            }
            WorkspaceCompletion::TrashOperationFailed { failure } => {
                target.dispatch_action(NotoraAction::TrashOperationFailed(failure));
            }
            WorkspaceCompletion::DocumentLoaded { request, document, metadata, tags } => {
                target.install_loaded_preview(request, document);
                target.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
                    request,
                    metadata,
                    tags,
                });
            }
            WorkspaceCompletion::DocumentLoadFailed { request, message }
                if target.selection_matches(request) =>
            {
                target.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            WorkspaceCompletion::DocumentLoadFailed { .. } => {}
            WorkspaceCompletion::WorkspaceScanCompleted { .. } => {
                target.record_catalog_reconciliation(false);
                target.request_navigation_tree();
                target.dispatch_action(NotoraAction::CatalogReindexed);
            }
            WorkspaceCompletion::WorkspaceIndexFailed { message } => {
                target.record_catalog_reconciliation(true);
                target.dispatch_action(NotoraAction::NavigationTreeFailed(message));
            }
            WorkspaceCompletion::CardQueryCompleted { query, page }
                if query
                    .search_generation
                    .is_none_or(|generation| target.accepts_search_generation(generation)) =>
            {
                target.dispatch_action(NotoraAction::CardQueryCompleted { query, page });
            }
            WorkspaceCompletion::CardQueryCompleted { .. } => {}
            WorkspaceCompletion::CardQueryFailed { query, message }
                if query
                    .search_generation
                    .is_none_or(|generation| target.accepts_search_generation(generation)) =>
            {
                target.dispatch_action(NotoraAction::CardQueryFailed { query, message });
            }
            WorkspaceCompletion::CardQueryFailed { .. } => {}
            WorkspaceCompletion::NavigationTreeLoaded { tree } => {
                target.dispatch_action(NotoraAction::NavigationTreeLoaded(tree));
            }
            WorkspaceCompletion::NavigationTreeFailed { message } => {
                target.dispatch_action(NotoraAction::NavigationTreeFailed(message));
            }
            WorkspaceCompletion::WorkspaceChanged { note_relocations, .. } => {
                target.synchronize_external_note_relocations(note_relocations);
            }
        }
    }
}

fn metadata_mutation_note_id(mutation: &MetadataMutation) -> notora_core::NoteId {
    match mutation {
        MetadataMutation::ToggleStar { note_id }
        | MetadataMutation::AttachTagByName { note_id, .. }
        | MetadataMutation::DetachTag { note_id, .. }
        | MetadataMutation::SetTitle { note_id, .. }
        | MetadataMutation::CompleteTitleInitializationFromHeader { note_id, .. }
        | MetadataMutation::CompleteTitleInitializationFromDocument { note_id, .. } => *note_id,
    }
}
