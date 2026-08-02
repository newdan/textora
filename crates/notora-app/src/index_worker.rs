//! 工作区专属的后台索引 worker 生命周期与命令边界。

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use notora_core::note_command::NoteCommand;

use crate::action::{CardQuery, DocumentLoadRequest, MetadataMutation, TrashOperation};

const INDEX_WORKER_THREAD_NAME: &str = "notora-workspace-indexer";

/// 只能由后台 catalog owner 执行的索引相关命令。
pub(crate) enum IndexWorkerCommand {
    QueryCards(CardQuery),
    QueryNavigationTree,
    ExecuteNoteCommand(NoteCommand),
    ExecuteMetadataMutation(MetadataMutation),
    CreateCatalogBackup { directory: PathBuf, retention: notora_core::BackupRetention },
    ExecuteTrashOperation(TrashOperation),
    PrepareDocument(DocumentLoadRequest),
    ReindexCatalog,
}

/// 通过断开 command sender 自然退出的后台 worker。
///
/// 具体线程闭包拥有 `Catalog` connection；主线程只保留类型化 sender，因而不能在
/// render 或 reducer 路径意外执行 SQLite 查询。
pub(crate) struct IndexWorker {
    command_sender: Option<mpsc::Sender<IndexWorkerCommand>>,
    join_handle: Option<JoinHandle<()>>,
}

impl IndexWorker {
    pub(crate) fn start(
        run: impl FnOnce(mpsc::Receiver<IndexWorkerCommand>) + Send + 'static,
    ) -> Result<Self, std::io::Error> {
        let (command_sender, command_receiver) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name(INDEX_WORKER_THREAD_NAME.to_owned())
            .spawn(move || run(command_receiver))?;
        Ok(Self { command_sender: Some(command_sender), join_handle: Some(join_handle) })
    }

    pub(crate) fn send(
        &self,
        command: IndexWorkerCommand,
    ) -> Result<(), mpsc::SendError<IndexWorkerCommand>> {
        let Some(command_sender) = &self.command_sender else {
            return Err(mpsc::SendError(command));
        };
        command_sender.send(command)
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.command_sender.take();
        if let Some(join_handle) = self.join_handle.take() {
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

    use super::IndexWorker;

    #[test]
    fn shutdown_disconnects_the_command_channel_and_joins_the_worker() {
        let (stopped_sender, stopped_receiver) = mpsc::channel();
        let mut worker = IndexWorker::start(move |command_receiver| {
            while command_receiver.recv().is_ok() {}
            let _ = stopped_sender.send(());
        })
        .expect("index worker should start");

        worker.shutdown();

        stopped_receiver.recv().expect("worker should observe sender disconnection before joining");
    }
}
