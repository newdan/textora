use appkit_shell::{ProductHost, ShellEffect};

use crate::action::{DocumentLoadRequest, MetadataMutation, MetadataMutationOutcome, NotoraAction};
use crate::editor_adapter::LoadedDocument;
use crate::external_files::CanonicalExternalPath;
use crate::product::{NotoraProduct, NotoraProductEvent, WorkspaceBootstrapCompletion};

use super::document_completion_interpreter::DocumentCompletionInterpreter;
use super::persistence_completion_interpreter::PersistenceCompletionInterpreter;
use super::workspace_completion_interpreter::WorkspaceCompletionInterpreter;

pub(super) trait ProductActionTarget {
    fn dispatch_action(&mut self, action: NotoraAction);
}

pub(super) trait LoadedDocumentTarget: ProductActionTarget {
    fn install_loaded_preview(&mut self, request: DocumentLoadRequest, document: LoadedDocument);
    fn selection_matches(&self, request: DocumentLoadRequest) -> bool;
}

/// Workspace 完成事件解释器所需的窄能力集合。
pub(super) trait WorkspaceCompletionTarget: LoadedDocumentTarget {
    fn accepts_encrypted_unlock(&self, request: DocumentLoadRequest, _generation: u64) -> bool {
        self.selection_matches(request)
    }
    fn install_unlocked_workspace_document(
        &mut self,
        _unlocked: crate::product::UnlockedWorkspaceDocument,
    ) {
    }
    fn install_created_encrypted_note(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    );
    fn synchronize_open_note_path(&mut self, result: &notora_core::note_command::NoteCommandResult);
    fn complete_pending_title_seed(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    );
    fn complete_metadata_mutation(
        &mut self,
        mutation: &MetadataMutation,
        note_id: notora_core::NoteId,
    ) -> Option<u64>;
    fn apply_title_initialization_outcome(
        &mut self,
        mutation: &MetadataMutation,
        outcome: MetadataMutationOutcome,
        note_id: notora_core::NoteId,
        title_revision: u64,
    );
    fn selected_document(&self) -> (Option<notora_core::DocumentIdentity>, u64);
    fn schedule_catalog_backup(&mut self);
    fn request_navigation_tree(&mut self);
    fn complete_trash_operation(&mut self, operation: crate::action::TrashOperation);
    fn record_catalog_reconciliation(&mut self, pending: bool);
    fn accepts_search_generation(
        &self,
        generation: crate::search_controller::SearchGeneration,
    ) -> bool;
    fn synchronize_external_note_relocations(
        &mut self,
        relocations: Vec<crate::product::WorkspaceNoteRelocation>,
    );
}

/// 外部文档与冲突完成事件解释器所需的窄能力集合。
pub(super) trait DocumentCompletionTarget: LoadedDocumentTarget {
    fn complete_external_file_open(
        &mut self,
        canonical_path: CanonicalExternalPath,
        document: LoadedDocument,
        activate: bool,
    );
    fn complete_external_save_as_canonicalization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        result: Result<CanonicalExternalPath, String>,
    );
    fn complete_conflict_reload(
        &mut self,
        identity: notora_core::DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        document: LoadedDocument,
    );
    fn relock_conflicted_document(
        &mut self,
        identity: notora_core::DocumentIdentity,
        _tab_id: appkit_core::workspace::types::TabId,
    ) {
        self.dispatch_action(NotoraAction::NoteCommandFailed(format!(
            "加密文件已被替换，需要重新解锁：{identity:?}"
        )));
    }
    fn active_save_conflict_identity(&self) -> Option<notora_core::DocumentIdentity>;
    fn complete_conflict_retry_revision_capture(
        &mut self,
        identity: notora_core::DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
        disk_revision: appkit_core::file_safety::DiskRevision,
    );
}

pub(super) trait PersistenceCompletionTarget: ProductActionTarget {
    fn record_settings_persistence_result(&mut self, result: Result<(), String>);
}

pub(super) trait WorkspaceBootstrapTarget {
    fn complete_workspace_bootstrap(&mut self, completion: WorkspaceBootstrapCompletion);
}

/// 顶层协调器仅要求三个分域协议的交集，不暴露具体组合根类型。
pub(super) trait ProductEventTarget:
    WorkspaceBootstrapTarget
    + WorkspaceCompletionTarget
    + DocumentCompletionTarget
    + PersistenceCompletionTarget
{
}

impl<T> ProductEventTarget for T where
    T: WorkspaceBootstrapTarget
        + WorkspaceCompletionTarget
        + DocumentCompletionTarget
        + PersistenceCompletionTarget
{
}

/// 一次 product inbox drain 的有序、强类型结果。
pub(super) struct ProductCompletions {
    pub(super) shell_effect: ShellEffect,
    pub(super) events: Vec<NotoraProductEvent>,
}

pub(super) struct ProductEventCoordinator;

impl ProductEventCoordinator {
    pub(super) fn drain(product: &mut NotoraProduct) -> ProductCompletions {
        let shell_effect = ProductHost::drain_product_events(product);
        let events = product.take_events();
        ProductCompletions { shell_effect, events }
    }

    pub(super) fn apply<T: ProductEventTarget>(target: &mut T, event: NotoraProductEvent) {
        match event {
            NotoraProductEvent::WorkspaceBootstrap(completion) => {
                target.complete_workspace_bootstrap(completion);
            }
            NotoraProductEvent::Workspace(event) => {
                WorkspaceCompletionInterpreter::apply(target, event.completion);
            }
            NotoraProductEvent::Document(completion) => {
                DocumentCompletionInterpreter::apply(target, completion);
            }
            NotoraProductEvent::Persistence(completion) => {
                PersistenceCompletionInterpreter::apply(target, completion);
            }
        }
    }
}
