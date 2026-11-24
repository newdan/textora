use crate::action::NotoraAction;
use crate::external_files::CanonicalExternalPath;
use notora_core::ExternalFileId;

use super::NotoraRuntime;
use super::action_runtime::{ActionRuntime, ExternalSaveAsApplication};
use super::document_runtime::{DocumentCommand, DocumentRuntime};

impl NotoraRuntime {
    pub(super) fn execute_document_command(&mut self, command: DocumentCommand) {
        match command {
            DocumentCommand::ExecuteNote(command) => self.submit_note_command(command),
            DocumentCommand::ExecuteTrash(operation) => self.submit_trash_operation(operation),
            DocumentCommand::RetryTitleUpdate(request) => {
                self.submit_or_defer_title_update(request)
            }
            DocumentCommand::RequestCatalogReindex { tab_id, saved_path } => {
                self.request_catalog_reindex_after_note_save(tab_id, saved_path.as_deref());
            }
            DocumentCommand::ChooseExternalSavePath { tab_id, external_file_id } => {
                self.save_untitled_external_file(tab_id, external_file_id);
            }
            DocumentCommand::CanonicalizeExternalSaveAs {
                tab_id,
                external_file_id,
                content_revision,
                saved_path,
            } => self.start_external_save_as_canonicalization(
                tab_id,
                external_file_id,
                content_revision,
                saved_path,
            ),
            DocumentCommand::ApplyExternalSaveAs { external_file_id, canonical_path } => {
                if let Some(action) = apply_external_save_as(
                    &mut self.action_runtime,
                    &mut self.document_runtime,
                    external_file_id,
                    canonical_path,
                ) {
                    self.dispatch_action(action);
                }
            }
            DocumentCommand::ProcessDueAutosaves => self.process_due_autosaves(),
            DocumentCommand::ExecuteMetadataMutation(mutation) => {
                for action in self.execute_metadata_mutation(mutation) {
                    self.dispatch_action(action);
                }
            }
            DocumentCommand::CaptureConflictRevision {
                identity,
                tab_id,
                content_revision,
                path,
            } => {
                self.start_conflict_retry_revision_capture(identity, tab_id, content_revision, path)
            }
            DocumentCommand::SaveConflictCopy { identity, prepared, transform } => {
                self.start_conflict_copy(identity, prepared, transform);
            }
            DocumentCommand::ReloadConflict {
                identity,
                tab_id,
                content_revision,
                path,
                session,
            } => {
                self.start_conflict_reload(identity, tab_id, content_revision, path, session);
            }
            DocumentCommand::ReadExternalFiles(requests) => {
                self.start_external_file_reads(requests)
            }
            DocumentCommand::LoadExternalDocument { request, canonical_path } => {
                self.start_external_document_load(request, canonical_path);
            }
        }
    }
}

fn apply_external_save_as(
    actions: &mut ActionRuntime,
    documents: &mut DocumentRuntime,
    external_file_id: ExternalFileId,
    canonical_path: CanonicalExternalPath,
) -> Option<NotoraAction> {
    let message = match actions.apply_external_save_as(external_file_id, canonical_path) {
        ExternalSaveAsApplication::Updated => {
            return documents
                .take_external_file_close_after_save(external_file_id)
                .then_some(NotoraAction::ExternalFileCloseRequested(external_file_id));
        }
        ExternalSaveAsApplication::PathAlreadyOpen => "另存为目标已在其他外部文件会话中打开",
        ExternalSaveAsApplication::SessionClosed => "另存为完成前外部文件会话已关闭",
    };
    documents.cancel_external_file_close_after_save(external_file_id);
    Some(NotoraAction::NoteCommandFailed(message.to_owned()))
}
