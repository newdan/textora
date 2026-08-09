use std::collections::{HashMap, VecDeque};

use appkit_core::workspace::types::TabId;
use appkit_shell::editor_runtime::{
    DocumentTextEditError, DocumentTextReplacement, EditorNotification, EditorOutcome,
    EditorRuntime, OpenDisposition, PreparedDocumentSave,
};
use appkit_shell::prepared_tab::PreparedTab;
use appkit_shell::tab_runtime::DocumentEditingAccess;
use appkit_shell::{ShellEffect, ShellEvent};
use notora_core::note_command::{MoveNoteRequest, NoteCommand};
use notora_core::{
    DocumentIdentity, DocumentKind, ExternalFileId, NoteEditorMetadata, NoteId,
    UpdateNoteTitleRequest, document_title_projection, replace_document_title,
};

use crate::action::MetadataMutation;
use crate::autosave::{AutoSaveRequest, AutoSaveScheduler, SystemAutoSaveClock};
use crate::document_registry::DocumentRegistry;
use crate::editor_adapter::{
    LoadedDocument, prepare_loaded_document_with_access, prepare_untitled_document,
};
use crate::effect_executor::ManualSaveRequest;
use crate::external_files::{CanonicalExternalPath, ExternalFileSession};
use crate::runtime_lru::{RuntimeLru, RuntimeTabState};
use crate::state::normalize_notora_title;
use winit::event_loop::EventLoopProxy;

const TRASH_SAVE_FAILURE_MESSAGE: &str = "笔记保存失败，因此未移入回收站";
const TRASH_SAVE_STALE_MESSAGE: &str = "笔记在保存完成前发生变化，因此未移入回收站";
const MOVE_SAVE_FAILURE_MESSAGE: &str = "笔记保存失败，因此未移动";
const MOVE_SAVE_STALE_MESSAGE: &str = "笔记在保存完成前发生变化，因此未移动";
const TITLE_SAVE_FAILURE_MESSAGE: &str = "笔记保存失败，因此未更新标题和文件名";

#[derive(Clone, Copy)]
pub(super) struct DocumentSelection {
    pub(super) identity: Option<DocumentIdentity>,
    pub(super) generation: u64,
    pub(super) editing_access: DocumentEditingAccess,
}

pub(super) struct TitleCommitContext {
    pub(super) selected_identity: Option<DocumentIdentity>,
    pub(super) editable_workspace_note: bool,
    pub(super) metadata: Option<NoteEditorMetadata>,
}

#[derive(Default)]
pub(super) struct DocumentOutcome {
    pub(super) actions: Vec<crate::action::NotoraAction>,
    pub(super) notifications: Vec<EditorNotification>,
    pub(super) shell_effect: ShellEffect,
    pub(super) needs_redraw: bool,
    pub(super) commands: Vec<DocumentCommand>,
}

pub(super) enum DocumentCommand {
    ExecuteNote(NoteCommand),
    ExecuteTrash(crate::action::TrashOperation),
    RetryTitleUpdate(UpdateNoteTitleRequest),
    RequestCatalogReindex(TabId),
    CompleteExternalSaveAs {
        request: AutoSaveRequest,
        save_succeeded: bool,
        saved_path: Option<std::path::PathBuf>,
    },
    ChooseExternalSavePath {
        tab_id: TabId,
        external_file_id: ExternalFileId,
    },
    CanonicalizeExternalSaveAs {
        tab_id: TabId,
        external_file_id: ExternalFileId,
        content_revision: u64,
        saved_path: std::path::PathBuf,
    },
    ApplyExternalSaveAs {
        external_file_id: ExternalFileId,
        canonical_path: CanonicalExternalPath,
    },
    ProcessDueAutosaves,
    ExecuteMetadataMutation(crate::action::MetadataMutation),
    CaptureConflictRevision {
        identity: DocumentIdentity,
        tab_id: TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    },
    BeginConflictRetry {
        request: ManualSaveRequest,
        pending: PendingConflictRetry,
    },
    SaveConflictCopy {
        identity: DocumentIdentity,
        prepared: PreparedDocumentSave,
    },
    ReloadConflict {
        identity: DocumentIdentity,
        tab_id: TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    },
    ReadExternalFiles(Vec<(std::path::PathBuf, bool)>),
    LoadExternalDocument {
        request: crate::action::DocumentLoadRequest,
        canonical_path: CanonicalExternalPath,
    },
}

impl DocumentOutcome {
    fn absorb_editor_outcome(&mut self, outcome: EditorOutcome) {
        self.notifications.extend(outcome.notifications);
        self.shell_effect = self.shell_effect.merge(outcome.shell_effect);
    }

    fn failure(message: String) -> Self {
        Self {
            actions: vec![crate::action::NotoraAction::NoteCommandFailed(message)],
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingExternalSaveAs {
    pub(super) external_file_id: ExternalFileId,
    pub(super) content_revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingConflictRetry {
    pub(super) identity: DocumentIdentity,
    pub(super) content_revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingTrashMove {
    pub(super) note_id: NoteId,
    pub(super) content_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingNoteMove {
    pub(super) request: MoveNoteRequest,
    pub(super) content_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingTitleUpdate {
    pub(super) request: UpdateNoteTitleRequest,
    pub(super) content_revision: u64,
}

/// editor、文档注册表与全部文档级 pending workflow 的唯一所有者。
pub(super) struct DocumentRuntime {
    pub(super) runtime_lru: RuntimeLru,
    pub(super) document_registry: DocumentRegistry,
    pub(super) autosave: AutoSaveScheduler<SystemAutoSaveClock>,
    pub(super) save_failure_messages: HashMap<TabId, String>,
    pub(super) pending_external_save_as: HashMap<TabId, PendingExternalSaveAs>,
    pub(super) pending_external_documents: HashMap<ExternalFileId, LoadedDocument>,
    pub(super) pending_conflict_retries: HashMap<TabId, PendingConflictRetry>,
    pub(super) pending_trash_moves: HashMap<TabId, PendingTrashMove>,
    pub(super) pending_note_moves: HashMap<TabId, PendingNoteMove>,
    pub(super) pending_title_updates: HashMap<TabId, PendingTitleUpdate>,
    pub(super) pending_title_seeds: HashMap<NoteId, String>,
    pub(super) pending_metadata_generations: HashMap<NoteId, VecDeque<u64>>,
    pub(super) pending_metadata_mutations: Vec<MetadataMutation>,
    pub(super) catalog_reconciliation_pending: bool,
    pub(super) editor_runtime: EditorRuntime,
}

impl DocumentRuntime {
    pub(super) fn new(
        runtime_lru: RuntimeLru,
        autosave: AutoSaveScheduler<SystemAutoSaveClock>,
        editor_runtime: EditorRuntime,
    ) -> Self {
        Self {
            runtime_lru,
            document_registry: DocumentRegistry::default(),
            autosave,
            save_failure_messages: HashMap::new(),
            pending_external_save_as: HashMap::new(),
            pending_external_documents: HashMap::new(),
            pending_conflict_retries: HashMap::new(),
            pending_trash_moves: HashMap::new(),
            pending_note_moves: HashMap::new(),
            pending_title_updates: HashMap::new(),
            pending_title_seeds: HashMap::new(),
            pending_metadata_generations: HashMap::new(),
            pending_metadata_mutations: Vec::new(),
            catalog_reconciliation_pending: false,
            editor_runtime,
        }
    }

    pub(super) fn reset_workspace_state(&mut self) {
        self.autosave.clear();
        self.save_failure_messages.clear();
        self.pending_external_save_as.clear();
        self.pending_conflict_retries.clear();
        self.pending_trash_moves.clear();
        self.pending_note_moves.clear();
        self.pending_title_updates.clear();
        self.pending_title_seeds.clear();
        self.pending_metadata_generations.clear();
        self.pending_metadata_mutations.clear();
        self.catalog_reconciliation_pending = false;
    }

    pub(super) fn register_metadata_mutation(
        &mut self,
        mutation: MetadataMutation,
        note_id: NoteId,
        selection_generation: u64,
    ) -> bool {
        if self.pending_metadata_mutations.contains(&mutation) {
            return false;
        }
        self.pending_metadata_mutations.push(mutation);
        self.pending_metadata_generations
            .entry(note_id)
            .or_default()
            .push_back(selection_generation);
        true
    }

    pub(super) fn prepare_trash_operation(
        &mut self,
        operation: crate::action::TrashOperation,
        origin: Option<notora_core::DocumentOrigin>,
    ) -> DocumentOutcome {
        let crate::action::TrashOperation::MoveToTrash { note_id } = operation else {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteTrash(operation)],
                ..DocumentOutcome::default()
            };
        };
        let Some(tab_id) = self.document_registry.tab_for(DocumentIdentity::Note(note_id)) else {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteTrash(operation)],
                ..DocumentOutcome::default()
            };
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome {
                actions: vec![crate::action::NotoraAction::TrashOperationFailed(
                    crate::action::TrashOperationFailure::Message(
                        "已打开的笔记不再可用".to_owned(),
                    ),
                )],
                ..DocumentOutcome::default()
            };
        };
        if !summary.dirty {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteTrash(operation)],
                ..DocumentOutcome::default()
            };
        }
        let Some(origin) = origin else {
            return DocumentOutcome {
                actions: vec![crate::action::NotoraAction::TrashOperationFailed(
                    crate::action::TrashOperationFailure::Message(
                        "只有工作区笔记可以移入回收站".to_owned(),
                    ),
                )],
                ..DocumentOutcome::default()
            };
        };
        self.pending_trash_moves.insert(
            tab_id,
            PendingTrashMove { note_id, content_revision: summary.content_revision },
        );
        self.autosave.request_immediate_save(&origin, tab_id, summary.content_revision);
        DocumentOutcome {
            commands: vec![DocumentCommand::ProcessDueAutosaves],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn prepare_note_move(
        &mut self,
        request: MoveNoteRequest,
        origin: Option<notora_core::DocumentOrigin>,
    ) -> DocumentOutcome {
        let identity = DocumentIdentity::Note(request.note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteNote(NoteCommand::Move(request))],
                ..DocumentOutcome::default()
            };
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::failure("已打开的笔记不再可用，因此未移动".to_owned());
        };
        if !summary.dirty {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteNote(NoteCommand::Move(request))],
                ..DocumentOutcome::default()
            };
        }
        let Some(origin) = origin else {
            return DocumentOutcome::failure("只有工作区笔记可以移动".to_owned());
        };
        self.pending_note_moves.insert(
            tab_id,
            PendingNoteMove { request, content_revision: summary.content_revision },
        );
        self.autosave.request_immediate_save(&origin, tab_id, summary.content_revision);
        DocumentOutcome {
            commands: vec![DocumentCommand::ProcessDueAutosaves],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn prepare_title_update(
        &mut self,
        request: UpdateNoteTitleRequest,
        origin: Option<notora_core::DocumentOrigin>,
    ) -> DocumentOutcome {
        let identity = DocumentIdentity::Note(request.note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteNote(NoteCommand::UpdateTitle(request))],
                ..DocumentOutcome::default()
            };
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::failure("已打开的笔记不再可用，因此未更新标题".to_owned());
        };
        if !summary.dirty {
            return DocumentOutcome {
                commands: vec![DocumentCommand::ExecuteNote(NoteCommand::UpdateTitle(request))],
                ..DocumentOutcome::default()
            };
        }
        let Some(origin) = origin else {
            return DocumentOutcome::failure("只有工作区笔记可以更新标题".to_owned());
        };
        self.pending_title_updates.insert(
            tab_id,
            PendingTitleUpdate { request, content_revision: summary.content_revision },
        );
        self.autosave.request_immediate_save(&origin, tab_id, summary.content_revision);
        DocumentOutcome {
            commands: vec![DocumentCommand::ProcessDueAutosaves],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn complete_metadata_mutation(
        &mut self,
        mutation: &MetadataMutation,
        note_id: NoteId,
    ) -> Option<u64> {
        if let Some(index) =
            self.pending_metadata_mutations.iter().position(|pending| pending == mutation)
        {
            self.pending_metadata_mutations.remove(index);
        }
        let queue = self.pending_metadata_generations.get_mut(&note_id)?;
        let generation = queue.pop_front();
        if queue.is_empty() {
            self.pending_metadata_generations.remove(&note_id);
        }
        generation
    }

    pub(super) fn record_catalog_reconciliation(&mut self, pending: bool) {
        self.catalog_reconciliation_pending = pending;
    }

    pub(super) fn install_loaded_preview(
        &mut self,
        request: crate::action::DocumentLoadRequest,
        document: LoadedDocument,
        selection: DocumentSelection,
    ) -> DocumentOutcome {
        if !Self::selection_matches(request, selection) {
            return DocumentOutcome::default();
        }
        if let Some(tab_id) = self.document_registry.tab_for(request.identity) {
            self.document_registry.touch_tab(tab_id);
            let mut outcome = DocumentOutcome::default();
            outcome.absorb_editor_outcome(self.editor_runtime.activate(tab_id));
            return outcome;
        }
        let prepared = match prepare_loaded_document_with_access(
            &self.editor_runtime,
            document,
            selection.editing_access,
        ) {
            Ok(prepared) => prepared,
            Err(error) => return DocumentOutcome::failure(error.to_string()),
        };
        self.install_prepared_preview(request, prepared, None, selection)
    }

    pub(super) fn install_prepared_preview(
        &mut self,
        request: crate::action::DocumentLoadRequest,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        selection: DocumentSelection,
    ) -> DocumentOutcome {
        if !Self::selection_matches(request, selection) {
            return DocumentOutcome::default();
        }
        if let Some(tab_id) = self.document_registry.tab_for(request.identity) {
            self.document_registry.touch_tab(tab_id);
            let mut outcome = DocumentOutcome::default();
            outcome.absorb_editor_outcome(self.editor_runtime.activate(tab_id));
            return outcome;
        }
        let replaced_preview = self.document_registry.preview_tab();
        let editor_outcome = self.editor_runtime.install_prepared_tab(
            prepared,
            suggested_file_name,
            OpenDisposition::Preview,
        );
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            return DocumentOutcome::failure("编辑器运行时未激活已安装的预览".to_owned());
        };
        if let Some(replaced_preview) = replaced_preview {
            self.document_registry.remove_tab(replaced_preview);
        }
        let _ = self.document_registry.register_preview(request.identity, tab_id);
        let mut outcome = DocumentOutcome::default();
        outcome.absorb_editor_outcome(editor_outcome);
        self.evict_excess_runtime_tabs();
        outcome
    }

    pub(super) fn promote_active_preview(&mut self) -> DocumentOutcome {
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            return DocumentOutcome::default();
        };
        self.promote_preview_for_tab(tab_id)
    }

    pub(super) fn promote_preview_for_tab(&mut self, tab_id: TabId) -> DocumentOutcome {
        if self.editor_runtime.active_tab_id() != Some(tab_id)
            || self.editor_runtime.upgrade_active_preview()
                == appkit_core::navigator::NavEffect::None
            || !self.document_registry.upgrade_preview(tab_id)
        {
            return DocumentOutcome::default();
        }
        self.editor_runtime.request_redraw();
        DocumentOutcome { needs_redraw: true, ..DocumentOutcome::default() }
    }

    pub(super) fn selection_matches(
        request: crate::action::DocumentLoadRequest,
        selection: DocumentSelection,
    ) -> bool {
        selection.identity == Some(request.identity)
            && selection.generation == request.selection_generation
    }

    pub(super) fn submit_autosave(
        &mut self,
        request: AutoSaveRequest,
        event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
    ) -> DocumentOutcome {
        let Some(summary) = self.editor_runtime.document_summary(request.tab_id) else {
            self.autosave.cancel(request.tab_id);
            self.save_failure_messages.remove(&request.tab_id);
            let mut outcome = DocumentOutcome::default();
            self.cancel_pending_workflows(request, &mut outcome);
            return outcome;
        };
        if summary.content_revision != request.content_revision {
            self.handle_superseded_autosave(request, summary.content_revision, summary.dirty);
            let mut outcome = DocumentOutcome { needs_redraw: true, ..DocumentOutcome::default() };
            self.cancel_pending_trash_move_into(request, TRASH_SAVE_STALE_MESSAGE, &mut outcome);
            self.cancel_pending_note_move_into(request, MOVE_SAVE_STALE_MESSAGE, &mut outcome);
            if let Some(pending) = self.pending_title_updates.remove(&request.tab_id) {
                outcome.commands.push(DocumentCommand::RetryTitleUpdate(pending.request));
            }
            return outcome;
        }
        if !summary.dirty {
            return self.finish_redundant_autosave(request);
        }
        let prepared = match self.editor_runtime.prepare_save(request.tab_id) {
            Ok(prepared) => prepared,
            Err(error) => return self.fail_autosave(request, error.to_string()),
        };
        if let Err(message) = self.submit_prepared_save(prepared, event_loop_proxy) {
            return self.fail_autosave(request, message);
        }
        DocumentOutcome::default()
    }

    pub(super) fn save_manually(
        &mut self,
        request: ManualSaveRequest,
        origin: Option<notora_core::DocumentOrigin>,
        event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
    ) -> DocumentOutcome {
        match request {
            ManualSaveRequest::Note { tab_id, content_revision } => {
                let Some(origin) = origin else {
                    return DocumentOutcome::default();
                };
                self.autosave.request_immediate_save(&origin, tab_id, content_revision);
                DocumentOutcome {
                    commands: vec![DocumentCommand::ProcessDueAutosaves],
                    ..DocumentOutcome::default()
                }
            }
            ManualSaveRequest::ExistingExternalFile { tab_id } => {
                self.submit_manual_external_save(tab_id, event_loop_proxy).1
            }
            ManualSaveRequest::UntitledExternalFile { tab_id, external_file_id } => {
                DocumentOutcome {
                    commands: vec![DocumentCommand::ChooseExternalSavePath {
                        tab_id,
                        external_file_id,
                    }],
                    ..DocumentOutcome::default()
                }
            }
        }
    }

    pub(super) fn submit_manual_external_save(
        &mut self,
        tab_id: TabId,
        event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
    ) -> (bool, DocumentOutcome) {
        let prepared = match self.editor_runtime.prepare_save(tab_id) {
            Ok(prepared) => prepared,
            Err(error) => return (false, DocumentOutcome::failure(error.to_string())),
        };
        match self.submit_prepared_save(prepared, event_loop_proxy) {
            Ok(()) => (true, DocumentOutcome::default()),
            Err(message) => (false, DocumentOutcome::failure(message)),
        }
    }

    pub(super) fn save_external_file_as_to_path(
        &mut self,
        tab_id: TabId,
        external_file_id: ExternalFileId,
        path: std::path::PathBuf,
        event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
    ) -> DocumentOutcome {
        let prepared = match self.editor_runtime.prepare_save_as(tab_id, &path) {
            Ok(prepared) => prepared,
            Err(error) => return DocumentOutcome::failure(error.to_string()),
        };
        let pending =
            PendingExternalSaveAs { external_file_id, content_revision: prepared.content_revision };
        if let Err(message) = self.submit_prepared_save(prepared, event_loop_proxy) {
            return DocumentOutcome::failure(message);
        }
        self.pending_external_save_as.insert(tab_id, pending);
        DocumentOutcome::default()
    }

    pub(super) fn complete_pending_external_save_as(
        &mut self,
        request: AutoSaveRequest,
        save_succeeded: bool,
        saved_path: Option<std::path::PathBuf>,
    ) -> DocumentOutcome {
        let Some(pending) = self.pending_external_save_as.get(&request.tab_id).copied() else {
            return DocumentOutcome::default();
        };
        if pending.content_revision != request.content_revision {
            return DocumentOutcome::default();
        }
        if !save_succeeded {
            self.pending_external_save_as.remove(&request.tab_id);
            return DocumentOutcome::default();
        }
        let Some(saved_path) = saved_path else {
            self.pending_external_save_as.remove(&request.tab_id);
            return DocumentOutcome::default();
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::CanonicalizeExternalSaveAs {
                tab_id: request.tab_id,
                external_file_id: pending.external_file_id,
                content_revision: pending.content_revision,
                saved_path,
            }],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn complete_external_save_as_canonicalization(
        &mut self,
        tab_id: TabId,
        external_file_id: ExternalFileId,
        content_revision: u64,
        result: Result<CanonicalExternalPath, String>,
    ) -> DocumentOutcome {
        let Some(pending) = self.pending_external_save_as.get(&tab_id).copied() else {
            return DocumentOutcome::default();
        };
        if pending.external_file_id != external_file_id
            || pending.content_revision != content_revision
        {
            return DocumentOutcome::default();
        }
        self.pending_external_save_as.remove(&tab_id);
        match result {
            Ok(canonical_path) => DocumentOutcome {
                commands: vec![DocumentCommand::ApplyExternalSaveAs {
                    external_file_id,
                    canonical_path,
                }],
                ..DocumentOutcome::default()
            },
            Err(message) => DocumentOutcome::failure(message),
        }
    }

    pub(super) fn open_external_paths(&self, paths: Vec<std::path::PathBuf>) -> DocumentOutcome {
        let requests = paths.into_iter().map(|path| (path, true)).collect::<Vec<_>>();
        if requests.is_empty() {
            return DocumentOutcome::default();
        }
        DocumentOutcome {
            commands: vec![DocumentCommand::ReadExternalFiles(requests)],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn restore_external_paths(
        &self,
        paths: Vec<std::path::PathBuf>,
        saved_last_path: Option<&std::path::Path>,
    ) -> DocumentOutcome {
        let requests = paths
            .into_iter()
            .map(|path| {
                let activate = saved_last_path.is_some_and(|saved_path| saved_path == path);
                (path, activate)
            })
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return DocumentOutcome::default();
        }
        DocumentOutcome {
            commands: vec![DocumentCommand::ReadExternalFiles(requests)],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn prepare_external_document(
        &mut self,
        request: crate::action::DocumentLoadRequest,
        session: Option<ExternalFileSession>,
        selection: DocumentSelection,
    ) -> DocumentOutcome {
        let unavailable =
            || DocumentOutcome::failure("外部文档不可用；请重新定位或移除对应会话".to_owned());
        let Some(session) = session else {
            return unavailable();
        };
        match session {
            ExternalFileSession::Existing { canonical_path, external_file_id, .. } => {
                if let Some(document) = self.pending_external_documents.remove(&external_file_id) {
                    return self.install_loaded_preview(request, document, selection);
                }
                DocumentOutcome {
                    commands: vec![DocumentCommand::LoadExternalDocument {
                        request,
                        canonical_path,
                    }],
                    ..DocumentOutcome::default()
                }
            }
            ExternalFileSession::Untitled { kind, .. } => {
                let (prepared, suggested_file_name) =
                    match prepare_untitled_document(&self.editor_runtime, kind) {
                        Ok(prepared) => prepared,
                        Err(error) => return DocumentOutcome::failure(error.to_string()),
                    };
                self.install_prepared_preview(
                    request,
                    prepared,
                    Some(suggested_file_name),
                    selection,
                )
            }
            ExternalFileSession::Missing { .. } => unavailable(),
        }
    }

    pub(super) fn complete_external_file_open(
        &mut self,
        identity: DocumentIdentity,
        document: LoadedDocument,
        activate: bool,
    ) -> DocumentOutcome {
        if !activate {
            return DocumentOutcome::default();
        }
        let DocumentIdentity::ExternalFile(external_file_id) = identity else {
            return DocumentOutcome::default();
        };
        if self.document_registry.tab_for(identity).is_none() {
            self.pending_external_documents.insert(external_file_id, document);
        }
        DocumentOutcome {
            actions: vec![crate::action::NotoraAction::ExternalFileOpened(identity)],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn retry_conflicted_document_save(
        &self,
        identity: DocumentIdentity,
    ) -> DocumentOutcome {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return DocumentOutcome::default();
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::default();
        };
        let Some(path) = summary.path else {
            return DocumentOutcome::default();
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::CaptureConflictRevision {
                identity,
                tab_id,
                content_revision: summary.content_revision,
                path,
            }],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn complete_conflict_retry_revision_capture(
        &mut self,
        identity: DocumentIdentity,
        tab_id: TabId,
        content_revision: u64,
        path: std::path::PathBuf,
        disk_revision: appkit_core::file_safety::DiskRevision,
        request: Option<ManualSaveRequest>,
    ) -> DocumentOutcome {
        if self.document_registry.identity_for(tab_id) != Some(identity) {
            return DocumentOutcome::default();
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::default();
        };
        if summary.content_revision != content_revision
            || !self.editor_runtime.update_document_path(tab_id, path, Some(disk_revision))
        {
            return DocumentOutcome::default();
        }
        let Some(request) = request else {
            return DocumentOutcome::default();
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::BeginConflictRetry {
                request,
                pending: PendingConflictRetry { identity, content_revision },
            }],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn begin_conflict_retry(
        &mut self,
        request: ManualSaveRequest,
        pending: PendingConflictRetry,
        origin: Option<notora_core::DocumentOrigin>,
        event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
    ) -> DocumentOutcome {
        match request {
            ManualSaveRequest::Note { tab_id, .. } => {
                self.pending_conflict_retries.insert(tab_id, pending);
                self.save_manually(request, origin, event_loop_proxy)
            }
            ManualSaveRequest::ExistingExternalFile { tab_id } => {
                let (submitted, outcome) =
                    self.submit_manual_external_save(tab_id, event_loop_proxy);
                if submitted {
                    self.pending_conflict_retries.insert(tab_id, pending);
                }
                outcome
            }
            ManualSaveRequest::UntitledExternalFile { .. } => {
                DocumentOutcome::failure("未命名文档没有可重试的磁盘冲突".to_owned())
            }
        }
    }

    pub(super) fn prepare_conflict_copy(
        &mut self,
        identity: DocumentIdentity,
        path: std::path::PathBuf,
    ) -> DocumentOutcome {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return DocumentOutcome::default();
        };
        let prepared = match self.editor_runtime.prepare_save_as(tab_id, &path) {
            Ok(prepared) => prepared,
            Err(error) => return DocumentOutcome::failure(error.to_string()),
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::SaveConflictCopy { identity, prepared }],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn reload_conflicted_document(&self, identity: DocumentIdentity) -> DocumentOutcome {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return DocumentOutcome::default();
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::default();
        };
        let Some(path) = summary.path else {
            return DocumentOutcome::default();
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::ReloadConflict {
                identity,
                tab_id,
                content_revision: summary.content_revision,
                path,
            }],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn complete_conflict_reload(
        &mut self,
        identity: DocumentIdentity,
        tab_id: TabId,
        content_revision: u64,
        loaded: LoadedDocument,
    ) -> DocumentOutcome {
        if self.document_registry.identity_for(tab_id) != Some(identity) {
            return DocumentOutcome::default();
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::default();
        };
        if summary.content_revision != content_revision {
            return DocumentOutcome::failure("加载磁盘版本时文档已发生变化".to_owned());
        }
        let prepared =
            match crate::editor_adapter::prepare_loaded_document(&self.editor_runtime, loaded) {
                Ok(prepared) => prepared,
                Err(error) => return DocumentOutcome::failure(error.to_string()),
            };
        if !self.editor_runtime.replace_document(tab_id, prepared.document) {
            return DocumentOutcome::default();
        }
        self.autosave.cancel(tab_id);
        self.save_failure_messages.remove(&tab_id);
        DocumentOutcome {
            actions: vec![crate::action::NotoraAction::SaveConflictResolved { identity }],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn commit_active_note_title(
        &mut self,
        title: String,
        context: TitleCommitContext,
    ) -> DocumentOutcome {
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            return DocumentOutcome::failure("当前没有活动笔记".to_owned());
        };
        let Some(identity @ DocumentIdentity::Note(note_id)) =
            self.document_registry.identity_for(tab_id)
        else {
            return DocumentOutcome::failure("标题只能编辑工作区笔记".to_owned());
        };
        if context.selected_identity != Some(identity) || !context.editable_workspace_note {
            return DocumentOutcome::failure("当前活动文档不是可编辑的工作区笔记".to_owned());
        }
        let Some(metadata) = context.metadata else {
            return DocumentOutcome::failure("笔记命名状态尚未加载".to_owned());
        };
        let normalized_title = normalize_notora_title(&title);
        let mutation = match metadata.title_initialization {
            notora_core::TitleInitialization::AwaitingFirstCommit => {
                crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
                    note_id,
                    title: normalized_title,
                }
            }
            notora_core::TitleInitialization::Independent => {
                return DocumentOutcome {
                    commands: vec![DocumentCommand::RetryTitleUpdate(UpdateNoteTitleRequest {
                        note_id,
                        expected_title_revision: metadata.title_revision,
                        title: normalized_title,
                    })],
                    ..DocumentOutcome::default()
                };
            }
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::ExecuteMetadataMutation(mutation)],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn initialize_title_after_save(
        &self,
        tab_id: TabId,
        saved_content_revision: u64,
        initialization: Option<notora_core::TitleInitialization>,
    ) -> DocumentOutcome {
        let Some(DocumentIdentity::Note(note_id)) = self.document_registry.identity_for(tab_id)
        else {
            return DocumentOutcome::default();
        };
        if initialization != Some(notora_core::TitleInitialization::AwaitingFirstCommit) {
            return DocumentOutcome::default();
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return DocumentOutcome::default();
        };
        let Some(path) = summary.path.as_deref() else {
            return DocumentOutcome::default();
        };
        let Some(kind @ (DocumentKind::Markdown | DocumentKind::Mindmap)) =
            DocumentKind::from_path(path)
        else {
            return DocumentOutcome::default();
        };
        let Some(snapshot) = self.editor_runtime.document_text_snapshot(tab_id) else {
            return DocumentOutcome::default();
        };
        if snapshot.content_revision != saved_content_revision {
            return DocumentOutcome::default();
        }
        let mutation = crate::action::MetadataMutation::CompleteTitleInitializationFromDocument {
            note_id,
            title: initial_title_from_document(kind, &snapshot.text),
        };
        DocumentOutcome {
            commands: vec![DocumentCommand::ExecuteMetadataMutation(mutation)],
            ..DocumentOutcome::default()
        }
    }

    pub(super) fn apply_title_initialization_outcome(
        &mut self,
        mutation: &crate::action::MetadataMutation,
        outcome: crate::action::MetadataMutationOutcome,
        note_id: NoteId,
        title_revision: u64,
    ) -> DocumentOutcome {
        let command = match (mutation, outcome) {
            (
                crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
                    title,
                    ..
                },
                crate::action::MetadataMutationOutcome::TitleInitializationWon,
            ) => {
                self.pending_title_seeds.insert(note_id, title.clone());
                Some(DocumentCommand::ExecuteNote(NoteCommand::UpdateTitle(
                    UpdateNoteTitleRequest {
                        note_id,
                        expected_title_revision: title_revision,
                        title: title.clone(),
                    },
                )))
            }
            (
                crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
                    title,
                    ..
                },
                crate::action::MetadataMutationOutcome::TitleInitializationLost,
            )
            | (
                crate::action::MetadataMutation::CompleteTitleInitializationFromDocument {
                    title: Some(title),
                    ..
                },
                crate::action::MetadataMutationOutcome::TitleInitializationWon,
            ) => Some(DocumentCommand::RetryTitleUpdate(UpdateNoteTitleRequest {
                note_id,
                expected_title_revision: title_revision,
                title: title.clone(),
            })),
            _ => None,
        };
        DocumentOutcome { commands: command.into_iter().collect(), ..DocumentOutcome::default() }
    }

    pub(super) fn complete_pending_title_seed(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) -> DocumentOutcome {
        if result.outcome != notora_core::NoteCommandOutcome::TitleUpdated {
            return DocumentOutcome::default();
        }
        let Some(title) = self.pending_title_seeds.remove(&result.note.note_id) else {
            return DocumentOutcome::default();
        };
        self.seed_document_title(result.note.note_id, &title)
    }

    fn seed_document_title(&mut self, note_id: NoteId, title: &str) -> DocumentOutcome {
        let identity = DocumentIdentity::Note(note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return DocumentOutcome::default();
        };
        let Some(snapshot) = self.editor_runtime.document_text_snapshot(tab_id) else {
            return DocumentOutcome::default();
        };
        let Some(path) =
            self.editor_runtime.document_summary(tab_id).and_then(|summary| summary.path)
        else {
            return DocumentOutcome::default();
        };
        let Some(kind @ (DocumentKind::Markdown | DocumentKind::Mindmap)) =
            DocumentKind::from_path(&path)
        else {
            return DocumentOutcome::default();
        };
        let projected_source = replace_document_title(kind, &snapshot.text, title);
        let Some((range, replacement)) =
            single_range_replacement(&snapshot.text, &projected_source)
        else {
            return DocumentOutcome::default();
        };
        let request = DocumentTextReplacement {
            tab_id,
            content_revision: snapshot.content_revision,
            range,
            replacement,
        };
        match self.editor_runtime.replace_document_text(request) {
            Ok(editor_outcome) => {
                let mut outcome = DocumentOutcome::default();
                outcome.absorb_editor_outcome(editor_outcome);
                if kind == DocumentKind::Mindmap {
                    self.move_mindmap_cursor_to_root_end(tab_id);
                }
                outcome
            }
            Err(error) => DocumentOutcome::failure(title_edit_error_message(error)),
        }
    }

    fn move_mindmap_cursor_to_root_end(&mut self, tab_id: TabId) {
        let Some(snapshot) = self.editor_runtime.document_text_snapshot(tab_id) else {
            return;
        };
        let Ok(tree) = textora_markdown::mmf::parser::parse(&snapshot.text) else {
            return;
        };
        let Some(tab) = self.editor_runtime.tab_session_mut(tab_id) else {
            return;
        };
        tab.document.cursor_mut().selection_anchor = None;
        tab.document.cursor_move_to_offset(tree.root.title_byte_range.end);
    }

    pub(super) fn drain_save_completions(&mut self) -> Vec<DocumentOutcome> {
        self.editor_runtime
            .drain_save_completions()
            .into_iter()
            .map(|completion| {
                let request = AutoSaveRequest {
                    tab_id: completion.tab_id,
                    content_revision: completion.content_revision,
                };
                let concurrent_modification = matches!(
                    &completion.result,
                    Err(appkit_core::document::DocumentSaveError::ConcurrentModification)
                );
                let failure_message =
                    completion.result.as_ref().err().map(std::string::ToString::to_string);
                let conflict_identity = concurrent_modification
                    .then(|| self.document_registry.identity_for(request.tab_id))
                    .flatten();
                let save_succeeded = completion.result.is_ok();
                let completed_conflict_retry = self
                    .pending_conflict_retries
                    .get(&request.tab_id)
                    .copied()
                    .filter(|retry| retry.content_revision == request.content_revision);
                let pending_trash_move = self
                    .pending_trash_moves
                    .get(&request.tab_id)
                    .copied()
                    .filter(|pending| pending.content_revision == request.content_revision);
                let pending_note_move = self
                    .pending_note_moves
                    .get(&request.tab_id)
                    .cloned()
                    .filter(|pending| pending.content_revision == request.content_revision);
                let pending_title_update = self
                    .pending_title_updates
                    .get(&request.tab_id)
                    .cloned()
                    .filter(|pending| pending.content_revision == request.content_revision);
                if completed_conflict_retry.is_some() {
                    self.pending_conflict_retries.remove(&request.tab_id);
                }
                let saved_path =
                    completion.result.as_ref().ok().map(|revision| revision.path.clone());
                let mut outcome = DocumentOutcome::default();
                outcome
                    .absorb_editor_outcome(self.editor_runtime.apply_save_completion(completion));
                outcome.commands.push(DocumentCommand::CompleteExternalSaveAs {
                    request,
                    save_succeeded,
                    saved_path,
                });
                if save_succeeded {
                    self.finish_successful_save(
                        request,
                        completed_conflict_retry,
                        pending_trash_move,
                        pending_note_move,
                        pending_title_update,
                        &mut outcome,
                    );
                } else {
                    self.finish_failed_save(
                        request,
                        failure_message,
                        conflict_identity,
                        &mut outcome,
                    );
                }
                outcome
            })
            .collect()
    }

    pub(super) fn pending_trash_move_has_current_saved_document(
        &self,
        tab_id: TabId,
        pending: PendingTrashMove,
    ) -> bool {
        self.editor_runtime.document_summary(tab_id).is_some_and(|summary| {
            !summary.dirty && summary.content_revision == pending.content_revision
        })
    }

    pub(super) fn pending_note_move_has_current_saved_document(
        &self,
        tab_id: TabId,
        pending: &PendingNoteMove,
    ) -> bool {
        self.editor_runtime.document_summary(tab_id).is_some_and(|summary| {
            !summary.dirty && summary.content_revision == pending.content_revision
        })
    }

    pub(super) fn pending_title_update_has_current_saved_document(
        &self,
        tab_id: TabId,
        pending: &PendingTitleUpdate,
    ) -> bool {
        self.editor_runtime.document_summary(tab_id).is_some_and(|summary| {
            !summary.dirty && summary.content_revision == pending.content_revision
        })
    }

    pub(super) fn record_autosave_failure(
        &mut self,
        request: AutoSaveRequest,
        message: String,
    ) -> DocumentOutcome {
        self.save_failure_messages.insert(request.tab_id, message);
        self.autosave.on_save_failed(request);
        self.editor_runtime.request_redraw();
        DocumentOutcome { needs_redraw: true, ..DocumentOutcome::default() }
    }

    fn finish_successful_save(
        &mut self,
        request: AutoSaveRequest,
        completed_conflict_retry: Option<PendingConflictRetry>,
        pending_trash_move: Option<PendingTrashMove>,
        pending_note_move: Option<PendingNoteMove>,
        pending_title_update: Option<PendingTitleUpdate>,
        outcome: &mut DocumentOutcome,
    ) {
        self.save_failure_messages.remove(&request.tab_id);
        self.autosave.on_save_completed(request);
        outcome.commands.push(DocumentCommand::RequestCatalogReindex(request.tab_id));
        if let Some(pending) = pending_trash_move {
            if self.pending_trash_move_has_current_saved_document(request.tab_id, pending) {
                self.pending_trash_moves.remove(&request.tab_id);
                outcome.commands.push(DocumentCommand::ExecuteTrash(
                    crate::action::TrashOperation::MoveToTrash { note_id: pending.note_id },
                ));
            } else {
                self.cancel_pending_trash_move_into(request, TRASH_SAVE_STALE_MESSAGE, outcome);
            }
        }
        if let Some(pending) = pending_note_move {
            if self.pending_note_move_has_current_saved_document(request.tab_id, &pending) {
                self.pending_note_moves.remove(&request.tab_id);
                outcome
                    .commands
                    .push(DocumentCommand::ExecuteNote(NoteCommand::Move(pending.request)));
            } else {
                self.cancel_pending_note_move_into(request, MOVE_SAVE_STALE_MESSAGE, outcome);
            }
        }
        if let Some(pending) = pending_title_update {
            if self.pending_title_update_has_current_saved_document(request.tab_id, &pending) {
                self.pending_title_updates.remove(&request.tab_id);
                outcome
                    .commands
                    .push(DocumentCommand::ExecuteNote(NoteCommand::UpdateTitle(pending.request)));
            } else if let Some(pending) = self.pending_title_updates.remove(&request.tab_id) {
                outcome.commands.push(DocumentCommand::RetryTitleUpdate(pending.request));
            }
        }
        if let Some(retry) = completed_conflict_retry {
            outcome.actions.push(crate::action::NotoraAction::SaveConflictResolved {
                identity: retry.identity,
            });
        }
    }

    fn finish_failed_save(
        &mut self,
        request: AutoSaveRequest,
        failure_message: Option<String>,
        conflict_identity: Option<DocumentIdentity>,
        outcome: &mut DocumentOutcome,
    ) {
        if let Some(message) = failure_message {
            self.save_failure_messages.insert(request.tab_id, message);
            self.autosave.on_save_failed(request);
            self.editor_runtime.request_redraw();
            outcome.needs_redraw = true;
        } else {
            self.autosave.on_save_failed(request);
        }
        self.cancel_pending_workflows(request, outcome);
        if let Some(identity) = conflict_identity {
            outcome.actions.push(crate::action::NotoraAction::SaveConflictDetected {
                identity,
                content_revision: request.content_revision,
            });
        }
    }

    fn handle_superseded_autosave(
        &mut self,
        request: AutoSaveRequest,
        current_content_revision: u64,
        current_dirty: bool,
    ) {
        self.save_failure_messages.remove(&request.tab_id);
        if current_dirty {
            self.autosave.on_save_superseded(request, current_content_revision);
        } else {
            self.autosave.cancel(request.tab_id);
        }
        self.editor_runtime.request_redraw();
    }

    fn finish_redundant_autosave(&mut self, request: AutoSaveRequest) -> DocumentOutcome {
        self.autosave.cancel(request.tab_id);
        self.save_failure_messages.remove(&request.tab_id);
        self.editor_runtime.request_redraw();
        let mut outcome = DocumentOutcome { needs_redraw: true, ..DocumentOutcome::default() };
        if let Some(pending) = self
            .pending_trash_moves
            .get(&request.tab_id)
            .copied()
            .filter(|pending| pending.content_revision == request.content_revision)
        {
            self.pending_trash_moves.remove(&request.tab_id);
            outcome.commands.push(DocumentCommand::ExecuteTrash(
                crate::action::TrashOperation::MoveToTrash { note_id: pending.note_id },
            ));
        }
        if let Some(pending) = self
            .pending_note_moves
            .get(&request.tab_id)
            .cloned()
            .filter(|pending| pending.content_revision == request.content_revision)
        {
            self.pending_note_moves.remove(&request.tab_id);
            outcome.commands.push(DocumentCommand::ExecuteNote(NoteCommand::Move(pending.request)));
        }
        outcome
    }

    fn fail_autosave(&mut self, request: AutoSaveRequest, message: String) -> DocumentOutcome {
        let mut outcome = self.record_autosave_failure(request, message);
        self.cancel_pending_workflows(request, &mut outcome);
        outcome
    }

    fn cancel_pending_workflows(
        &mut self,
        request: AutoSaveRequest,
        outcome: &mut DocumentOutcome,
    ) {
        self.cancel_pending_trash_move_into(request, TRASH_SAVE_FAILURE_MESSAGE, outcome);
        self.cancel_pending_note_move_into(request, MOVE_SAVE_FAILURE_MESSAGE, outcome);
        self.cancel_pending_title_update_into(request, TITLE_SAVE_FAILURE_MESSAGE, outcome);
    }

    fn cancel_pending_trash_move_into(
        &mut self,
        request: AutoSaveRequest,
        message: &str,
        outcome: &mut DocumentOutcome,
    ) {
        let matching = self
            .pending_trash_moves
            .get(&request.tab_id)
            .is_some_and(|pending| pending.content_revision == request.content_revision);
        if !matching {
            return;
        }
        self.pending_trash_moves.remove(&request.tab_id);
        outcome.actions.push(crate::action::NotoraAction::TrashOperationFailed(
            crate::action::TrashOperationFailure::Message(message.to_owned()),
        ));
    }

    fn cancel_pending_note_move_into(
        &mut self,
        request: AutoSaveRequest,
        message: &str,
        outcome: &mut DocumentOutcome,
    ) {
        let matching = self
            .pending_note_moves
            .get(&request.tab_id)
            .is_some_and(|pending| pending.content_revision == request.content_revision);
        if !matching {
            return;
        }
        self.pending_note_moves.remove(&request.tab_id);
        outcome.actions.push(crate::action::NotoraAction::NoteCommandFailed(message.to_owned()));
    }

    fn cancel_pending_title_update_into(
        &mut self,
        request: AutoSaveRequest,
        message: &str,
        outcome: &mut DocumentOutcome,
    ) {
        let matching = self
            .pending_title_updates
            .get(&request.tab_id)
            .is_some_and(|pending| pending.content_revision == request.content_revision);
        if !matching {
            return;
        }
        self.pending_title_updates.remove(&request.tab_id);
        outcome.actions.push(crate::action::NotoraAction::NoteCommandFailed(message.to_owned()));
    }

    pub(super) fn submit_prepared_save(
        &mut self,
        prepared: PreparedDocumentSave,
        event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
    ) -> Result<(), String> {
        let proxy = event_loop_proxy.ok_or_else(|| "事件循环启动前保存线程不可用".to_owned())?;
        self.editor_runtime.submit_save(prepared, move || {
            let _ = proxy.send_event(ShellEvent::SaveResultsReady);
        })
    }

    pub(super) fn evict_excess_runtime_tabs(&mut self) {
        let active_tab_id = self.editor_runtime.active_tab_id();
        let runtime_tabs = self
            .editor_runtime
            .tab_ids_in_order()
            .into_iter()
            .filter_map(|tab_id| {
                let summary = self.editor_runtime.document_summary(tab_id)?;
                Some(RuntimeTabState {
                    tab_id,
                    is_dirty: summary.dirty,
                    is_saving: matches!(
                        self.autosave.state(tab_id),
                        Some(crate::autosave::AutoSaveState::Saving { .. })
                    ),
                    is_pinned: self.editor_runtime.is_pinned(tab_id),
                    is_active: active_tab_id == Some(tab_id),
                })
            })
            .collect::<Vec<_>>();
        for candidate in self.runtime_lru.select_evictions(&self.document_registry, &runtime_tabs) {
            self.autosave.cancel(candidate.tab_id);
            self.save_failure_messages.remove(&candidate.tab_id);
            let _ = self.editor_runtime.close_for_product(candidate.tab_id);
            self.document_registry.remove_tab(candidate.tab_id);
        }
    }
}

fn single_range_replacement(
    original: &str,
    projected: &str,
) -> Option<(std::ops::Range<usize>, String)> {
    if original == projected {
        return None;
    }
    let original_bytes = original.as_bytes();
    let projected_bytes = projected.as_bytes();
    let mut prefix = 0;
    while prefix < original_bytes.len()
        && prefix < projected_bytes.len()
        && original_bytes[prefix] == projected_bytes[prefix]
    {
        prefix += 1;
    }
    while prefix > 0 && (!original.is_char_boundary(prefix) || !projected.is_char_boundary(prefix))
    {
        prefix -= 1;
    }

    let mut suffix = 0;
    while suffix < original_bytes.len().saturating_sub(prefix)
        && suffix < projected_bytes.len().saturating_sub(prefix)
        && original_bytes[original_bytes.len() - suffix - 1]
            == projected_bytes[projected_bytes.len() - suffix - 1]
    {
        suffix += 1;
    }
    while suffix > 0
        && (!original.is_char_boundary(original.len() - suffix)
            || !projected.is_char_boundary(projected.len() - suffix))
    {
        suffix -= 1;
    }

    let original_end = original.len() - suffix;
    let projected_end = projected.len() - suffix;
    Some((prefix..original_end, projected[prefix..projected_end].to_owned()))
}

fn title_edit_error_message(error: DocumentTextEditError) -> String {
    match error {
        DocumentTextEditError::UnknownTab { .. } => "当前笔记已关闭，请重新选择".to_owned(),
        DocumentTextEditError::StaleRevision { .. } => "笔记已发生变化，请重新提交标题".to_owned(),
        DocumentTextEditError::InvalidByteRange { .. } => "标题范围无效，请重新提交".to_owned(),
        DocumentTextEditError::ReadOnly { .. } => "当前笔记不可编辑".to_owned(),
    }
}

pub(super) fn initial_title_from_document(kind: DocumentKind, source: &str) -> Option<String> {
    let candidate = match kind {
        DocumentKind::Markdown => document_title_projection(kind, source).title,
        DocumentKind::Mindmap => textora_markdown::mmf::parser::parse(source).ok()?.root.title,
        DocumentKind::Text => return None,
    };
    let trimmed_candidate = candidate.trim();
    (!trimmed_candidate.is_empty()).then(|| trimmed_candidate.to_owned())
}
