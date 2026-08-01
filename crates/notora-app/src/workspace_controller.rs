//! 工作区选择及后台扫描服务协调。

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notora_core::note_command::NoteCommand;
use notora_core::{
    Catalog, CatalogError, DocumentIdentity, Workspace, WorkspaceDescriptor, WorkspaceError,
    WorkspaceFileBatch, WorkspaceFileMonitor, WorkspaceFileMonitorError, execute_note_command,
    scan_workspace,
};

use crate::action::{CardQuery, DocumentLoadRequest};
use crate::index_worker::{IndexWorker, IndexWorkerCommand};
use crate::product::{NotoraProduct, NotoraProductEvent, NotoraProductEventSender};

const CATALOG_FILE_NAME: &str = "catalog.sqlite3";
const WORKSPACE_WORKER_IDLE_WAIT: Duration = Duration::from_millis(25);

/// 用户或恢复流程发起的工作区操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommand {
    /// 用户取消目录选择；不改变当前工作区。
    SelectionCancelled,
    /// 打开一个已经存在的目录。
    OpenExisting { root: PathBuf },
    /// 创建目录后将其作为工作区打开。
    Create { root: PathBuf },
    /// 关闭当前工作区，并使全部在途后台结果失效。
    Close,
}

/// 当前活动工作区的公开快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWorkspace {
    pub descriptor: WorkspaceDescriptor,
    pub generation: u64,
}

/// 工作区命令的执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommandResult {
    Unchanged,
    Opened(ActiveWorkspace),
    Closed { generation: u64 },
}

/// 工作区初始化或后台服务启动失败。
#[derive(Debug)]
pub enum WorkspaceControllerError {
    CreateDirectory { path: PathBuf, source: std::io::Error },
    Workspace(WorkspaceError),
    Catalog(CatalogError),
    FileMonitor(WorkspaceFileMonitorError),
    IndexerThreadUnavailable,
    NoActiveWorkspace,
    CommandWorkerDisconnected,
}

impl std::fmt::Display for WorkspaceControllerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateDirectory { path, source } => {
                write!(
                    formatter,
                    "could not create workspace directory {}: {source}",
                    path.display()
                )
            }
            Self::Workspace(source) => write!(formatter, "could not open workspace: {source}"),
            Self::Catalog(source) => {
                write!(formatter, "could not open workspace catalog: {source}")
            }
            Self::FileMonitor(source) => {
                write!(formatter, "could not watch workspace files: {source}")
            }
            Self::IndexerThreadUnavailable => {
                formatter.write_str("could not start the workspace indexing worker")
            }
            Self::NoActiveWorkspace => formatter.write_str("no workspace is active"),
            Self::CommandWorkerDisconnected => {
                formatter.write_str("workspace command worker is unavailable")
            }
        }
    }
}

impl std::error::Error for WorkspaceControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDirectory { source, .. } => Some(source),
            Self::Workspace(source) => Some(source),
            Self::Catalog(source) => Some(source),
            Self::FileMonitor(source) => Some(source),
            Self::IndexerThreadUnavailable
            | Self::NoActiveWorkspace
            | Self::CommandWorkerDisconnected => None,
        }
    }
}

/// 唯一持有活动工作区 watcher 与索引 worker 的产品服务。
#[derive(Default)]
pub struct WorkspaceController {
    next_generation: u64,
    active_session: Option<WorkspaceSession>,
}

impl WorkspaceController {
    pub fn execute(
        &mut self,
        command: WorkspaceCommand,
        product: &mut NotoraProduct,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
        match command {
            WorkspaceCommand::SelectionCancelled => Ok(WorkspaceCommandResult::Unchanged),
            WorkspaceCommand::OpenExisting { root } => self.open_existing(root, product),
            WorkspaceCommand::Create { root } => {
                fs::create_dir_all(&root).map_err(|source| {
                    WorkspaceControllerError::CreateDirectory { path: root.clone(), source }
                })?;
                self.open_existing(root, product)
            }
            WorkspaceCommand::Close => Ok(self.close(product)),
        }
    }

    pub fn active_workspace(&self) -> Option<ActiveWorkspace> {
        self.active_session.as_ref().map(WorkspaceSession::active_workspace)
    }

    /// 将文件命令交由活动工作区的后台 worker 执行。
    pub fn execute_note_command(
        &self,
        command: NoteCommand,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::ExecuteNoteCommand(command))
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 将已选择文档的磁盘读取交由活动工作区后台 worker。
    pub fn prepare_document(
        &self,
        request: DocumentLoadRequest,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::PrepareDocument(request))
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    pub fn query_cards(&self, query: CardQuery) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::QueryCards(query))
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 保存成功后的派生 catalog 字段由既有 worker 重建，绝不在主线程读取正文或执行 SQL。
    pub fn request_catalog_reindex(&self) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::ReindexCatalog)
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    fn open_existing(
        &mut self,
        root: PathBuf,
        product: &mut NotoraProduct,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
        let workspace =
            Workspace::open_or_initialize(&root).map_err(WorkspaceControllerError::Workspace)?;
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        Catalog::open(&catalog_path).map_err(WorkspaceControllerError::Catalog)?;

        let generation = self.advance_generation();
        let descriptor = workspace.descriptor();
        let event_sender = product.event_sender();
        let session = WorkspaceSession::start(workspace, catalog_path, generation, event_sender)?;
        self.close_active_session();
        product.set_active_workspace(descriptor.workspace_id, generation);
        self.active_session = Some(session);
        let active_workspace = self
            .active_workspace()
            .expect("an installed workspace session must expose an active workspace");
        Ok(WorkspaceCommandResult::Opened(active_workspace))
    }

    fn close(&mut self, product: &mut NotoraProduct) -> WorkspaceCommandResult {
        self.close_active_session();
        let generation = self.advance_generation();
        product.clear_active_workspace();
        WorkspaceCommandResult::Closed { generation }
    }

    fn close_active_session(&mut self) {
        if let Some(mut session) = self.active_session.take() {
            session.shutdown();
        }
    }

    fn advance_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }
}

impl Drop for WorkspaceController {
    fn drop(&mut self) {
        self.close_active_session();
    }
}

struct WorkspaceSession {
    active_workspace: ActiveWorkspace,
    file_monitor: WorkspaceFileMonitor,
    indexer: IndexWorker,
}

impl WorkspaceSession {
    fn start(
        workspace: Workspace,
        catalog_path: PathBuf,
        generation: u64,
        event_sender: NotoraProductEventSender,
    ) -> Result<Self, WorkspaceControllerError> {
        let descriptor = workspace.descriptor();
        let indexer_descriptor = descriptor.clone();
        let (file_monitor, file_batches) =
            WorkspaceFileMonitor::start(workspace.root().to_path_buf())
                .map_err(WorkspaceControllerError::FileMonitor)?;
        let indexer = IndexWorker::start(move |command_receiver| {
            run_indexer(
                workspace,
                catalog_path,
                indexer_descriptor,
                generation,
                file_batches,
                command_receiver,
                event_sender,
            )
        })
        .map_err(|_| WorkspaceControllerError::IndexerThreadUnavailable)?;
        Ok(Self {
            active_workspace: ActiveWorkspace { descriptor, generation },
            file_monitor,
            indexer,
        })
    }

    fn active_workspace(&self) -> ActiveWorkspace {
        self.active_workspace.clone()
    }

    fn shutdown(&mut self) {
        self.file_monitor.shutdown();
        self.indexer.shutdown();
    }
}

fn run_indexer(
    workspace: Workspace,
    catalog_path: PathBuf,
    descriptor: WorkspaceDescriptor,
    generation: u64,
    file_batches: mpsc::Receiver<WorkspaceFileBatch>,
    command_receiver: mpsc::Receiver<IndexWorkerCommand>,
    event_sender: NotoraProductEventSender,
) {
    let Ok(catalog) = Catalog::open(&catalog_path) else {
        let _ = event_sender.send(NotoraProductEvent::WorkspaceIndexFailed {
            workspace_id: descriptor.workspace_id,
            workspace_generation: generation,
            message: "workspace catalog is unavailable to the indexing worker".to_owned(),
        });
        return;
    };
    index_workspace(&workspace, &catalog, descriptor.workspace_id, generation, &event_sender);
    loop {
        while let Ok(command) = command_receiver.try_recv() {
            execute_workspace_command(
                &workspace,
                &catalog,
                descriptor.workspace_id,
                generation,
                command,
                &event_sender,
            );
        }
        match file_batches.recv_timeout(WORKSPACE_WORKER_IDLE_WAIT) {
            Ok(batch) => {
                index_workspace(
                    &workspace,
                    &catalog,
                    descriptor.workspace_id,
                    generation,
                    &event_sender,
                );
                let _ = event_sender.send(NotoraProductEvent::WorkspaceChanged {
                    workspace_id: descriptor.workspace_id,
                    workspace_generation: generation,
                    changed_paths: batch.relative_paths,
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn execute_workspace_command(
    workspace: &Workspace,
    catalog: &Catalog,
    workspace_id: notora_core::WorkspaceId,
    workspace_generation: u64,
    command: IndexWorkerCommand,
    event_sender: &NotoraProductEventSender,
) {
    match command {
        IndexWorkerCommand::QueryCards(query) => {
            match catalog.query_catalog_cards(&query.scope, query.cursor.as_ref(), query.page_size)
            {
                Ok(page) => {
                    let _ = event_sender.send(NotoraProductEvent::CardQueryCompleted {
                        workspace_id,
                        workspace_generation,
                        query,
                        page,
                    });
                }
                Err(error) => {
                    let _ = event_sender.send(NotoraProductEvent::CardQueryFailed {
                        workspace_id,
                        workspace_generation,
                        query,
                        message: error.to_string(),
                    });
                }
            }
        }
        IndexWorkerCommand::ExecuteNoteCommand(command) => {
            execute_note_command_in_worker(
                workspace,
                catalog,
                workspace_id,
                workspace_generation,
                command,
                event_sender,
            );
        }
        IndexWorkerCommand::PrepareDocument(request) => {
            prepare_document_in_worker(
                workspace,
                catalog,
                workspace_id,
                workspace_generation,
                request,
                event_sender,
            );
        }
        IndexWorkerCommand::ReindexCatalog => {
            index_workspace(workspace, catalog, workspace_id, workspace_generation, event_sender);
        }
    }
}

fn execute_note_command_in_worker(
    workspace: &Workspace,
    catalog: &Catalog,
    workspace_id: notora_core::WorkspaceId,
    workspace_generation: u64,
    command: NoteCommand,
    event_sender: &NotoraProductEventSender,
) {
    match execute_note_command(workspace, catalog, command) {
        Ok(result) => {
            let _ = event_sender.send(NotoraProductEvent::NoteCommandCompleted {
                workspace_id,
                workspace_generation,
                result,
            });
            index_workspace(workspace, catalog, workspace_id, workspace_generation, event_sender);
        }
        Err(error) => {
            let _ = event_sender.send(NotoraProductEvent::NoteCommandFailed {
                workspace_id,
                workspace_generation,
                message: error.to_string(),
            });
        }
    }
}

fn prepare_document_in_worker(
    workspace: &Workspace,
    catalog: &Catalog,
    workspace_id: notora_core::WorkspaceId,
    workspace_generation: u64,
    request: DocumentLoadRequest,
    event_sender: &NotoraProductEventSender,
) {
    let result = match request.identity {
        DocumentIdentity::Note(note_id) => catalog
            .active_note(note_id)
            .map_err(|error| error.to_string())
            .and_then(|note| note.ok_or_else(|| format!("active note {note_id} no longer exists")))
            .and_then(|note| {
                workspace
                    .resolve_relative_path(&note.relative_path)
                    .map_err(|error| error.to_string())
            })
            .and_then(|path| {
                crate::editor_adapter::load_document(&path).map_err(|error| error.to_string())
            }),
        DocumentIdentity::ExternalFile(_) => {
            Err("external document preparation is not available before N3-8".to_owned())
        }
    };
    match result {
        Ok(document) => {
            let _ = event_sender.send(NotoraProductEvent::DocumentLoaded {
                workspace_id,
                workspace_generation,
                request,
                document,
            });
        }
        Err(message) => {
            let _ = event_sender.send(NotoraProductEvent::DocumentLoadFailed {
                workspace_id,
                workspace_generation,
                request,
                message,
            });
        }
    }
}

fn index_workspace(
    workspace: &Workspace,
    catalog: &Catalog,
    workspace_id: notora_core::WorkspaceId,
    workspace_generation: u64,
    event_sender: &NotoraProductEventSender,
) {
    match scan_workspace(workspace, catalog) {
        Ok(completion) => {
            let _ = event_sender.send(NotoraProductEvent::WorkspaceScanCompleted {
                workspace_id,
                workspace_generation,
                completion,
            });
        }
        Err(error) => {
            let _ = event_sender.send(NotoraProductEvent::WorkspaceIndexFailed {
                workspace_id,
                workspace_generation,
                message: error.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    use appkit_shell::ProductHost;

    use super::{
        WorkspaceCommand, WorkspaceCommandResult, WorkspaceController, WorkspaceControllerError,
    };
    use crate::action::CardQuery;
    use crate::product::{NotoraProduct, NotoraProductEvent};

    #[test]
    fn cancelled_selection_keeps_the_current_workspace_unchanged() {
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();

        assert_eq!(
            controller
                .execute(WorkspaceCommand::SelectionCancelled, &mut product)
                .expect("cancelling a selection should be harmless"),
            WorkspaceCommandResult::Unchanged
        );
        assert_eq!(controller.active_workspace(), None);
    }

    #[test]
    fn creates_directory_initializes_workspace_and_advances_generation_on_close() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let root = directory.path().join("new-workspace");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();

        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(WorkspaceCommand::Create { root: root.clone() }, &mut product)
            .expect("new workspace should open")
        else {
            panic!("create command should open a workspace");
        };
        assert_eq!(active_workspace.generation, 1);
        assert!(root.join(".notora/workspace.toml").is_file());

        assert_eq!(
            controller
                .execute(WorkspaceCommand::Close, &mut product)
                .expect("close should be safe"),
            WorkspaceCommandResult::Closed { generation: 2 }
        );
        assert_eq!(controller.active_workspace(), None);
    }

    #[test]
    fn corrupt_manifest_does_not_replace_the_active_workspace() {
        let active_directory =
            tempfile::tempdir().expect("active workspace directory should exist");
        let corrupt_directory =
            tempfile::tempdir().expect("corrupt workspace directory should exist");
        fs::create_dir(corrupt_directory.path().join(".notora"))
            .expect("metadata directory fixture should be created");
        fs::write(corrupt_directory.path().join(".notora/workspace.toml"), "not toml")
            .expect("corrupt manifest fixture should be written");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: active_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("active workspace should open")
        else {
            panic!("open command should activate the workspace");
        };

        assert!(matches!(
            controller.execute(
                WorkspaceCommand::OpenExisting { root: corrupt_directory.path().to_path_buf() },
                &mut product,
            ),
            Err(WorkspaceControllerError::Workspace(_))
        ));
        assert_eq!(controller.active_workspace(), Some(active_workspace));
    }

    #[test]
    fn product_discards_late_scan_results_after_a_workspace_switch() {
        let first_directory = tempfile::tempdir().expect("first workspace directory should exist");
        let second_directory =
            tempfile::tempdir().expect("second workspace directory should exist");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(first_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: first_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("first workspace should open")
        else {
            panic!("first workspace should activate");
        };
        let sender = product.event_sender();
        let WorkspaceCommandResult::Opened(second_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: second_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("second workspace should open")
        else {
            panic!("second workspace should activate");
        };
        sender
            .send(NotoraProductEvent::WorkspaceChanged {
                workspace_id: first_workspace.descriptor.workspace_id,
                workspace_generation: first_workspace.generation,
                changed_paths: vec!["late.md".into()],
            })
            .expect("product receiver should stay available");
        sender
            .send(NotoraProductEvent::WorkspaceChanged {
                workspace_id: second_workspace.descriptor.workspace_id,
                workspace_generation: second_workspace.generation,
                changed_paths: vec!["current.md".into()],
            })
            .expect("product receiver should stay available");

        let _ = product.drain_product_events();
        let events = product.take_workspace_events();
        assert!(events.iter().any(|event| matches!(
            event,
            NotoraProductEvent::WorkspaceChanged { workspace_id, workspace_generation, .. }
                if *workspace_id == second_workspace.descriptor.workspace_id
                    && *workspace_generation == second_workspace.generation
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            NotoraProductEvent::WorkspaceChanged { workspace_id, workspace_generation, .. }
                if *workspace_id == first_workspace.descriptor.workspace_id
                    && *workspace_generation == first_workspace.generation
        )));
    }

    #[test]
    fn active_workspace_worker_executes_note_commands_and_returns_a_product_event() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
                &mut product,
            )
            .expect("workspace should open")
        else {
            panic!("open command should activate the workspace");
        };

        controller
            .execute_note_command(notora_core::note_command::NoteCommand::Create(
                notora_core::note_command::CreateNoteRequest {
                    kind: notora_core::DocumentKind::Markdown,
                    target_directory: None,
                    tag_to_attach: None,
                },
            ))
            .expect("active workspace should accept the note command");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_workspace_events();
            if events.iter().any(|event| {
                matches!(
                    event,
                    NotoraProductEvent::NoteCommandCompleted {
                        workspace_id,
                        workspace_generation,
                        result,
                    } if *workspace_id == active_workspace.descriptor.workspace_id
                        && *workspace_generation == active_workspace.generation
                        && result.note.relative_path == std::path::Path::new("未命名 1.md")
                )
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "note command completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn active_workspace_worker_returns_card_query_completion_with_its_generation() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
                &mut product,
            )
            .expect("workspace should open")
        else {
            panic!("open command should activate the workspace");
        };
        let query = CardQuery::from(notora_core::NavigationScope::WorkspaceRoot);
        controller.query_cards(query.clone()).expect("worker should accept card query");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_workspace_events();
            if events.iter().any(|event| {
                matches!(
                    event,
                    NotoraProductEvent::CardQueryCompleted {
                        workspace_id,
                        workspace_generation,
                        query: completed_query,
                        page,
                    } if *workspace_id == active_workspace.descriptor.workspace_id
                        && *workspace_generation == active_workspace.generation
                        && completed_query == &query
                        && page.cards.is_empty()
                )
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "card query completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
