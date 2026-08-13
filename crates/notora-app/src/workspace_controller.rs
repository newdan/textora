//! 工作区选择及后台扫描服务协调。

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notora_core::note_command::NoteCommand;
use notora_core::{
    Catalog, CatalogNavigationTree, DocumentIdentity, Workspace, WorkspaceDescriptor,
    WorkspaceError, WorkspaceFileBatch, WorkspaceFileMonitor, WorkspaceFileMonitorError,
    execute_note_command, scan_workspace, scan_workspace_directories, scan_workspace_file_batch,
    scan_workspace_paths,
};

use crate::action::{CardQuery, DocumentLoadRequest, MetadataMutation, TrashOperation};
use crate::index_worker::{IndexWorker, IndexWorkerCommand, WorkspaceDocumentSource};
use crate::product::{
    NotoraProduct, NotoraProductEventSender, WorkspaceCompletion, WorkspaceEventScope,
    WorkspaceEventSender, WorkspaceNoteRelocation,
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
            Self::IndexerThreadUnavailable
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
    catalog_backups_directory: Option<PathBuf>,
    migration_backup_retention: notora_core::BackupRetention,
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
            catalog_backups_directory: Some(catalog_backups_directory),
            migration_backup_retention,
        }
    }
    pub fn execute(
        &mut self,
        command: WorkspaceCommand,
        product: &mut NotoraProduct,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
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
    ) -> Result<(), WorkspaceControllerError> {
        self.prepare_workspace_document(request, WorkspaceDocumentSource::ActiveNote)
    }

    pub fn prepare_trashed_document(
        &self,
        request: DocumentLoadRequest,
    ) -> Result<(), WorkspaceControllerError> {
        self.prepare_workspace_document(request, WorkspaceDocumentSource::TrashedNote)
    }

    fn prepare_workspace_document(
        &self,
        request: DocumentLoadRequest,
        source: WorkspaceDocumentSource,
    ) -> Result<(), WorkspaceControllerError> {
        let session =
            self.active_session.as_ref().ok_or(WorkspaceControllerError::NoActiveWorkspace)?;
        session
            .indexer
            .send(IndexWorkerCommand::PrepareDocument { request, source })
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
        let generation = self.next_generation.wrapping_add(1);
        let descriptor = workspace.descriptor();
        let catalog_backups_directory = self
            .catalog_backups_directory
            .as_ref()
            .map(|directory| directory.join(descriptor.workspace_id.to_string()));
        let event_sender = product.event_sender();
        let session = WorkspaceSession::start(
            workspace,
            catalog_path,
            catalog_backups_directory,
            self.migration_backup_retention,
            generation,
            event_sender,
        )?;
        self.close_active_session();
        self.next_generation = generation;
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
            catalog_backups_directory: None,
            migration_backup_retention: default_migration_backup_retention(),
        }
    }
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
        let (mut file_monitor, file_batches) =
            WorkspaceFileMonitor::start(workspace.root().to_path_buf())
                .map_err(WorkspaceControllerError::FileMonitor)?;
        let (startup_sender, startup_receiver) = mpsc::channel();
        let mut indexer = IndexWorker::start(move |command_receiver| {
            run_indexer_with_startup(
                workspace,
                catalog_path,
                catalog_backups_directory,
                migration_backup_retention,
                file_batches,
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
    file_batches: mpsc::Receiver<WorkspaceFileBatch>,
    command_receiver: mpsc::Receiver<IndexWorkerCommand>,
    event_sender: WorkspaceEventSender,
) {
    run_indexer_with_startup(
        workspace,
        catalog_path,
        catalog_backups_directory,
        migration_backup_retention,
        file_batches,
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
    file_batches: mpsc::Receiver<WorkspaceFileBatch>,
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
    let mut pending_presence_confirmation_paths = BTreeSet::new();
    let mut presence_confirmation_due_at = None;
    loop {
        while let Ok(command) = command_receiver.try_recv() {
            execute_workspace_command(&workspace, &catalog, command, &event_sender);
        }
        match file_batches.recv_timeout(WORKSPACE_WORKER_IDLE_WAIT) {
            Ok(batch) => {
                pending_presence_confirmation_paths.extend(batch.relative_paths.iter().cloned());
                presence_confirmation_due_at =
                    Some(Instant::now() + WATCHER_PRESENCE_CONFIRMATION_DELAY);
                let note_relocations =
                    index_workspace_file_batch(&workspace, &catalog, &batch, &event_sender);
                send_navigation_tree(&workspace, &catalog, &event_sender);
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
                return;
            }
        }
        if presence_confirmation_due_at.is_some_and(|due_at| due_at <= Instant::now()) {
            let relative_paths = std::mem::take(&mut pending_presence_confirmation_paths)
                .into_iter()
                .collect::<Vec<_>>();
            presence_confirmation_due_at = None;
            index_workspace_paths(&workspace, &catalog, &relative_paths, &event_sender);
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
        IndexWorkerCommand::PrepareDocument { request, source } => {
            prepare_document_in_worker(workspace, catalog, request, source, event_sender);
        }
        IndexWorkerCommand::ReindexCatalog => {
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
    event_sender: &WorkspaceEventSender,
) {
    let result = (|| {
        let DocumentIdentity::Note(note_id) = request.identity else {
            return Err("外部文档必须由外部文件会话加载".to_owned());
        };
        let path = match source {
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
        }?;
        let metadata = catalog
            .note_editor_metadata(note_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("笔记 {note_id} 缺少编辑区 metadata"))?;
        let tags = catalog.tags_for_note(note_id).map_err(|error| error.to_string())?;
        let document =
            crate::editor_adapter::load_document(&path).map_err(|error| error.to_string())?;
        Ok((document, metadata, tags))
    })();
    match result {
        Ok((document, metadata, tags)) => {
            let _ = event_sender.send(WorkspaceCompletion::DocumentLoaded {
                request,
                document,
                metadata,
                tags,
            });
        }
        Err(message) => {
            let _ = event_sender.send(WorkspaceCompletion::DocumentLoadFailed { request, message });
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
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use appkit_shell::ProductHost;

    use super::{
        ActiveWorkspace, CATALOG_FILE_NAME, Catalog, Workspace, WorkspaceCommand,
        WorkspaceCommandResult, WorkspaceController, WorkspaceControllerError,
        default_migration_backup_retention, run_indexer, scan_workspace,
    };
    use crate::action::{CardQuery, DocumentLoadRequest};
    use crate::product::{
        NotoraProduct, NotoraProductEvent, WorkspaceCompletion, WorkspaceEventScope,
        WorkspaceEventSender,
    };
    use notora_core::DocumentIdentity;

    fn active_completion<'a>(
        event: &'a NotoraProductEvent,
        active_workspace: &ActiveWorkspace,
    ) -> Option<&'a WorkspaceCompletion> {
        let NotoraProductEvent::Workspace(event) = event else {
            return None;
        };
        if event.scope.workspace_id != active_workspace.descriptor.workspace_id
            || event.scope.generation != active_workspace.generation
        {
            return None;
        }
        Some(&event.completion)
    }

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
    fn create_rejects_an_existing_target_without_opening_it() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let existing_root = directory.path().join("existing");
        fs::create_dir(&existing_root).expect("existing target should be created");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();

        assert!(matches!(
            controller.execute(
                WorkspaceCommand::Create { root: existing_root.clone() },
                &mut product,
            ),
            Err(WorkspaceControllerError::CreateDirectory { path, source })
                if path == existing_root && source.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(controller.active_workspace(), None);
        assert!(!existing_root.join(".notora").exists());
    }

    #[test]
    fn failed_replacement_does_not_advance_generation_or_close_the_active_session() {
        let first_directory =
            tempfile::tempdir().expect("first workspace test directory should be created");
        let corrupt_directory =
            tempfile::tempdir().expect("corrupt workspace test directory should be created");
        fs::create_dir(corrupt_directory.path().join(".notora"))
            .expect("metadata directory should be created");
        fs::write(corrupt_directory.path().join(".notora/workspace.toml"), "not valid toml")
            .expect("corrupt manifest should be written");
        let second_directory =
            tempfile::tempdir().expect("second workspace test directory should be created");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(first_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: first_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("first workspace should open")
        else {
            panic!("first workspace should become active");
        };

        assert!(
            controller
                .execute(
                    WorkspaceCommand::OpenExisting { root: corrupt_directory.path().to_path_buf() },
                    &mut product,
                )
                .is_err()
        );
        assert_eq!(controller.active_workspace(), Some(first_workspace.clone()));

        let WorkspaceCommandResult::Opened(second_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: second_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("second valid workspace should open")
        else {
            panic!("second workspace should become active");
        };
        assert_eq!(second_workspace.generation, first_workspace.generation + 1);
    }

    #[test]
    fn indexer_catalog_startup_failure_does_not_replace_the_active_session() {
        let active_directory =
            tempfile::tempdir().expect("active workspace directory should exist");
        let target_directory =
            tempfile::tempdir().expect("target workspace directory should exist");
        let target_workspace = notora_core::Workspace::open_or_initialize(target_directory.path())
            .expect("target workspace metadata should initialize");
        let target_catalog_path = target_workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let target_catalog = rusqlite::Connection::open(&target_catalog_path)
            .expect("target catalog fixture should open");
        target_catalog
            .pragma_update(None, "user_version", 9_999_i64)
            .expect("future catalog version should be written");
        drop(target_catalog);
        drop(target_workspace);
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: active_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("active workspace should open")
        else {
            panic!("active workspace should become active");
        };

        assert!(
            controller
                .execute(
                    WorkspaceCommand::OpenExisting { root: target_directory.path().to_path_buf() },
                    &mut product,
                )
                .is_err()
        );
        assert_eq!(controller.active_workspace(), Some(active_workspace));
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
        WorkspaceEventSender::new(
            sender.clone(),
            WorkspaceEventScope {
                workspace_id: first_workspace.descriptor.workspace_id,
                generation: first_workspace.generation,
            },
        )
        .send(WorkspaceCompletion::WorkspaceChanged {
            changed_paths: vec!["late.md".into()],
            note_relocations: vec![],
        })
        .expect("product receiver should stay available");
        WorkspaceEventSender::new(
            sender,
            WorkspaceEventScope {
                workspace_id: second_workspace.descriptor.workspace_id,
                generation: second_workspace.generation,
            },
        )
        .send(WorkspaceCompletion::WorkspaceChanged {
            changed_paths: vec!["current.md".into()],
            note_relocations: vec![],
        })
        .expect("product receiver should stay available");

        let _ = product.drain_product_events();
        let events = product.take_events();
        assert!(events.iter().any(|event| matches!(
            event,
            NotoraProductEvent::Workspace(event)
                if event.scope.workspace_id == second_workspace.descriptor.workspace_id
                    && event.scope.generation == second_workspace.generation
                    && matches!(event.completion, WorkspaceCompletion::WorkspaceChanged { .. })
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            NotoraProductEvent::Workspace(event)
                if event.scope.workspace_id == first_workspace.descriptor.workspace_id
                    && event.scope.generation == first_workspace.generation
                    && matches!(event.completion, WorkspaceCompletion::WorkspaceChanged { .. })
        )));
    }

    #[test]
    fn watcher_disconnection_reports_a_recoverable_index_failure() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace = notora_core::Workspace::open_or_initialize(directory.path())
            .expect("workspace metadata should be initialized");
        let descriptor = workspace.descriptor();
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        Catalog::open(&catalog_path).expect("catalog should initialize before worker startup");
        let (file_batch_sender, file_batches) = mpsc::channel();
        let (_command_sender, command_receiver) = mpsc::channel();
        let mut product = NotoraProduct::new();
        product.set_active_workspace(descriptor.workspace_id, 1);
        drop(file_batch_sender);

        run_indexer(
            workspace,
            catalog_path,
            None,
            default_migration_backup_retention(),
            file_batches,
            command_receiver,
            WorkspaceEventSender::new(
                product.event_sender(),
                WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
            ),
        );

        let _ = product.drain_product_events();
        assert!(product.take_events().iter().any(|event| {
            matches!(
                event,
                NotoraProductEvent::Workspace(event)
                    if event.scope.workspace_id == descriptor.workspace_id
                        && event.scope.generation == 1
                        && matches!(
                            &event.completion,
                            WorkspaceCompletion::WorkspaceIndexFailed { message }
                                if message.contains("文件监视器已断开")
                        )
            )
        }));
    }

    #[test]
    fn indexer_recovery_after_preflight_notifies_the_product_about_restored_metadata() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let backup_directory =
            tempfile::tempdir().expect("backup test directory should be created");
        let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
            .expect("workspace metadata should be initialized");
        let descriptor = workspace.descriptor();
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");
        catalog.create_tag("Retained").expect("fixture metadata should persist");
        let workspace_backups = backup_directory.path().join(descriptor.workspace_id.to_string());
        notora_core::create_catalog_backup(
            &catalog,
            &workspace_backups,
            notora_core::BackupRetention::keep_latest(1)
                .expect("positive backup retention should be valid"),
        )
        .expect("fixture backup should persist");
        drop(catalog);
        fs::write(&catalog_path, "damaged after the startup preflight")
            .expect("fixture catalog should be damaged");
        let (file_batch_sender, file_batches) = mpsc::channel();
        let (_command_sender, command_receiver) = mpsc::channel();
        let mut product = NotoraProduct::new();
        product.set_active_workspace(descriptor.workspace_id, 1);
        drop(file_batch_sender);

        run_indexer(
            workspace,
            catalog_path,
            Some(workspace_backups),
            default_migration_backup_retention(),
            file_batches,
            command_receiver,
            WorkspaceEventSender::new(
                product.event_sender(),
                WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
            ),
        );

        let _ = product.drain_product_events();
        assert!(product.take_events().iter().any(|event| {
            matches!(
                event,
                NotoraProductEvent::Workspace(event)
                    if event.scope.workspace_id == descriptor.workspace_id
                        && event.scope.generation == 1
                        && matches!(
                            &event.completion,
                            WorkspaceCompletion::CatalogRecoveryNotified { message }
                                if message.contains("元数据已从")
                        )
            )
        }));
    }

    #[test]
    fn watcher_batch_schedules_a_second_presence_check_for_a_deleted_note() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("removed.md");
        fs::write(&note_path, "# Removed").expect("fixture note should be written");
        let workspace = notora_core::Workspace::open_or_initialize(directory.path())
            .expect("workspace metadata should be initialized");
        let descriptor = workspace.descriptor();
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let (file_batch_sender, file_batches) = mpsc::channel();
        let (_command_sender, command_receiver) = mpsc::channel();
        let mut product = NotoraProduct::new();
        product.set_active_workspace(descriptor.workspace_id, 1);
        let event_sender = product.event_sender();
        let worker = thread::spawn({
            let catalog_path = catalog_path.clone();
            move || {
                run_indexer(
                    workspace,
                    catalog_path,
                    None,
                    default_migration_backup_retention(),
                    file_batches,
                    command_receiver,
                    WorkspaceEventSender::new(
                        event_sender,
                        WorkspaceEventScope {
                            workspace_id: descriptor.workspace_id,
                            generation: 1,
                        },
                    ),
                )
            }
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut completed_scans = 0;
        while completed_scans < 1 {
            let _ = product.drain_product_events();
            completed_scans += product
                .take_events()
                .into_iter()
                .filter(|event| matches!(
                    event,
                    NotoraProductEvent::Workspace(event)
                        if matches!(event.completion, WorkspaceCompletion::WorkspaceScanCompleted { .. })
                ))
                .count();
            assert!(Instant::now() < deadline, "initial scan should complete promptly");
            thread::sleep(Duration::from_millis(10));
        }
        fs::remove_file(&note_path).expect("fixture note should be removed");
        file_batch_sender
            .send(notora_core::WorkspaceFileBatch {
                relative_paths: vec!["removed.md".into()],
                changes: vec![notora_core::WorkspaceFileChange::Removed("removed.md".into())],
            })
            .expect("watcher batch should reach the indexer");

        while completed_scans < 3 {
            let _ = product.drain_product_events();
            completed_scans += product
                .take_events()
                .into_iter()
                .filter(|event| matches!(
                    event,
                    NotoraProductEvent::Workspace(event)
                        if matches!(event.completion, WorkspaceCompletion::WorkspaceScanCompleted { .. })
                ))
                .count();
            assert!(
                Instant::now() < deadline,
                "deleted note should receive a delayed confirmation scan"
            );
            thread::sleep(Duration::from_millis(10));
        }
        drop(file_batch_sender);
        worker.join().expect("indexer should stop after its file channel closes");
        let catalog = Catalog::open(&catalog_path).expect("catalog should reopen after indexing");
        assert!(catalog.active_notes().expect("active notes should query").is_empty());
    }

    #[test]
    fn watcher_batch_refreshes_navigation_after_an_external_directory_change() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace = notora_core::Workspace::open_or_initialize(directory.path())
            .expect("workspace metadata should be initialized");
        let descriptor = workspace.descriptor();
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let (file_batch_sender, file_batches) = mpsc::channel();
        let (_command_sender, command_receiver) = mpsc::channel();
        let mut product = NotoraProduct::new();
        product.set_active_workspace(descriptor.workspace_id, 1);
        let event_sender = product.event_sender();
        let worker = thread::spawn(move || {
            run_indexer(
                workspace,
                catalog_path,
                None,
                default_migration_backup_retention(),
                file_batches,
                command_receiver,
                WorkspaceEventSender::new(
                    event_sender,
                    WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
                ),
            )
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            if product.take_events().iter().any(|event| {
                matches!(
                    event,
                    NotoraProductEvent::Workspace(event)
                        if matches!(
                            event.completion,
                            WorkspaceCompletion::WorkspaceScanCompleted { .. }
                        )
                )
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "initial scan should complete promptly");
            thread::sleep(Duration::from_millis(10));
        }

        let external_directory = PathBuf::from("external-empty");
        fs::create_dir(directory.path().join(&external_directory))
            .expect("external directory fixture should be created");
        file_batch_sender
            .send(notora_core::WorkspaceFileBatch {
                relative_paths: vec![external_directory.clone()],
                changes: vec![notora_core::WorkspaceFileChange::Created(
                    external_directory.clone(),
                )],
            })
            .expect("watcher batch should reach the indexer");

        loop {
            let _ = product.drain_product_events();
            if product.take_events().iter().any(|event| {
                matches!(
                    event,
                    NotoraProductEvent::Workspace(event)
                        if matches!(
                            &event.completion,
                            WorkspaceCompletion::NavigationTreeLoaded { tree }
                                if tree.directories.contains(&external_directory)
                        )
                )
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "external directory change should refresh the navigation tree"
            );
            thread::sleep(Duration::from_millis(10));
        }

        drop(file_batch_sender);
        worker.join().expect("indexer should stop after its file channel closes");
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
            .execute_note_command(notora_core::note_command::NoteCommand::CreateConfigured(
                notora_core::note_command::ConfiguredCreateNoteRequest {
                    kind: notora_core::DocumentKind::Markdown,
                    target_directory: None,
                    encryption: notora_core::NoteEncryption::Unencrypted,
                },
            ))
            .expect("active workspace should accept the note command");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::NoteCommandCompleted { result }
                            if result.note.relative_path == std::path::Path::new("无标题.md")
                    )
                })
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
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::CardQueryCompleted { query: completed_query, page }
                            if completed_query == &query && page.cards.is_empty()
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "card query completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn document_load_completion_carries_editor_metadata_and_formal_tags() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        std::fs::write(directory.path().join("note.md"), "# 路线图\n\n正文")
            .expect("workspace note should be written");
        let workspace = Workspace::open_or_initialize(directory.path())
            .expect("workspace metadata should initialize");
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let catalog = Catalog::open(&catalog_path).expect("workspace catalog should open");
        scan_workspace(&workspace, &catalog).expect("workspace note should be indexed");
        let note_id = catalog
            .active_notes()
            .expect("active notes should load")
            .first()
            .expect("indexed note should exist")
            .note_id;
        let formal_tag = catalog.create_tag("产品/Notora").expect("formal tag should be created");
        let formal_tag_id = formal_tag.tag_id;
        catalog.attach_tag(note_id, formal_tag_id).expect("formal tag should attach");
        drop(catalog);
        drop(workspace);

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
        let request = DocumentLoadRequest {
            identity: DocumentIdentity::Note(note_id),
            selection_generation: 7,
        };
        controller.prepare_document(request).expect("worker should accept document loading");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::DocumentLoaded {
                            request: completed_request,
                            metadata,
                            tags,
                            ..
                        } if *completed_request == request
                            && metadata.note_id == note_id
                            && tags.iter().any(|tag| tag.tag_id == formal_tag_id)
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "document metadata should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn trashed_document_load_reads_the_controlled_trash_file() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        std::fs::write(directory.path().join("trashed.md"), "# 已删除\n\n回收站正文")
            .expect("workspace note should be written");
        let workspace = Workspace::open_or_initialize(directory.path())
            .expect("workspace metadata should initialize");
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let catalog = Catalog::open(&catalog_path).expect("workspace catalog should open");
        scan_workspace(&workspace, &catalog).expect("workspace note should be indexed");
        let note_id = catalog
            .active_notes()
            .expect("active notes should load")
            .first()
            .expect("indexed note should exist")
            .note_id;
        notora_core::move_to_trash(&workspace, &catalog, note_id)
            .expect("fixture note should move to trash");
        drop(catalog);
        drop(workspace);

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
        let request = DocumentLoadRequest {
            identity: DocumentIdentity::Note(note_id),
            selection_generation: 9,
        };
        controller
            .prepare_trashed_document(request)
            .expect("worker should accept trashed document loading");
        let canonical_trash_root = std::fs::canonicalize(directory.path().join(".notora/trash"))
            .expect("trash fixture root should canonicalize");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::DocumentLoaded {
                            request: completed_request,
                            document,
                            ..
                        } if *completed_request == request
                            && document.contents == "# 已删除\n\n回收站正文"
                            && document.path.starts_with(&canonical_trash_root)
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "trashed document should load promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn active_workspace_worker_executes_metadata_mutations_off_the_main_thread() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        std::fs::write(directory.path().join("note.md"), "body")
            .expect("workspace note should be written");
        let workspace = Workspace::open_or_initialize(directory.path())
            .expect("workspace metadata should initialize");
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let catalog = Catalog::open(&catalog_path).expect("workspace catalog should open");
        scan_workspace(&workspace, &catalog).expect("workspace note should be indexed");
        let note_id = catalog
            .active_notes()
            .expect("active notes should load")
            .first()
            .expect("indexed note should exist")
            .note_id;
        drop(catalog);
        drop(workspace);
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
            .execute_metadata_mutation(crate::action::MetadataMutation::AttachTagByName {
                note_id,
                display_name: "产品/Notora".to_owned(),
            })
            .expect("active workspace should accept metadata mutations");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::MetadataMutationCompleted {
                            note_id: completed_note_id,
                            metadata,
                            tags,
                            mutation: crate::action::MetadataMutation::AttachTagByName {
                                note_id: mutation_note_id,
                                display_name,
                            },
                            ..
                        } if *completed_note_id == note_id
                            && metadata.note_id == note_id
                            && metadata.encryption == notora_core::NoteEncryption::Unencrypted
                            && *mutation_note_id == note_id
                            && display_name == "产品/Notora"
                            && tags.len() == 1
                            && tags[0].display_name == "产品/Notora"
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "metadata completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn catalog_backup_is_created_by_the_workspace_worker_and_reported_through_the_product_channel()
    {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let backup_directory =
            tempfile::tempdir().expect("backup test directory should be created");
        let mut controller = WorkspaceController::with_catalog_backups_directory(
            backup_directory.path().to_path_buf(),
        );
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("workspace should open")
        else {
            panic!("open command should activate the workspace");
        };
        let retention = notora_core::BackupRetention::keep_latest(1)
            .expect("positive retention should be valid");
        let workspace_backups =
            backup_directory.path().join(active_workspace.descriptor.workspace_id.to_string());

        controller
            .create_catalog_backup(workspace_backups.clone(), retention)
            .expect("worker should accept the catalog backup request");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::CatalogBackupCompleted { backup_path }
                            if backup_path.is_file()
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "catalog backup completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn navigation_tree_uses_filesystem_directories_even_when_they_are_empty() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(workspace_directory.path().join("empty"))
            .expect("empty directory should be created");
        fs::create_dir_all(workspace_directory.path().join("docs/plans"))
            .expect("nested empty directory should be created");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("workspace should open")
        else {
            panic!("open command should activate the workspace");
        };

        controller.query_navigation_tree().expect("navigation query should reach the worker");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            let loaded_directories = events.iter().find_map(|event| {
                let WorkspaceCompletion::NavigationTreeLoaded { tree } =
                    active_completion(event, &active_workspace)?
                else {
                    return None;
                };
                Some(tree.directories.clone())
            });
            if let Some(directories) = loaded_directories {
                assert_eq!(
                    directories,
                    vec![
                        PathBuf::from("docs"),
                        PathBuf::from("docs/plans"),
                        PathBuf::from("empty"),
                    ]
                );
                break;
            }
            assert!(Instant::now() < deadline, "navigation tree should load promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn active_workspace_worker_creates_directory_and_refreshes_navigation() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut controller = WorkspaceController::default();
        let mut product = NotoraProduct::new();
        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("workspace should open")
        else {
            panic!("open command should activate the workspace");
        };

        controller
            .execute_directory_command(notora_core::WorkspaceDirectoryCommand::Create {
                parent_relative_path: PathBuf::new(),
                name: "notes".to_owned(),
            })
            .expect("directory command should reach the worker");

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut completed = false;
        let mut refreshed = false;
        while !completed || !refreshed {
            let _ = product.drain_product_events();
            for event in product.take_events() {
                match active_completion(&event, &active_workspace) {
                    Some(WorkspaceCompletion::DirectoryCommandCompleted { result }) => {
                        completed = result.relative_path == std::path::Path::new("notes");
                    }
                    Some(WorkspaceCompletion::NavigationTreeLoaded { tree }) => {
                        refreshed = tree.directories.contains(&PathBuf::from("notes"));
                    }
                    _ => {}
                }
            }
            assert!(Instant::now() < deadline, "directory completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(workspace_directory.path().join("notes").is_dir());
    }

    #[test]
    fn opening_an_older_catalog_creates_a_backup_before_migration() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let backup_directory =
            tempfile::tempdir().expect("backup test directory should be created");
        let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
            .expect("workspace metadata should be initialized");
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        let test_catalog = rusqlite::Connection::open(&catalog_path)
            .expect("test catalog should reopen for schema setup");
        test_catalog
            .execute_batch(
                "CREATE TABLE notes (
                    note_id TEXT PRIMARY KEY NOT NULL,
                    relative_path TEXT NOT NULL UNIQUE,
                    kind INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    excerpt TEXT NOT NULL,
                    modified_ns INTEGER NOT NULL,
                    file_size INTEGER NOT NULL,
                    content_hash BLOB NOT NULL,
                    starred INTEGER NOT NULL DEFAULT 0 CHECK (starred IN (0, 1)),
                    lifecycle INTEGER NOT NULL DEFAULT 0 CHECK (lifecycle IN (0, 1))
                );
                CREATE INDEX notes_active_path_index ON notes(lifecycle, relative_path);
                CREATE INDEX notes_starred_modified_index
                    ON notes(lifecycle, starred, modified_ns DESC);
                CREATE TABLE tags (
                    tag_id TEXT PRIMARY KEY NOT NULL,
                    normalized_name TEXT NOT NULL UNIQUE,
                    display_name TEXT NOT NULL
                );
                CREATE TABLE note_tags (
                    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
                    tag_id TEXT NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
                    PRIMARY KEY (note_id, tag_id)
                );
                CREATE TABLE trash_entries (
                    note_id TEXT PRIMARY KEY NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
                    original_relative_path TEXT NOT NULL,
                    trash_relative_path TEXT NOT NULL UNIQUE,
                    deleted_at_ns INTEGER NOT NULL
                );
                PRAGMA user_version = 1;",
            )
            .expect("test catalog should match the previous schema version");

        let workspace_backup_directory =
            backup_directory.path().join(workspace.descriptor().workspace_id.to_string());
        let mut controller = WorkspaceController::with_catalog_backups_directory_and_retention(
            backup_directory.path().to_path_buf(),
            notora_core::BackupRetention::keep_latest(1)
                .expect("positive migration backup retention should be valid"),
        );
        let mut product = NotoraProduct::new();

        controller
            .execute(
                WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("older catalog should migrate after its backup is complete");

        let deadline = Instant::now() + Duration::from_secs(2);
        while Catalog::migration_required(&catalog_path)
            .expect("workspace catalog migration state should be readable")
        {
            assert!(Instant::now() < deadline, "catalog migration should complete promptly");
            thread::sleep(Duration::from_millis(10));
        }

        let backup_path = notora_core::latest_valid_catalog_backup(&workspace_backup_directory)
            .expect("backup directory should be readable")
            .expect("migration should create a catalog backup");
        assert!(
            Catalog::migration_required(&backup_path)
                .expect("backup should remain readable before migration")
        );
        assert!(
            !Catalog::migration_required(&catalog_path)
                .expect("workspace catalog should be migrated after opening")
        );
    }

    #[test]
    fn catalog_recovery_notifies_the_product_when_metadata_cannot_be_restored() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let backup_directory =
            tempfile::tempdir().expect("backup test directory should be created");
        let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
            .expect("workspace metadata should be initialized");
        let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
        std::fs::write(&catalog_path, "not a sqlite catalog")
            .expect("test catalog should be damaged");
        let mut controller = WorkspaceController::with_catalog_backups_directory(
            backup_directory.path().to_path_buf(),
        );
        let mut product = NotoraProduct::new();

        let WorkspaceCommandResult::Opened(active_workspace) = controller
            .execute(
                WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
                &mut product,
            )
            .expect("damaged catalog should rebuild without blocking workspace access")
        else {
            panic!("workspace should open after catalog recovery");
        };

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::CatalogRecoveryNotified { message }
                            if message.contains("元数据可能已丢失")
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "catalog recovery notice should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn active_workspace_worker_moves_only_the_requested_note_to_trash() {
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
            .execute_note_command(notora_core::note_command::NoteCommand::CreateConfigured(
                notora_core::note_command::ConfiguredCreateNoteRequest {
                    kind: notora_core::DocumentKind::Markdown,
                    target_directory: None,
                    encryption: notora_core::NoteEncryption::Unencrypted,
                },
            ))
            .expect("worker should create a note");

        let deadline = Instant::now() + Duration::from_secs(2);
        let note_id = loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if let Some(note_id) =
                events.iter().find_map(|event| match active_completion(event, &active_workspace) {
                    Some(WorkspaceCompletion::NoteCommandCompleted { result }) => {
                        Some(result.note.note_id)
                    }
                    _ => None,
                })
            {
                break note_id;
            }
            assert!(Instant::now() < deadline, "note creation completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        };
        controller
            .execute_trash_operation(crate::action::TrashOperation::MoveToTrash { note_id })
            .expect("worker should accept the exact note to trash");

        loop {
            let _ = product.drain_product_events();
            let events = product.take_events();
            if events.iter().any(|event| {
                active_completion(event, &active_workspace).is_some_and(|completion| {
                    matches!(
                        completion,
                        WorkspaceCompletion::TrashOperationCompleted {
                            operation: crate::action::TrashOperation::MoveToTrash {
                                note_id: completed_note_id,
                            },
                        } if *completed_note_id == note_id
                    )
                })
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "trash completion should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!directory.path().join("无标题.md").exists());
    }
}
