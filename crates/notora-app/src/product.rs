use std::fmt;
use std::sync::{Arc, Mutex, mpsc};

use appkit_shell::{ProductHost, ProductWakeHandle, ShellEffect};
use notora_core::note_command::NoteCommandResult;
use notora_core::{ScanCompletion, WorkspaceId};

use crate::action::DocumentLoadRequest;

/// 后台服务只能经 notora 自有 channel 发送的 payload。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotoraProductEvent {
    CardQueryCompleted {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
    },
    WorkspaceScanCompleted {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        completion: ScanCompletion,
    },
    WorkspaceChanged {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        changed_paths: Vec<std::path::PathBuf>,
    },
    WorkspaceIndexFailed {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        message: String,
    },
    NoteCommandCompleted {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        result: NoteCommandResult,
    },
    NoteCommandFailed {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        message: String,
    },
    DocumentLoaded {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        request: DocumentLoadRequest,
        document: crate::editor_adapter::LoadedDocument,
    },
    DocumentLoadFailed {
        workspace_id: WorkspaceId,
        workspace_generation: u64,
        request: DocumentLoadRequest,
        message: String,
    },
}

#[derive(Clone)]
pub struct NotoraProductEventSender {
    sender: mpsc::Sender<NotoraProductEvent>,
    wake_handle: Arc<Mutex<Option<ProductWakeHandle>>>,
}

impl NotoraProductEventSender {
    pub fn send(&self, event: NotoraProductEvent) -> Result<(), ProductEventSendError> {
        self.sender.send(event).map_err(|_| ProductEventSendError)?;
        if let Some(wake_handle) =
            self.wake_handle.lock().map_err(|_| ProductEventSendError)?.as_ref()
        {
            wake_handle.wake().map_err(|_| ProductEventSendError)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductEventSendError;

impl fmt::Display for ProductEventSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("notora product event receiver is unavailable")
    }
}

impl std::error::Error for ProductEventSendError {}

/// 由产品持有、并在退出时有序停止的后台服务。
pub trait ProductServiceShutdown {
    fn shutdown(&mut self);
}

/// 产品服务宿主。shell 只看到唤醒和聚合后的 ShellEffect。
pub struct NotoraProduct {
    event_sender: NotoraProductEventSender,
    event_receiver: mpsc::Receiver<NotoraProductEvent>,
    wake_handle: Arc<Mutex<Option<ProductWakeHandle>>>,
    active_workspace: Option<(WorkspaceId, u64)>,
    workspace_events: Vec<NotoraProductEvent>,
    service_shutdown_handles: Vec<Box<dyn ProductServiceShutdown>>,
    services_started: bool,
    shutdown: bool,
}

impl NotoraProduct {
    pub fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::channel();
        let wake_handle = Arc::new(Mutex::new(None));
        Self {
            event_sender: NotoraProductEventSender {
                sender: event_sender,
                wake_handle: Arc::clone(&wake_handle),
            },
            event_receiver,
            wake_handle,
            active_workspace: None,
            workspace_events: Vec::new(),
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

    /// 取出当前活动工作区产生的后台更新；旧 generation 已在 drain 时丢弃。
    pub fn take_workspace_events(&mut self) -> Vec<NotoraProductEvent> {
        std::mem::take(&mut self.workspace_events)
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
            NotoraProductEvent::CardQueryCompleted { workspace_id, workspace_generation }
            | NotoraProductEvent::WorkspaceScanCompleted {
                workspace_id,
                workspace_generation,
                ..
            }
            | NotoraProductEvent::WorkspaceChanged { workspace_id, workspace_generation, .. }
            | NotoraProductEvent::WorkspaceIndexFailed {
                workspace_id, workspace_generation, ..
            }
            | NotoraProductEvent::NoteCommandCompleted {
                workspace_id, workspace_generation, ..
            }
            | NotoraProductEvent::NoteCommandFailed {
                workspace_id, workspace_generation, ..
            }
            | NotoraProductEvent::DocumentLoaded { workspace_id, workspace_generation, .. }
            | NotoraProductEvent::DocumentLoadFailed {
                workspace_id, workspace_generation, ..
            } => (*workspace_id, *workspace_generation),
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
        if let Ok(mut stored_wake_handle) = self.wake_handle.lock() {
            *stored_wake_handle = Some(wake);
            self.services_started = true;
        }
    }

    fn drain_product_events(&mut self) -> ShellEffect {
        let mut effect = ShellEffect::NONE;
        while let Ok(event) = self.event_receiver.try_recv() {
            if self.event_matches_active_workspace(&event) {
                self.workspace_events.push(event);
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
        if let Ok(mut stored_wake_handle) = self.wake_handle.lock() {
            *stored_wake_handle = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use appkit_shell::{ProductHost, ShellEffect};

    use notora_core::WorkspaceId;

    use super::{NotoraProduct, NotoraProductEvent, ProductServiceShutdown};
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
        let sender = product.event_sender();
        sender
            .send(NotoraProductEvent::CardQueryCompleted {
                workspace_id: active_workspace_id,
                workspace_generation: 3,
            })
            .expect("product receiver should be alive");
        sender
            .send(NotoraProductEvent::WorkspaceChanged {
                workspace_id: active_workspace_id,
                workspace_generation: 4,
                changed_paths: vec![],
            })
            .expect("product receiver should be alive");

        assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
        assert_eq!(product.drain_product_events(), ShellEffect::NONE);
    }

    #[test]
    fn drain_discards_events_from_another_workspace_with_the_same_generation() {
        let mut product = NotoraProduct::new();
        product.set_active_workspace(WorkspaceId::generate(), 4);
        let sender = product.event_sender();
        sender
            .send(NotoraProductEvent::WorkspaceChanged {
                workspace_id: WorkspaceId::generate(),
                workspace_generation: 4,
                changed_paths: vec![],
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
        product
            .event_sender()
            .send(NotoraProductEvent::DocumentLoadFailed {
                workspace_id,
                workspace_generation: 9,
                request: DocumentLoadRequest { identity, selection_generation: 2 },
                message: "fixture read failed".to_owned(),
            })
            .expect("product receiver should be alive");

        assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
        assert!(matches!(
            product.take_workspace_events().as_slice(),
            [NotoraProductEvent::DocumentLoadFailed {
                workspace_id: event_workspace_id,
                workspace_generation: 9,
                request: DocumentLoadRequest { identity: event_identity, selection_generation: 2 },
                ..
            }] if *event_workspace_id == workspace_id && *event_identity == identity
        ));
    }

    #[test]
    fn sender_reports_a_disconnected_receiver() {
        let sender = NotoraProduct::new().event_sender();
        assert!(
            sender
                .send(NotoraProductEvent::WorkspaceChanged {
                    workspace_id: WorkspaceId::generate(),
                    workspace_generation: 1,
                    changed_paths: vec![],
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
