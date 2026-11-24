//! 工作区专属的后台索引 worker 生命周期与命令边界。

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use notora_core::WorkspaceFileMonitorEvent;
use notora_core::note_command::NoteCommand;

use crate::action::{CardQuery, DocumentLoadRequest, MetadataMutation, TrashOperation};

const INDEX_WORKER_THREAD_NAME: &str = "notora-workspace-indexer";

/// 只能由后台 catalog owner 执行的索引相关命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceDocumentSource {
    ActiveNote,
    TrashedNote,
}

pub(crate) enum IndexWorkerCommand {
    QueryCards(CardQuery),
    QueryNavigationTree,
    ExecuteNoteCommand(NoteCommand),
    ExecuteDirectoryCommand(notora_core::WorkspaceDirectoryCommand),
    ExecuteMetadataMutation(MetadataMutation),
    CreateCatalogBackup {
        directory: PathBuf,
        retention: notora_core::BackupRetention,
    },
    ExecuteTrashOperation(TrashOperation),
    PrepareDocument {
        request: DocumentLoadRequest,
        source: WorkspaceDocumentSource,
        cached_session: Option<std::sync::Arc<textora_encryption::UnlockedNoteSession>>,
    },
    UnlockEncryptedDocument {
        request: DocumentLoadRequest,
        password: std::sync::Arc<textora_encryption::EncryptionPassword>,
        generation: u64,
        source: WorkspaceDocumentSource,
    },
    ReindexCatalog {
        relative_path: Option<PathBuf>,
    },
}

pub(crate) enum IndexWorkerEvent {
    Command(IndexWorkerCommand),
    Monitor(WorkspaceFileMonitorEvent),
    MonitorDisconnected,
    Shutdown,
}

pub(crate) struct IndexWorkerMonitorSender {
    events: mpsc::Sender<IndexWorkerEvent>,
    reported_failure: bool,
}

impl IndexWorkerMonitorSender {
    pub(crate) fn new(events: mpsc::Sender<IndexWorkerEvent>) -> Self {
        Self { events, reported_failure: false }
    }

    pub(crate) fn send(
        &mut self,
        event: WorkspaceFileMonitorEvent,
    ) -> Result<(), mpsc::SendError<IndexWorkerEvent>> {
        self.reported_failure |= matches!(event, WorkspaceFileMonitorEvent::Failed(_));
        self.events.send(IndexWorkerEvent::Monitor(event))
    }
}

impl Drop for IndexWorkerMonitorSender {
    fn drop(&mut self) {
        if !self.reported_failure {
            let _ = self.events.send(IndexWorkerEvent::MonitorDisconnected);
        }
    }
}

/// 索引线程接收工作区命令、监视事件和独立的关停指令。
///
/// 具体线程闭包拥有 `Catalog` connection；主线程只保留类型化 sender，因而不能在
/// render 或 reducer 路径意外执行 SQLite 查询。
pub(crate) struct IndexWorker {
    event_sender: mpsc::Sender<IndexWorkerEvent>,
    join_handle: Option<JoinHandle<()>>,
}

impl IndexWorker {
    pub(crate) fn start(
        event_sender: mpsc::Sender<IndexWorkerEvent>,
        run: impl FnOnce() + Send + 'static,
    ) -> Result<Self, std::io::Error> {
        let join_handle =
            thread::Builder::new().name(INDEX_WORKER_THREAD_NAME.to_owned()).spawn(run)?;
        Ok(Self { event_sender, join_handle: Some(join_handle) })
    }

    pub(crate) fn send(
        &self,
        command: IndexWorkerCommand,
    ) -> Result<(), mpsc::SendError<IndexWorkerEvent>> {
        self.event_sender.send(IndexWorkerEvent::Command(command))
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(join_handle) = self.join_handle.take() {
            let _ = self.event_sender.send(IndexWorkerEvent::Shutdown);
            let _ = join_handle.join();
        }
    }
}

impl Drop for IndexWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{IndexWorker, IndexWorkerEvent, IndexWorkerMonitorSender};

    #[test]
    fn shutdown_joins_the_worker_while_the_monitor_can_still_send() {
        let (stopped_sender, stopped_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let mut monitor = IndexWorkerMonitorSender::new(event_sender.clone());
        let mut worker = IndexWorker::start(event_sender, move || {
            assert!(matches!(event_receiver.recv(), Ok(IndexWorkerEvent::Shutdown)));
            let _ = stopped_sender.send(());
        })
        .expect("index worker should start");

        worker.shutdown();

        stopped_receiver.recv().expect("worker should process shutdown before joining");
        assert!(monitor.send(notora_core::WorkspaceFileMonitorEvent::Ready).is_err());
    }
}
