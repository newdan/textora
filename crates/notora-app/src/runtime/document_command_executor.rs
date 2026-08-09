use appkit_core::workspace::types::TabId;

use crate::action::{MetadataMutation, NotoraAction};
use crate::autosave::AutoSaveRequest;
use crate::effect_executor::ManualSaveRequest;
use crate::external_files::CanonicalExternalPath;

use super::action_runtime::ExternalSaveAsApplication;
use super::document_runtime::{DocumentCommand, DocumentOutcome, PendingConflictRetry};

/// 文档命令解释器所依赖的窄运行时能力。
pub(super) trait DocumentCommandTarget {
    fn execute_note_command(&mut self, command: notora_core::note_command::NoteCommand);
    fn execute_trash_operation(&mut self, operation: crate::action::TrashOperation);
    fn retry_title_update(&mut self, request: notora_core::UpdateNoteTitleRequest);
    fn request_catalog_reindex(&mut self, tab_id: TabId);
    fn complete_external_save_as(
        &mut self,
        request: AutoSaveRequest,
        save_succeeded: bool,
        saved_path: Option<std::path::PathBuf>,
    );
    fn choose_external_save_path(
        &mut self,
        tab_id: TabId,
        external_file_id: notora_core::ExternalFileId,
    );
    fn canonicalize_external_save_as(
        &mut self,
        tab_id: TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        saved_path: std::path::PathBuf,
    );
    fn apply_external_save_as(
        &mut self,
        external_file_id: notora_core::ExternalFileId,
        canonical_path: CanonicalExternalPath,
    ) -> ExternalSaveAsApplication;
    fn dispatch_action(&mut self, action: NotoraAction);
    fn process_due_autosaves(&mut self);
    fn execute_metadata_mutation(&mut self, mutation: MetadataMutation) -> Vec<NotoraAction>;
    fn capture_conflict_revision(
        &mut self,
        identity: notora_core::DocumentIdentity,
        tab_id: TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    );
    fn begin_conflict_retry(
        &mut self,
        request: ManualSaveRequest,
        pending: PendingConflictRetry,
    ) -> DocumentOutcome;
    fn apply_document_outcome(&mut self, outcome: DocumentOutcome);
    fn save_conflict_copy(
        &mut self,
        identity: notora_core::DocumentIdentity,
        prepared: appkit_shell::editor_runtime::PreparedDocumentSave,
    );
    fn reload_conflict(
        &mut self,
        identity: notora_core::DocumentIdentity,
        tab_id: TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    );
    fn read_external_files(&mut self, requests: Vec<(std::path::PathBuf, bool)>);
    fn load_external_document(
        &mut self,
        request: crate::action::DocumentLoadRequest,
        canonical_path: CanonicalExternalPath,
    );
}

pub(super) struct DocumentCommandExecutor;

impl DocumentCommandExecutor {
    pub(super) fn execute<T: DocumentCommandTarget>(target: &mut T, command: DocumentCommand) {
        match command {
            DocumentCommand::ExecuteNote(command) => target.execute_note_command(command),
            DocumentCommand::ExecuteTrash(operation) => target.execute_trash_operation(operation),
            DocumentCommand::RetryTitleUpdate(request) => target.retry_title_update(request),
            DocumentCommand::RequestCatalogReindex(tab_id) => {
                target.request_catalog_reindex(tab_id);
            }
            DocumentCommand::CompleteExternalSaveAs { request, save_succeeded, saved_path } => {
                target.complete_external_save_as(request, save_succeeded, saved_path);
            }
            DocumentCommand::ChooseExternalSavePath { tab_id, external_file_id } => {
                target.choose_external_save_path(tab_id, external_file_id);
            }
            DocumentCommand::CanonicalizeExternalSaveAs {
                tab_id,
                external_file_id,
                content_revision,
                saved_path,
            } => target.canonicalize_external_save_as(
                tab_id,
                external_file_id,
                content_revision,
                saved_path,
            ),
            DocumentCommand::ApplyExternalSaveAs { external_file_id, canonical_path } => {
                match target.apply_external_save_as(external_file_id, canonical_path) {
                    ExternalSaveAsApplication::Updated => {}
                    ExternalSaveAsApplication::PathAlreadyOpen => {
                        target.dispatch_action(NotoraAction::NoteCommandFailed(
                            "另存为目标已在其他外部文件会话中打开".to_owned(),
                        ));
                    }
                    ExternalSaveAsApplication::SessionClosed => {
                        target.dispatch_action(NotoraAction::NoteCommandFailed(
                            "另存为完成前外部文件会话已关闭".to_owned(),
                        ));
                    }
                }
            }
            DocumentCommand::ProcessDueAutosaves => target.process_due_autosaves(),
            DocumentCommand::ExecuteMetadataMutation(mutation) => {
                for action in target.execute_metadata_mutation(mutation) {
                    target.dispatch_action(action);
                }
            }
            DocumentCommand::CaptureConflictRevision {
                identity,
                tab_id,
                content_revision,
                path,
            } => target.capture_conflict_revision(identity, tab_id, content_revision, path),
            DocumentCommand::BeginConflictRetry { request, pending } => {
                let outcome = target.begin_conflict_retry(request, pending);
                target.apply_document_outcome(outcome);
            }
            DocumentCommand::SaveConflictCopy { identity, prepared } => {
                target.save_conflict_copy(identity, prepared);
            }
            DocumentCommand::ReloadConflict { identity, tab_id, content_revision, path } => {
                target.reload_conflict(identity, tab_id, content_revision, path);
            }
            DocumentCommand::ReadExternalFiles(requests) => target.read_external_files(requests),
            DocumentCommand::LoadExternalDocument { request, canonical_path } => {
                target.load_external_document(request, canonical_path);
            }
        }
    }
}
