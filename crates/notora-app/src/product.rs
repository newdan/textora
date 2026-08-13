use appkit_shell::{
    ProductEventInbox, ProductEventSender, ProductHost, ProductWakeHandle, ShellEffect,
    product_event_channel,
};
use notora_core::note_command::NoteCommandResult;
use notora_core::{CatalogCardPage, ScanCompletion, WorkspaceId};

use crate::action::DocumentLoadRequest;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceNoteRelocation {
    pub note_id: notora_core::NoteId,
    pub from: std::path::PathBuf,
    pub to: std::path::PathBuf,
    pub metadata: notora_core::NoteEditorMetadata,
    pub tags: Vec<notora_core::TagSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceEventScope {
    pub workspace_id: WorkspaceId,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceCompletionEnvelope {
    pub scope: WorkspaceEventScope,
    pub completion: WorkspaceCompletion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceCompletion {
    CardQueryCompleted {
        query: crate::action::CardQuery,
        page: CatalogCardPage,
    },
    CardQueryFailed {
        query: crate::action::CardQuery,
        message: String,
    },
    NavigationTreeLoaded {
        tree: notora_core::CatalogNavigationTree,
    },
    NavigationTreeFailed {
        message: String,
    },
    WorkspaceScanCompleted {
        completion: ScanCompletion,
    },
    WorkspaceChanged {
        changed_paths: Vec<std::path::PathBuf>,
        note_relocations: Vec<WorkspaceNoteRelocation>,
    },
    WorkspaceIndexFailed {
        message: String,
    },
    NoteCommandCompleted {
        result: NoteCommandResult,
    },
    NoteCommandFailed {
        message: String,
    },
    DirectoryCommandCompleted {
        result: notora_core::WorkspaceDirectoryCommandResult,
    },
    DirectoryCommandFailed {
        message: String,
    },
    MetadataMutationCompleted {
        mutation: crate::action::MetadataMutation,
        note_id: notora_core::NoteId,
        metadata: notora_core::NoteEditorMetadata,
        tags: Vec<notora_core::TagSummary>,
        outcome: crate::action::MetadataMutationOutcome,
    },
    MetadataMutationFailed {
        mutation: crate::action::MetadataMutation,
        message: String,
    },
    CatalogBackupCompleted {
        backup_path: std::path::PathBuf,
    },
    CatalogBackupFailed {
        message: String,
    },
    CatalogRecoveryNotified {
        message: String,
    },
    TrashOperationCompleted {
        operation: crate::action::TrashOperation,
    },
    TrashOperationFailed {
        failure: crate::action::TrashOperationFailure,
    },
    DocumentLoaded {
        request: DocumentLoadRequest,
        document: crate::editor_adapter::LoadedDocument,
        metadata: notora_core::NoteEditorMetadata,
        tags: Vec<notora_core::TagSummary>,
    },
    DocumentLoadFailed {
        request: DocumentLoadRequest,
        message: String,
    },
    ConflictCopyCompleted {
        identity: notora_core::DocumentIdentity,
        result: Result<(), String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentCompletion {
    ExternalFileOpenCompleted {
        canonical_path: crate::external_files::CanonicalExternalPath,
        document: crate::editor_adapter::LoadedDocument,
        activate: bool,
    },
    ExternalFileOpenFailed {
        message: String,
    },
    ExternalDocumentLoaded {
        request: DocumentLoadRequest,
        document: crate::editor_adapter::LoadedDocument,
    },
    ExternalDocumentLoadFailed {
        request: DocumentLoadRequest,
        message: String,
    },
    ExternalSaveAsCanonicalized {
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        result: Result<crate::external_files::CanonicalExternalPath, String>,
    },
    ConflictReloadCompleted {
        identity: notora_core::DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        document: crate::editor_adapter::LoadedDocument,
    },
    ConflictReloadFailed {
        identity: notora_core::DocumentIdentity,
        message: String,
    },
    ConflictRetryRevisionCaptured {
        identity: notora_core::DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
        disk_revision: appkit_core::file_safety::DiskRevision,
    },
    ConflictRetryRevisionFailed {
        identity: notora_core::DocumentIdentity,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersistenceCompletion {
    SettingsPersistenceCompleted { result: Result<(), String> },
    SessionPersistenceFailed { message: String },
}

/// 后台服务只能经 notora 自有 channel 发送的 payload。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotoraProductEvent {
    Workspace(WorkspaceCompletionEnvelope),
    Document(DocumentCompletion),
    Persistence(PersistenceCompletion),
}

pub type NotoraProductEventSender = ProductEventSender<NotoraProductEvent>;

#[derive(Clone)]
pub struct WorkspaceEventSender {
    sender: NotoraProductEventSender,
    scope: WorkspaceEventScope,
}

impl WorkspaceEventSender {
    pub fn new(sender: NotoraProductEventSender, scope: WorkspaceEventScope) -> Self {
        Self { sender, scope }
    }

    pub fn send(
        &self,
        completion: WorkspaceCompletion,
    ) -> Result<(), appkit_shell::ProductEventSendError> {
        self.sender.send(NotoraProductEvent::Workspace(WorkspaceCompletionEnvelope {
            scope: self.scope.clone(),
            completion,
        }))
    }
}

/// 由产品持有、并在退出时有序停止的后台服务。
pub trait ProductServiceShutdown {
    fn shutdown(&mut self);
}

/// 产品服务宿主。shell 只看到唤醒和聚合后的 ShellEffect。
pub struct NotoraProduct {
    event_sender: NotoraProductEventSender,
    event_inbox: ProductEventInbox<NotoraProductEvent>,
    active_workspace: Option<(WorkspaceId, u64)>,
    pending_events: Vec<NotoraProductEvent>,
    service_shutdown_handles: Vec<Box<dyn ProductServiceShutdown>>,
    services_started: bool,
    shutdown: bool,
}

impl NotoraProduct {
    pub fn new() -> Self {
        let (event_sender, event_inbox) = product_event_channel();
        Self {
            event_sender,
            event_inbox,
            active_workspace: None,
            pending_events: Vec::new(),
            service_shutdown_handles: Vec::new(),
            services_started: false,
            shutdown: false,
        }
    }

    pub fn event_sender(&self) -> NotoraProductEventSender {
        self.event_sender.clone()
    }

    pub fn set_active_workspace(&mut self, workspace_id: WorkspaceId, workspace_generation: u64) {
        self.active_workspace = Some((workspace_id, workspace_generation));
    }

    pub fn clear_active_workspace(&mut self) {
        self.active_workspace = None;
    }

    /// 取出已完成的后台事件；过期工作区事件已在 drain 时丢弃。
    pub fn take_events(&mut self) -> Vec<NotoraProductEvent> {
        std::mem::take(&mut self.pending_events)
    }

    pub fn register_service_shutdown(
        &mut self,
        mut service: impl ProductServiceShutdown + 'static,
    ) {
        if self.shutdown {
            service.shutdown();
            return;
        }
        self.service_shutdown_handles.push(Box::new(service));
    }

    fn event_matches_active_workspace(&self, event: &NotoraProductEvent) -> bool {
        let event_workspace = match event {
            NotoraProductEvent::Workspace(event) => {
                (event.scope.workspace_id, event.scope.generation)
            }
            NotoraProductEvent::Document(_) | NotoraProductEvent::Persistence(_) => return true,
        };
        self.active_workspace == Some(event_workspace)
    }
}

impl Default for NotoraProduct {
    fn default() -> Self {
        Self::new()
    }
}

impl ProductHost for NotoraProduct {
    fn start_background_services(&mut self, wake: ProductWakeHandle) {
        if self.services_started || self.shutdown {
            return;
        }
        let _ = self.event_inbox.register_wake_handle(wake);
        self.services_started = true;
    }

    fn drain_product_events(&mut self) -> ShellEffect {
        let mut effect = ShellEffect::NONE;
        for event in self.event_inbox.drain() {
            if self.event_matches_active_workspace(&event) {
                self.pending_events.push(event);
                effect = effect.merge(ShellEffect::REDRAW);
            }
        }
        effect
    }

    fn shutdown(&mut self) {
        if self.shutdown {
            return;
        }
        self.shutdown = true;
        for service in &mut self.service_shutdown_handles {
            service.shutdown();
        }
        self.service_shutdown_handles.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use appkit_shell::{ProductHost, ShellEffect};

    use notora_core::WorkspaceId;

    use super::{
        NotoraProduct, NotoraProductEvent, ProductServiceShutdown, WorkspaceCompletion,
        WorkspaceEventScope, WorkspaceEventSender,
    };
    use crate::action::DocumentLoadRequest;

    struct ShutdownRecorder {
        call_count: Rc<Cell<usize>>,
    }

    impl ProductServiceShutdown for ShutdownRecorder {
        fn shutdown(&mut self) {
            self.call_count.set(self.call_count.get() + 1);
        }
    }

    #[test]
    fn creates_a_product_event_host() {
        let _product = NotoraProduct::new();
    }

    #[test]
    fn drain_discards_late_workspace_events_and_redraws_for_current_generation() {
        let mut product = NotoraProduct::new();
        let active_workspace_id = WorkspaceId::generate();
        product.set_active_workspace(active_workspace_id, 4);
        WorkspaceEventSender::new(
            product.event_sender(),
            WorkspaceEventScope { workspace_id: active_workspace_id, generation: 3 },
        )
        .send(WorkspaceCompletion::CardQueryCompleted {
            query: crate::action::CardQuery::from(notora_core::NavigationScope::WorkspaceRoot),
            page: notora_core::CatalogCardPage { cards: vec![], next_cursor: None },
        })
        .expect("product receiver should be alive");
        WorkspaceEventSender::new(
            product.event_sender(),
            WorkspaceEventScope { workspace_id: active_workspace_id, generation: 4 },
        )
        .send(WorkspaceCompletion::WorkspaceChanged {
            changed_paths: vec![],
            note_relocations: vec![],
        })
        .expect("product receiver should be alive");

        assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
        assert_eq!(product.drain_product_events(), ShellEffect::NONE);
    }

    #[test]
    fn drain_discards_events_from_another_workspace_with_the_same_generation() {
        let mut product = NotoraProduct::new();
        product.set_active_workspace(WorkspaceId::generate(), 4);
        WorkspaceEventSender::new(
            product.event_sender(),
            WorkspaceEventScope { workspace_id: WorkspaceId::generate(), generation: 4 },
        )
        .send(WorkspaceCompletion::WorkspaceChanged {
            changed_paths: vec![],
            note_relocations: vec![],
        })
        .expect("product receiver should be alive");

        assert_eq!(product.drain_product_events(), ShellEffect::NONE);
    }

    #[test]
    fn drain_keeps_document_load_results_for_the_active_workspace_generation() {
        let mut product = NotoraProduct::new();
        let workspace_id = WorkspaceId::generate();
        let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        product.set_active_workspace(workspace_id, 9);
        WorkspaceEventSender::new(
            product.event_sender(),
            WorkspaceEventScope { workspace_id, generation: 9 },
        )
        .send(WorkspaceCompletion::DocumentLoadFailed {
            request: DocumentLoadRequest { identity, selection_generation: 2 },
            message: "fixture read failed".to_owned(),
        })
        .expect("product receiver should be alive");

        assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
        assert!(matches!(
            product.take_events().as_slice(),
            [NotoraProductEvent::Workspace(event)]
                if event.scope.workspace_id == workspace_id
                    && event.scope.generation == 9
                    && matches!(
                        event.completion,
                        WorkspaceCompletion::DocumentLoadFailed {
                            request: DocumentLoadRequest {
                                identity: event_identity,
                                selection_generation: 2,
                            },
                            ..
                        } if event_identity == identity
                    )
        ));
    }

    #[test]
    fn sender_reports_a_disconnected_receiver() {
        let sender = WorkspaceEventSender::new(
            NotoraProduct::new().event_sender(),
            WorkspaceEventScope { workspace_id: WorkspaceId::generate(), generation: 1 },
        );
        assert!(
            sender
                .send(WorkspaceCompletion::WorkspaceChanged {
                    changed_paths: vec![],
                    note_relocations: vec![],
                })
                .is_err()
        );
    }

    #[test]
    fn repeated_shutdown_is_safe() {
        let mut product = NotoraProduct::new();
        let call_count = Rc::new(Cell::new(0));
        product.register_service_shutdown(ShutdownRecorder { call_count: Rc::clone(&call_count) });
        product.shutdown();
        product.shutdown();
        assert_eq!(product.drain_product_events(), ShellEffect::NONE);
        assert_eq!(call_count.get(), 1);
    }
}
