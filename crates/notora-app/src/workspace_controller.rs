//! 工作区选择及后台扫描服务协调。

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use notora_core::note_command::NoteCommand;
use notora_core::{
    Catalog, CatalogNavigationTree, DocumentIdentity, Workspace, WorkspaceDescriptor,
    WorkspaceError, WorkspaceFileBatch, WorkspaceFileMonitor, WorkspaceFileMonitorError,
    WorkspaceFileMonitorEvent, execute_note_command, scan_workspace, scan_workspace_directories,
    scan_workspace_file_batch, scan_workspace_paths,
};

use crate::action::{CardQuery, DocumentLoadRequest, MetadataMutation, TrashOperation};
use crate::index_worker::{IndexWorker, IndexWorkerCommand, WorkspaceDocumentSource};
use crate::product::{
    NotoraProduct, NotoraProductEvent, NotoraProductEventSender, WorkspaceBootstrapCompletion,
    WorkspaceCompletion, WorkspaceEventScope, WorkspaceEventSender, WorkspaceNoteRelocation,
};

const CATALOG_FILE_NAME: &str = "catalog.sqlite3";
const DEFAULT_MIGRATION_BACKUP_RETAINED_COUNT: usize = 8;
const WORKSPACE_WORKER_IDLE_WAIT: Duration = Duration::from_millis(25);
const WATCHER_PRESENCE_CONFIRMATION_DELAY: Duration = Duration::from_millis(300);

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
    FileMonitor(WorkspaceFileMonitorError),
    BootstrapThreadUnavailable,
    BootstrapWorkerDisconnected,
    IndexerThreadUnavailable,
    IndexerStartup { message: String },
    NoActiveWorkspace,
    CommandWorkerDisconnected,
}

impl std::fmt::Display for WorkspaceControllerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateDirectory { path, source } => {
                write!(formatter, "无法创建工作区目录 {}：{source}", path.display())
            }
            Self::Workspace(source) => write!(formatter, "无法打开工作区：{source}"),
            Self::FileMonitor(source) => {
                write!(formatter, "无法监视工作区文件：{source}")
            }
            Self::BootstrapThreadUnavailable => formatter.write_str("无法启动工作区准备线程"),
            Self::BootstrapWorkerDisconnected => formatter.write_str("工作区准备线程未返回结果"),
            Self::IndexerThreadUnavailable => formatter.write_str("无法启动工作区索引线程"),
            Self::IndexerStartup { message } => {
                write!(formatter, "工作区索引线程启动失败：{message}")
            }
            Self::NoActiveWorkspace => formatter.write_str("当前没有活动工作区"),
            Self::CommandWorkerDisconnected => formatter.write_str("工作区命令线程不可用"),
        }
    }
}

impl std::error::Error for WorkspaceControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDirectory { source, .. } => Some(source),
            Self::Workspace(source) => Some(source),
            Self::FileMonitor(source) => Some(source),
            Self::BootstrapThreadUnavailable
            | Self::BootstrapWorkerDisconnected
            | Self::IndexerThreadUnavailable
            | Self::IndexerStartup { .. }
            | Self::NoActiveWorkspace
            | Self::CommandWorkerDisconnected => None,
        }
    }
}

/// 唯一持有活动工作区 watcher 与索引 worker 的产品服务。
pub struct WorkspaceController {
    next_generation: u64,
    active_session: Option<WorkspaceSession>,
    pending_preparation: Option<PendingWorkspacePreparation>,
    catalog_backups_directory: Option<PathBuf>,
    migration_backup_retention: notora_core::BackupRetention,
}

struct PendingWorkspacePreparation {
    generation: u64,
    result_receiver: mpsc::Receiver<Result<WorkspaceSession, WorkspaceControllerError>>,
}

impl WorkspaceController {
    pub fn with_catalog_backups_directory(catalog_backups_directory: PathBuf) -> Self {
        Self::with_catalog_backups_directory_and_retention(
            catalog_backups_directory,
            default_migration_backup_retention(),
        )
    }

    pub fn with_catalog_backups_directory_and_retention(
        catalog_backups_directory: PathBuf,
        migration_backup_retention: notora_core::BackupRetention,
    ) -> Self {
        Self {
            next_generation: 0,
            active_session: None,
            pending_preparation: None,
            catalog_backups_directory: Some(catalog_backups_directory),
            migration_backup_retention,
        }
    }

    pub fn execute(
        &mut self,
        command: WorkspaceCommand,
        product: &mut NotoraProduct,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
        if !matches!(command, WorkspaceCommand::SelectionCancelled) {
            self.pending_preparation = None;
        }
        match command {
            WorkspaceCommand::SelectionCancelled => Ok(WorkspaceCommandResult::Unchanged),
            WorkspaceCommand::OpenExisting { root } => self.open_existing(root, product),
            WorkspaceCommand::Create { root } => {
                fs::create_dir(&root).map_err(|source| {
                    WorkspaceControllerError::CreateDirectory { path: root.clone(), source }
                })?;
                self.open_existing(root, product)
            }
            WorkspaceCommand::Close => Ok(self.close(product)),
        }
    }

    /// 在后台准备恢复用工作区；调用线程只创建 worker，不等待 watcher 或 catalog。
    pub fn begin_open_existing(
        &mut self,
        root: PathBuf,
        product: &NotoraProduct,
    ) -> Result<u64, WorkspaceControllerError> {
        self.pending_preparation = None;
        let generation = self.advance_generation();
        let catalog_backups_directory = self.catalog_backups_directory.clone();
        let migration_backup_retention = self.migration_backup_retention;
        let event_sender = product.event_sender();
        let completion_sender = event_sender.clone();
        let (result_sender, result_receiver) = mpsc::channel();
        thread::Builder::new()
            .name("notora-workspace-bootstrap".to_owned())
            .spawn(move || {
                let result = prepare_workspace_session(
                    root,
                    catalog_backups_directory,
                    migration_backup_retention,
                    generation,
                    event_sender,
                );
                let _ = result_sender.send(result);
                let _ = completion_sender.send(NotoraProductEvent::WorkspaceBootstrap(
                    WorkspaceBootstrapCompletion { generation },
                ));
            })
            .map_err(|_| WorkspaceControllerError::BootstrapThreadUnavailable)?;
        self.pending_preparation =
            Some(PendingWorkspacePreparation { generation, result_receiver });
        Ok(generation)
    }

    /// 安装已经准备完成且仍属于当前 generation 的工作区；永不等待 worker。
    pub fn complete_open_existing(
        &mut self,
        generation: u64,
        product: &mut NotoraProduct,
    ) -> Result<Option<WorkspaceCommandResult>, WorkspaceControllerError> {
        let Some(pending) = self.pending_preparation.take() else {
            return Ok(None);
        };
        if pending.generation != generation {
            self.pending_preparation = Some(pending);
            return Ok(None);
        }
        let session = match pending.result_receiver.try_recv() {
            Ok(result) => result?,
            Err(mpsc::TryRecvError::Empty) => {
                self.pending_preparation = Some(pending);
                return Ok(None);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(WorkspaceControllerError::BootstrapWorkerDisconnected);
            }
        };
        Ok(Some(self.install_session(session, product)))
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

    /// 将目录变更交由活动工作区的后台 worker 执行。
    pub fn execute_directory_command(
        &self,
        command: notora_core::WorkspaceDirectoryCommand,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::ExecuteDirectoryCommand(command))
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 将星标和标签变更交由活动工作区的唯一 catalog owner 执行。
    pub fn execute_metadata_mutation(
        &self,
        mutation: MetadataMutation,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::ExecuteMetadataMutation(mutation))
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 向唯一 catalog owner 提交异步一致性备份；调用线程绝不复制数据库文件。
    pub fn create_catalog_backup(
        &self,
        directory: PathBuf,
        retention: notora_core::BackupRetention,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::CreateCatalogBackup { directory, retention })
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 将回收站的文件系统与 catalog 事务交给工作区后台 worker。
    pub fn execute_trash_operation(
        &self,
        operation: TrashOperation,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::ExecuteTrashOperation(operation))
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 将已选择文档的磁盘读取交由活动工作区后台 worker。
    pub fn prepare_document(
        &self,
        request: DocumentLoadRequest,
        cached_session: Option<std::sync::Arc<textora_encryption::UnlockedNoteSession>>,
    ) -> Result<(), WorkspaceControllerError> {
        self.prepare_workspace_document(
            request,
            WorkspaceDocumentSource::ActiveNote,
            cached_session,
        )
    }

    pub fn prepare_trashed_document(
        &self,
        request: DocumentLoadRequest,
        cached_session: Option<std::sync::Arc<textora_encryption::UnlockedNoteSession>>,
    ) -> Result<(), WorkspaceControllerError> {
        self.prepare_workspace_document(
            request,
            WorkspaceDocumentSource::TrashedNote,
            cached_session,
        )
    }

    pub fn unlock_encrypted_document(
        &self,
        unlock_request: crate::action::EncryptedNoteUnlockRequest,
        trashed: bool,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        let source = if trashed {
            WorkspaceDocumentSource::TrashedNote
        } else {
            WorkspaceDocumentSource::ActiveNote
        };
        session
            .indexer
            .send(IndexWorkerCommand::UnlockEncryptedDocument {
                request: unlock_request.request,
                password: unlock_request.password,
                generation: unlock_request.generation,
                source,
            })
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    fn prepare_workspace_document(
        &self,
        request: DocumentLoadRequest,
        source: WorkspaceDocumentSource,
        cached_session: Option<std::sync::Arc<textora_encryption::UnlockedNoteSession>>,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::PrepareDocument { request, source, cached_session })
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

    /// 读取预计算的导航树；仅 worker 持有 catalog connection。
    pub fn query_navigation_tree(&self) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::QueryNavigationTree)
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    /// 保存成功后的派生 catalog 字段由既有 worker 重建，绝不在主线程读取正文或执行 SQL。
    pub fn request_catalog_reindex(
        &self,
        relative_path: Option<PathBuf>,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::ReindexCatalog { relative_path })
            .map_err(|_| WorkspaceControllerError::CommandWorkerDisconnected)
    }

    fn open_existing(
        &mut self,
        root: PathBuf,
        product: &mut NotoraProduct,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
        let generation = self.next_generation.wrapping_add(1);
        let session = prepare_workspace_session(
            root,
            self.catalog_backups_directory.clone(),
            self.migration_backup_retention,
            generation,
            product.event_sender(),
        )?;
        self.next_generation = generation;
        Ok(self.install_session(session, product))
    }

    fn install_session(
        &mut self,
        session: WorkspaceSession,
        product: &mut NotoraProduct,
    ) -> WorkspaceCommandResult {
        let active_workspace = session.active_workspace();
        self.close_active_session();
        product.set_active_workspace(
            active_workspace.descriptor.workspace_id,
            active_workspace.generation,
        );
        self.active_session = Some(session);
        WorkspaceCommandResult::Opened(active_workspace)
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

fn recovered_catalog_notice(backup_path: &std::path::Path) -> String {
    format!("目录索引已损坏，元数据已从 {} 恢复", backup_path.display())
}

fn rebuilt_catalog_notice(corrupt_path: &std::path::Path) -> String {
    format!("目录索引已损坏；原文件已保存在 {}，部分元数据可能已丢失", corrupt_path.display())
}

impl Default for WorkspaceController {
    fn default() -> Self {
        Self {
            next_generation: 0,
            active_session: None,
            pending_preparation: None,
            catalog_backups_directory: None,
            migration_backup_retention: default_migration_backup_retention(),
        }
    }
}

fn prepare_workspace_session(
    root: PathBuf,
    catalog_backups_directory: Option<PathBuf>,
    migration_backup_retention: notora_core::BackupRetention,
    generation: u64,
    event_sender: NotoraProductEventSender,
) -> Result<WorkspaceSession, WorkspaceControllerError> {
    let workspace =
        Workspace::open_or_initialize(&root).map_err(WorkspaceControllerError::Workspace)?;
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let descriptor = workspace.descriptor();
    let workspace_catalog_backups_directory = catalog_backups_directory
        .map(|directory| directory.join(descriptor.workspace_id.to_string()));
    WorkspaceSession::start(
        workspace,
        catalog_path,
        workspace_catalog_backups_directory,
        migration_backup_retention,
        generation,
        event_sender,
    )
}

fn default_migration_backup_retention() -> notora_core::BackupRetention {
    notora_core::BackupRetention::keep_latest(DEFAULT_MIGRATION_BACKUP_RETAINED_COUNT)
        .expect("default migration backup retention must be non-zero")
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
        catalog_backups_directory: Option<PathBuf>,
        migration_backup_retention: notora_core::BackupRetention,
        generation: u64,
        event_sender: NotoraProductEventSender,
    ) -> Result<Self, WorkspaceControllerError> {
        let descriptor = workspace.descriptor();
        let event_sender = WorkspaceEventSender::new(
            event_sender,
            WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation },
        );
        let (mut file_monitor, monitor_events) =
            WorkspaceFileMonitor::start(workspace.root().to_path_buf())
                .map_err(WorkspaceControllerError::FileMonitor)?;
        let (startup_sender, startup_receiver) = mpsc::channel();
        let mut indexer = IndexWorker::start(move |command_receiver| {
            run_indexer_with_startup(
                workspace,
                catalog_path,
                catalog_backups_directory,
                migration_backup_retention,
                monitor_events,
                command_receiver,
                event_sender,
                Some(startup_sender),
            )
        })
        .map_err(|_| WorkspaceControllerError::IndexerThreadUnavailable)?;
        match startup_receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                file_monitor.shutdown();
                indexer.shutdown();
                return Err(WorkspaceControllerError::IndexerStartup { message });
            }
            Err(_) => {
                file_monitor.shutdown();
                indexer.shutdown();
                return Err(WorkspaceControllerError::IndexerStartup {
                    message: "索引线程在完成启动前意外退出".to_owned(),
                });
            }
        }
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

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn run_indexer(
    workspace: Workspace,
    catalog_path: PathBuf,
    catalog_backups_directory: Option<PathBuf>,
    migration_backup_retention: notora_core::BackupRetention,
    monitor_events: mpsc::Receiver<WorkspaceFileMonitorEvent>,
    command_receiver: mpsc::Receiver<IndexWorkerCommand>,
    event_sender: WorkspaceEventSender,
) {
    run_indexer_with_startup(
        workspace,
        catalog_path,
        catalog_backups_directory,
        migration_backup_retention,
        monitor_events,
        command_receiver,
        event_sender,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn run_indexer_with_startup(
    workspace: Workspace,
    catalog_path: PathBuf,
    catalog_backups_directory: Option<PathBuf>,
    migration_backup_retention: notora_core::BackupRetention,
    monitor_events: mpsc::Receiver<WorkspaceFileMonitorEvent>,
    command_receiver: mpsc::Receiver<IndexWorkerCommand>,
    event_sender: WorkspaceEventSender,
    mut startup_sender: Option<mpsc::Sender<Result<(), String>>>,
) {
    if let Some(backup_directory) = &catalog_backups_directory
        && Catalog::migration_required(&catalog_path).unwrap_or(false)
        && let Err(error) = notora_core::create_catalog_backup_from_path(
            &catalog_path,
            backup_directory,
            migration_backup_retention,
        )
    {
        report_indexer_startup_failure(
            &event_sender,
            &mut startup_sender,
            format!("迁移前备份工作区目录索引失败：{error}"),
        );
        return;
    }
    let catalog_result = match catalog_backups_directory {
        Some(backup_directory) => Catalog::open_or_recover(&catalog_path, &backup_directory)
            .map(|outcome| match outcome {
                notora_core::CatalogOpenOutcome::Opened(catalog) => (catalog, None),
                notora_core::CatalogOpenOutcome::RecoveredFromBackup { catalog, backup_path } => {
                    (catalog, Some(recovered_catalog_notice(&backup_path)))
                }
                notora_core::CatalogOpenOutcome::RebuiltWithoutMetadata {
                    catalog,
                    corrupt_path,
                } => (catalog, Some(rebuilt_catalog_notice(&corrupt_path))),
            })
            .map_err(|error| error.to_string()),
        None => Catalog::open(&catalog_path)
            .map(|catalog| (catalog, None))
            .map_err(|error| error.to_string()),
    };
    let (catalog, recovery_notice) = match catalog_result {
        Ok(result) => result,
        Err(error) => {
            report_indexer_startup_failure(
                &event_sender,
                &mut startup_sender,
                format!("索引线程无法访问工作区目录索引：{error}"),
            );
            return;
        }
    };
    if let Some(message) = recovery_notice {
        let _ = event_sender.send(WorkspaceCompletion::CatalogRecoveryNotified { message });
    }
    match notora_core::recover_note_path_operations(&workspace, &catalog) {
        Ok(report) if report.committed_operations > 0 || report.rolled_back_operations > 0 => {
            let _ = event_sender.send(WorkspaceCompletion::CatalogRecoveryNotified {
                message: format!(
                    "已恢复未完成的文件改名：确认 {} 项，回滚 {} 项",
                    report.committed_operations, report.rolled_back_operations
                ),
            });
        }
        Ok(_) => {}
        Err(error) => {
            report_indexer_startup_failure(
                &event_sender,
                &mut startup_sender,
                format!("工作区存在未完成的文件改名，需要先恢复：{error}"),
            );
            return;
        }
    }
    if let Some(sender) = startup_sender.take() {
        let _ = sender.send(Ok(()));
    }
    index_workspace(&workspace, &catalog, &event_sender);
    run_indexer_event_loop(&workspace, &catalog, monitor_events, command_receiver, &event_sender);
}

fn run_indexer_event_loop(
    workspace: &Workspace,
    catalog: &Catalog,
    monitor_events: mpsc::Receiver<WorkspaceFileMonitorEvent>,
    command_receiver: mpsc::Receiver<IndexWorkerCommand>,
    event_sender: &WorkspaceEventSender,
) {
    let mut monitor_events = Some(monitor_events);
    let mut pending_presence_confirmation_paths = BTreeSet::new();
    let mut presence_confirmation_due_at = None;
    loop {
        if !drain_indexer_commands(workspace, catalog, &command_receiver, event_sender) {
            return;
        }
        let event = match &monitor_events {
            Some(receiver) => receiver.recv_timeout(WORKSPACE_WORKER_IDLE_WAIT),
            None => {
                thread::sleep(WORKSPACE_WORKER_IDLE_WAIT);
                Err(mpsc::RecvTimeoutError::Timeout)
            }
        };
        match event {
            Ok(WorkspaceFileMonitorEvent::Ready) => {
                index_workspace(workspace, catalog, event_sender);
                send_navigation_tree(workspace, catalog, event_sender);
            }
            Ok(WorkspaceFileMonitorEvent::Failed(error)) => {
                let _ = event_sender.send(WorkspaceCompletion::WorkspaceIndexFailed {
                    message: format!("工作区文件监视器启动或运行失败，自动同步已停止：{error}"),
                });
                monitor_events = None;
            }
            Ok(WorkspaceFileMonitorEvent::Changes(batch)) => {
                pending_presence_confirmation_paths.extend(batch.relative_paths.iter().cloned());
                presence_confirmation_due_at =
                    Some(Instant::now() + WATCHER_PRESENCE_CONFIRMATION_DELAY);
                let note_relocations =
                    index_workspace_file_batch(workspace, catalog, &batch, event_sender);
                send_navigation_tree(workspace, catalog, event_sender);
                let _ = event_sender.send(WorkspaceCompletion::WorkspaceChanged {
                    changed_paths: batch.relative_paths,
                    note_relocations,
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = event_sender.send(WorkspaceCompletion::WorkspaceIndexFailed {
                    message: "工作区文件监视器已断开，自动同步已停止".to_owned(),
                });
                monitor_events = None;
            }
        }
        if presence_confirmation_due_at.is_some_and(|due_at| due_at <= Instant::now()) {
            let relative_paths = std::mem::take(&mut pending_presence_confirmation_paths)
                .into_iter()
                .collect::<Vec<_>>();
            presence_confirmation_due_at = None;
            index_workspace_paths(workspace, catalog, &relative_paths, event_sender);
        }
    }
}

fn drain_indexer_commands(
    workspace: &Workspace,
    catalog: &Catalog,
    receiver: &mpsc::Receiver<IndexWorkerCommand>,
    event_sender: &WorkspaceEventSender,
) -> bool {
    loop {
        match receiver.try_recv() {
            Ok(command) => execute_workspace_command(workspace, catalog, command, event_sender),
            Err(mpsc::TryRecvError::Empty) => return true,
            Err(mpsc::TryRecvError::Disconnected) => return false,
        }
    }
}

fn report_indexer_startup_failure(
    event_sender: &WorkspaceEventSender,
    startup_sender: &mut Option<mpsc::Sender<Result<(), String>>>,
    message: String,
) {
    if let Some(sender) = startup_sender.take() {
        let _ = sender.send(Err(message.clone()));
    }
    let _ = event_sender.send(WorkspaceCompletion::WorkspaceIndexFailed { message });
}

fn execute_workspace_command(
    workspace: &Workspace,
    catalog: &Catalog,
    command: IndexWorkerCommand,
    event_sender: &WorkspaceEventSender,
) {
    match command {
        IndexWorkerCommand::QueryCards(query) => {
            match catalog.query_catalog_cards(&query.scope, query.cursor.as_ref(), query.page_size)
            {
                Ok(page) => {
                    let _ =
                        event_sender.send(WorkspaceCompletion::CardQueryCompleted { query, page });
                }
                Err(error) => {
                    let _ = event_sender.send(WorkspaceCompletion::CardQueryFailed {
                        query,
                        message: error.to_string(),
                    });
                }
            }
        }
        IndexWorkerCommand::QueryNavigationTree => {
            send_navigation_tree(workspace, catalog, event_sender);
        }
        IndexWorkerCommand::ExecuteNoteCommand(command) => {
            execute_note_command_in_worker(workspace, catalog, command, event_sender);
        }
        IndexWorkerCommand::ExecuteDirectoryCommand(command) => {
            execute_directory_command_in_worker(workspace, catalog, command, event_sender);
        }
        IndexWorkerCommand::ExecuteMetadataMutation(mutation) => {
            execute_metadata_mutation_in_worker(catalog, mutation, event_sender);
        }
        IndexWorkerCommand::CreateCatalogBackup { directory, retention } => {
            match notora_core::create_catalog_backup(catalog, &directory, retention) {
                Ok(backup_path) => {
                    let _ = event_sender
                        .send(WorkspaceCompletion::CatalogBackupCompleted { backup_path });
                }
                Err(error) => {
                    let _ = event_sender.send(WorkspaceCompletion::CatalogBackupFailed {
                        message: error.to_string(),
                    });
                }
            }
        }
        IndexWorkerCommand::ExecuteTrashOperation(operation) => {
            let result = match operation {
                TrashOperation::MoveToTrash { note_id } => {
                    notora_core::move_to_trash(workspace, catalog, note_id).map(|_| ())
                }
                TrashOperation::Restore { note_id } => {
                    notora_core::restore_from_trash(workspace, catalog, note_id).map(|_| ())
                }
                TrashOperation::RestoreWithRenamedPath { note_id } => {
                    notora_core::restore_from_trash_with_renamed_path(workspace, catalog, note_id)
                        .map(|_| ())
                }
                TrashOperation::PermanentlyDelete { note_id } => {
                    notora_core::permanently_delete_trashed_note(workspace, catalog, note_id)
                }
                TrashOperation::Empty => notora_core::empty_trash(workspace, catalog),
            };
            match result {
                Ok(()) => {
                    let _ = event_sender
                        .send(WorkspaceCompletion::TrashOperationCompleted { operation });
                    index_workspace(workspace, catalog, event_sender);
                }
                Err(error) => {
                    let failure = match (&operation, &error) {
                        (
                            TrashOperation::Restore { note_id },
                            notora_core::TrashError::RestoreConflict { .. },
                        ) => crate::action::TrashOperationFailure::RestoreConflict {
                            note_id: *note_id,
                        },
                        _ => crate::action::TrashOperationFailure::Message(error.to_string()),
                    };
                    let _ =
                        event_sender.send(WorkspaceCompletion::TrashOperationFailed { failure });
                }
            }
        }
        IndexWorkerCommand::PrepareDocument { request, source, cached_session } => {
            prepare_document_in_worker(
                workspace,
                catalog,
                request,
                source,
                cached_session,
                event_sender,
            );
        }
        IndexWorkerCommand::UnlockEncryptedDocument { request, password, generation, source } => {
            unlock_encrypted_document_in_worker(
                workspace,
                catalog,
                request,
                &password,
                generation,
                source,
                event_sender,
            )
        }
        IndexWorkerCommand::ReindexCatalog { relative_path } => {
            if let Some(path) = relative_path
                && workspace.resolve_relative_path(&path).is_ok_and(|path| path.is_file())
            {
                index_workspace_paths(workspace, catalog, &[path], event_sender);
                return;
            }
            index_workspace(workspace, catalog, event_sender);
        }
    }
}

fn navigation_tree_from_sources(
    workspace: &Workspace,
    catalog: &Catalog,
) -> Result<CatalogNavigationTree, String> {
    let directories = scan_workspace_directories(workspace).map_err(|error| error.to_string())?;
    let tags = catalog.tags_with_active_note_counts().map_err(|error| error.to_string())?;
    Ok(CatalogNavigationTree { directories, tags })
}

fn send_navigation_tree(
    workspace: &Workspace,
    catalog: &Catalog,
    event_sender: &WorkspaceEventSender,
) {
    match navigation_tree_from_sources(workspace, catalog) {
        Ok(tree) => {
            let _ = event_sender.send(WorkspaceCompletion::NavigationTreeLoaded { tree });
        }
        Err(message) => {
            let _ = event_sender.send(WorkspaceCompletion::NavigationTreeFailed { message });
        }
    }
}

fn execute_metadata_mutation_in_worker(
    catalog: &Catalog,
    mutation: MetadataMutation,
    event_sender: &WorkspaceEventSender,
) {
    let note_id = match &mutation {
        MetadataMutation::ToggleStar { note_id }
        | MetadataMutation::AttachTagByName { note_id, .. }
        | MetadataMutation::DetachTag { note_id, .. }
        | MetadataMutation::SetTitle { note_id, .. }
        | MetadataMutation::CompleteTitleInitializationFromHeader { note_id, .. }
        | MetadataMutation::CompleteTitleInitializationFromDocument { note_id, .. } => *note_id,
    };
    let mutation_result = match &mutation {
        MetadataMutation::ToggleStar { note_id } => catalog
            .toggle_note_starred(*note_id)
            .map(|_| crate::action::MetadataMutationOutcome::Applied),
        MetadataMutation::AttachTagByName { note_id, display_name } => catalog
            .attach_tag_by_name(*note_id, display_name)
            .map(|_| crate::action::MetadataMutationOutcome::Applied),
        MetadataMutation::DetachTag { note_id, tag_id } => catalog
            .detach_tag(*note_id, *tag_id)
            .map(|_| crate::action::MetadataMutationOutcome::Applied),
        MetadataMutation::SetTitle { note_id, title } => catalog
            .update_note_title(*note_id, title)
            .map(|()| crate::action::MetadataMutationOutcome::Applied),
        MetadataMutation::CompleteTitleInitializationFromHeader { note_id, title } => catalog
            .complete_title_initialization(*note_id, Some(title))
            .map(title_initialization_outcome),
        MetadataMutation::CompleteTitleInitializationFromDocument { note_id, title } => catalog
            .complete_title_initialization(*note_id, title.as_deref())
            .map(title_initialization_outcome),
    };
    let result = mutation_result.and_then(|outcome| {
        let metadata = catalog.note_editor_metadata(note_id)?.ok_or(
            notora_core::CatalogError::InvalidStoredValue {
                column: "note_id",
                value: note_id.to_string(),
            },
        )?;
        let tags = catalog.tags_for_note(note_id)?;
        Ok((note_id, metadata, tags, outcome))
    });
    match result {
        Ok((note_id, metadata, tags, outcome)) => {
            let _ = event_sender.send(WorkspaceCompletion::MetadataMutationCompleted {
                mutation,
                note_id,
                metadata,
                tags,
                outcome,
            });
        }
        Err(error) => {
            let _ = event_sender.send(WorkspaceCompletion::MetadataMutationFailed {
                mutation,
                message: error.to_string(),
            });
        }
    }
}

fn title_initialization_outcome(won: bool) -> crate::action::MetadataMutationOutcome {
    if won {
        crate::action::MetadataMutationOutcome::TitleInitializationWon
    } else {
        crate::action::MetadataMutationOutcome::TitleInitializationLost
    }
}

fn execute_note_command_in_worker(
    workspace: &Workspace,
    catalog: &Catalog,
    command: NoteCommand,
    event_sender: &WorkspaceEventSender,
) {
    match execute_note_command(workspace, catalog, command) {
        Ok(result) => {
            let _ = event_sender.send(WorkspaceCompletion::NoteCommandCompleted { result });
            index_workspace(workspace, catalog, event_sender);
        }
        Err(error) => {
            let _ = event_sender
                .send(WorkspaceCompletion::NoteCommandFailed { message: error.to_string() });
        }
    }
}

fn execute_directory_command_in_worker(
    workspace: &Workspace,
    catalog: &Catalog,
    command: notora_core::WorkspaceDirectoryCommand,
    event_sender: &WorkspaceEventSender,
) {
    match notora_core::execute_workspace_directory_command(workspace, command) {
        Ok(result) => {
            let _ = event_sender.send(WorkspaceCompletion::DirectoryCommandCompleted { result });
            send_navigation_tree(workspace, catalog, event_sender);
        }
        Err(error) => {
            let _ = event_sender
                .send(WorkspaceCompletion::DirectoryCommandFailed { message: error.to_string() });
        }
    }
}

fn prepare_document_in_worker(
    workspace: &Workspace,
    catalog: &Catalog,
    request: DocumentLoadRequest,
    source: WorkspaceDocumentSource,
    cached_session: Option<std::sync::Arc<textora_encryption::UnlockedNoteSession>>,
    event_sender: &WorkspaceEventSender,
) {
    enum PreparedWorkspaceDocument {
        Plain {
            document: crate::editor_adapter::LoadedDocument,
            metadata: notora_core::NoteEditorMetadata,
            tags: Vec<notora_core::TagSummary>,
        },
        UnlockRequired {
            title: String,
            metadata: notora_core::NoteEditorMetadata,
            tags: Vec<notora_core::TagSummary>,
        },
        Unlocked(crate::product::UnlockedWorkspaceDocument),
    }

    let result = (|| -> Result<PreparedWorkspaceDocument, String> {
        let DocumentIdentity::Note(note_id) = request.identity else {
            return Err("外部文档必须由外部文件会话加载".to_owned());
        };
        let path = resolve_workspace_document_path(workspace, catalog, note_id, source)?;
        let metadata = catalog
            .note_editor_metadata(note_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("笔记 {note_id} 缺少编辑区 metadata"))?;
        let tags = catalog.tags_for_note(note_id).map_err(|error| error.to_string())?;
        if metadata.encryption == notora_core::NoteEncryption::Encrypted {
            let serialized = std::fs::read(&path).map_err(|error| error.to_string())?;
            textora_encryption::inspect_encrypted_markdown(&serialized)
                .map_err(|error| error.to_string())?;
            if let Some(session) = cached_session
                && let Ok(contents) =
                    textora_encryption::decrypt_markdown_with_session(&serialized, &session)
            {
                let disk_revision = appkit_core::file_safety::capture_revision(&path)
                    .map_err(|error| error.to_string())?;
                return Ok(PreparedWorkspaceDocument::Unlocked(
                    crate::product::UnlockedWorkspaceDocument {
                        request,
                        origin: crate::product::EncryptedDocumentUnlockOrigin::CachedSession,
                        document: crate::editor_adapter::LoadedDocument {
                            path,
                            contents,
                            disk_revision: Some(disk_revision),
                        },
                        session,
                        metadata,
                        tags,
                    },
                ));
            }
            let title = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .ok_or_else(|| "加密笔记路径缺少文件名".to_owned())?;
            return Ok(PreparedWorkspaceDocument::UnlockRequired { title, metadata, tags });
        }
        let document =
            crate::editor_adapter::load_document(&path).map_err(|error| error.to_string())?;
        Ok(PreparedWorkspaceDocument::Plain { document, metadata, tags })
    })();
    match result {
        Ok(PreparedWorkspaceDocument::Plain { document, metadata, tags }) => {
            let _ = event_sender.send(WorkspaceCompletion::DocumentLoaded {
                request,
                document,
                metadata,
                tags,
            });
        }
        Ok(PreparedWorkspaceDocument::UnlockRequired { title, metadata, tags }) => {
            let _ = event_sender.send(WorkspaceCompletion::EncryptedDocumentUnlockRequired {
                request,
                title,
                metadata,
                tags,
            });
        }
        Ok(PreparedWorkspaceDocument::Unlocked(unlocked)) => {
            let _ = event_sender.send(WorkspaceCompletion::EncryptedDocumentUnlocked { unlocked });
        }
        Err(message) => {
            let _ = event_sender.send(WorkspaceCompletion::DocumentLoadFailed { request, message });
        }
    }
}

fn unlock_encrypted_document_in_worker(
    workspace: &Workspace,
    catalog: &Catalog,
    request: DocumentLoadRequest,
    password: &textora_encryption::EncryptionPassword,
    generation: u64,
    source: WorkspaceDocumentSource,
    event_sender: &WorkspaceEventSender,
) {
    let result = (|| {
        let DocumentIdentity::Note(note_id) = request.identity else {
            return Err("外部文档不能作为工作区加密笔记解锁".to_owned());
        };
        let path = resolve_workspace_document_path(workspace, catalog, note_id, source)?;
        let metadata = catalog
            .note_editor_metadata(note_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("笔记 {note_id} 缺少编辑区 metadata"))?;
        let tags = catalog.tags_for_note(note_id).map_err(|error| error.to_string())?;
        let serialized = std::fs::read(&path).map_err(|error| error.to_string())?;
        let disk_revision =
            appkit_core::file_safety::capture_revision(&path).map_err(|error| error.to_string())?;
        let unlocked = textora_encryption::unlock_encrypted_markdown(&serialized, password)
            .map_err(|_| "密码错误".to_owned())?;
        let (contents, session) = unlocked.into_parts();
        Ok(crate::product::UnlockedWorkspaceDocument {
            request,
            origin: crate::product::EncryptedDocumentUnlockOrigin::Password { generation },
            document: crate::editor_adapter::LoadedDocument {
                path,
                contents,
                disk_revision: Some(disk_revision),
            },
            session: std::sync::Arc::new(session),
            metadata,
            tags,
        })
    })();
    let completion = match result {
        Ok(unlocked) => WorkspaceCompletion::EncryptedDocumentUnlocked { unlocked },
        Err(message) => {
            WorkspaceCompletion::EncryptedDocumentUnlockFailed { request, generation, message }
        }
    };
    let _ = event_sender.send(completion);
}

fn resolve_workspace_document_path(
    workspace: &Workspace,
    catalog: &Catalog,
    note_id: notora_core::NoteId,
    source: WorkspaceDocumentSource,
) -> Result<std::path::PathBuf, String> {
    match source {
        WorkspaceDocumentSource::ActiveNote => catalog
            .active_note(note_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("活动笔记 {note_id} 已不存在"))
            .and_then(|note| {
                workspace
                    .resolve_relative_path(&note.relative_path)
                    .map_err(|error| error.to_string())
            }),
        WorkspaceDocumentSource::TrashedNote => {
            notora_core::trash::resolve_trashed_note_path(workspace, catalog, note_id)
                .map_err(|error| error.to_string())
        }
    }
}

fn index_workspace(workspace: &Workspace, catalog: &Catalog, event_sender: &WorkspaceEventSender) {
    match scan_workspace(workspace, catalog) {
        Ok(completion) => {
            let _ = event_sender.send(WorkspaceCompletion::WorkspaceScanCompleted { completion });
        }
        Err(error) => {
            let _ = event_sender
                .send(WorkspaceCompletion::WorkspaceIndexFailed { message: error.to_string() });
        }
    }
}

fn index_workspace_paths(
    workspace: &Workspace,
    catalog: &Catalog,
    relative_paths: &[PathBuf],
    event_sender: &WorkspaceEventSender,
) {
    match scan_workspace_paths(workspace, catalog, relative_paths) {
        Ok(completion) => {
            let _ = event_sender.send(WorkspaceCompletion::WorkspaceScanCompleted { completion });
        }
        Err(error) => {
            let _ = event_sender
                .send(WorkspaceCompletion::WorkspaceIndexFailed { message: error.to_string() });
        }
    }
}

fn index_workspace_file_batch(
    workspace: &Workspace,
    catalog: &Catalog,
    batch: &WorkspaceFileBatch,
    event_sender: &WorkspaceEventSender,
) -> Vec<WorkspaceNoteRelocation> {
    let previous_paths_by_note_id = match catalog.active_notes() {
        Ok(notes) => notes
            .into_iter()
            .map(|note| (note.note_id, note.relative_path))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(error) => {
            let _ = event_sender
                .send(WorkspaceCompletion::WorkspaceIndexFailed { message: error.to_string() });
            return Vec::new();
        }
    };
    match scan_workspace_file_batch(workspace, catalog, batch) {
        Ok(completion) => {
            let _ = event_sender.send(WorkspaceCompletion::WorkspaceScanCompleted { completion });
            match collect_note_relocations(catalog, &previous_paths_by_note_id) {
                Ok(relocations) => relocations,
                Err(error) => {
                    let _ = event_sender.send(WorkspaceCompletion::WorkspaceIndexFailed {
                        message: error.to_string(),
                    });
                    Vec::new()
                }
            }
        }
        Err(error) => {
            let _ = event_sender
                .send(WorkspaceCompletion::WorkspaceIndexFailed { message: error.to_string() });
            Vec::new()
        }
    }
}

fn collect_note_relocations(
    catalog: &Catalog,
    previous_paths_by_note_id: &std::collections::HashMap<notora_core::NoteId, PathBuf>,
) -> Result<Vec<WorkspaceNoteRelocation>, notora_core::CatalogError> {
    let mut relocations = Vec::new();
    for note in catalog.active_notes()? {
        let Some(previous_path) = previous_paths_by_note_id.get(&note.note_id) else {
            continue;
        };
        if *previous_path == note.relative_path {
            continue;
        }
        let Some(metadata) = catalog.note_editor_metadata(note.note_id)? else {
            continue;
        };
        relocations.push(WorkspaceNoteRelocation {
            note_id: note.note_id,
            from: previous_path.clone(),
            to: note.relative_path,
            metadata,
            tags: catalog.tags_for_note(note.note_id)?,
        });
    }
    Ok(relocations)
}

#[cfg(test)]
mod tests;
