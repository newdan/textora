//! 通过可恢复的领域命令操作工作区笔记文件。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::catalog::{NotePathOperation, NotePathOperationKind, NotePathOperationState};
use crate::workspace::move_file_no_replace;
use crate::{
    Catalog, CatalogError, CatalogNote, DEFAULT_NOTE_TITLE, DocumentKind, NoteEncryption,
    NoteFileNameBinding, NoteId, Workspace, WorkspaceError, allocate_title_bound_file_name,
    normalize_title_file_stem, parse_note_text_summary,
};

/// 创建时已经具备全部执行条件的互斥存储方式。
#[derive(Clone)]
pub enum CreateNoteStorage {
    Unencrypted,
    Encrypted { password: Arc<textora_encryption::EncryptionPassword> },
}

impl CreateNoteStorage {
    pub fn encrypted(password: textora_encryption::EncryptionPassword) -> Self {
        Self::Encrypted { password: Arc::new(password) }
    }

    pub fn encryption(&self) -> NoteEncryption {
        match self {
            Self::Unencrypted => NoteEncryption::Unencrypted,
            Self::Encrypted { .. } => NoteEncryption::Encrypted,
        }
    }

    pub fn password(&self) -> Option<&textora_encryption::EncryptionPassword> {
        match self {
            Self::Unencrypted => None,
            Self::Encrypted { password } => Some(password),
        }
    }
}

impl std::fmt::Debug for CreateNoteStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unencrypted => formatter.write_str("Unencrypted"),
            Self::Encrypted { .. } => formatter.write_str("Encrypted { password: <redacted> }"),
        }
    }
}

impl PartialEq for CreateNoteStorage {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unencrypted, Self::Unencrypted) => true,
            (Self::Encrypted { password: left }, Self::Encrypted { password: right }) => {
                Arc::ptr_eq(left, right)
            }
            _ => false,
        }
    }
}

impl Eq for CreateNoteStorage {}

/// 已显式确定文档类型、位置与持久化属性的新建请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredCreateNoteRequest {
    pub kind: DocumentKind,
    pub target_directory: Option<PathBuf>,
    pub storage: CreateNoteStorage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNoteTitleRequest {
    pub note_id: NoteId,
    pub expected_title_revision: u64,
    pub title: String,
}

/// 移动笔记时传入的工作区内目标目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveNoteRequest {
    pub note_id: NoteId,
    pub target_directory: PathBuf,
}

/// 所有会变更工作区文件的类型化命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteCommand {
    CreateConfigured(ConfiguredCreateNoteRequest),
    UpdateTitle(UpdateNoteTitleRequest),
    Move(MoveNoteRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteCommandOutcome {
    Created,
    TitleUpdated,
    Moved,
}

/// 创建结果携带的运行时访问状态；密钥字节不会因结果克隆而复制。
#[derive(Clone)]
pub enum CreatedNoteAccess {
    Unencrypted,
    Encrypted { session: Arc<textora_encryption::UnlockedNoteSession> },
}

impl std::fmt::Debug for CreatedNoteAccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unencrypted => formatter.write_str("Unencrypted"),
            Self::Encrypted { session } => {
                formatter.debug_struct("Encrypted").field("session", session).finish()
            }
        }
    }
}

impl PartialEq for CreatedNoteAccess {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unencrypted, Self::Unencrypted) => true,
            (Self::Encrypted { session: left }, Self::Encrypted { session: right }) => {
                Arc::ptr_eq(left, right)
            }
            _ => false,
        }
    }
}

impl Eq for CreatedNoteAccess {}

/// 成功执行文件命令后的稳定笔记状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteCommandResult {
    pub note: CatalogNote,
    pub previous_relative_path: Option<PathBuf>,
    pub outcome: NoteCommandOutcome,
    pub created_access: Option<CreatedNoteAccess>,
}

/// 兼容仅含新建命令时的公开返回类型；后续命令共用相同结果结构。
pub type CreateNoteResult = NoteCommandResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NotePathRecoveryReport {
    pub committed_operations: usize,
    pub rolled_back_operations: usize,
}

#[derive(Debug)]
pub enum NotePathRecoveryError {
    Catalog(CatalogError),
    Workspace(WorkspaceError),
    FileMove {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    AmbiguousOperation {
        operation_id: uuid::Uuid,
        source_relative_path: PathBuf,
        target_relative_path: PathBuf,
        reason: &'static str,
    },
}

impl std::fmt::Display for NotePathRecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(source) => write!(formatter, "catalog path recovery failed: {source}"),
            Self::Workspace(source) => write!(formatter, "invalid recovery path: {source}"),
            Self::FileMove { from, to, source } => write!(
                formatter,
                "could not roll back note path from {} to {}: {source}",
                from.display(),
                to.display()
            ),
            Self::AmbiguousOperation {
                operation_id,
                source_relative_path,
                target_relative_path,
                reason,
            } => write!(
                formatter,
                "note path operation {operation_id} requires manual recovery ({} -> {}): {reason}",
                source_relative_path.display(),
                target_relative_path.display()
            ),
        }
    }
}

impl std::error::Error for NotePathRecoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(source) => Some(source),
            Self::Workspace(source) => Some(source),
            Self::FileMove { source, .. } => Some(source),
            Self::AmbiguousOperation { .. } => None,
        }
    }
}

/// 文件与 catalog 不能形成跨系统原子事务时的明确失败状态。
#[derive(Debug)]
pub enum NoteCommandError {
    Catalog {
        source: CatalogError,
    },
    Workspace(WorkspaceError),
    TargetDirectoryMissing {
        path: PathBuf,
    },
    TargetDirectoryNotDirectory {
        path: PathBuf,
    },
    FileWrite {
        path: PathBuf,
        source: std::io::Error,
    },
    FileMetadata {
        path: PathBuf,
        source: std::io::Error,
    },
    CatalogAfterFileWrite {
        relative_path: PathBuf,
        source: CatalogError,
    },
    NoteNotFound {
        note_id: NoteId,
    },
    InvalidFileName {
        path: PathBuf,
    },
    FileMove {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    CatalogAfterFileMove {
        from_relative_path: PathBuf,
        to_relative_path: PathBuf,
        source: Box<CatalogError>,
    },
    AutomaticNameExhausted {
        directory: PathBuf,
    },
    StaleTitleRevision {
        note_id: NoteId,
        expected: u64,
        actual: u64,
    },
    MarkdownReferenceRead {
        path: PathBuf,
        source: std::io::Error,
    },
    MarkdownLinksWouldBreak {
        target_relative_path: PathBuf,
        source_relative_paths: Vec<PathBuf>,
    },
    Encryption {
        source: textora_encryption::EncryptionError,
    },
    EncryptedStorageRequiresMarkdown,
}

impl std::fmt::Display for NoteCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog { source } => {
                write!(formatter, "catalog command lookup failed: {source}")
            }
            Self::Workspace(source) => write!(formatter, "invalid note workspace target: {source}"),
            Self::TargetDirectoryMissing { path } => {
                write!(formatter, "note target directory does not exist: {}", path.display())
            }
            Self::TargetDirectoryNotDirectory { path } => {
                write!(formatter, "note target is not a directory: {}", path.display())
            }
            Self::FileWrite { path, source } => {
                write!(formatter, "could not create note file {}: {source}", path.display())
            }
            Self::FileMetadata { path, source } => {
                write!(
                    formatter,
                    "could not read created note metadata {}: {source}",
                    path.display()
                )
            }
            Self::CatalogAfterFileWrite { relative_path, source } => write!(
                formatter,
                "note file {} was created but catalog indexing failed; reconciliation can recover it: {source}",
                relative_path.display()
            ),
            Self::NoteNotFound { note_id } => {
                write!(formatter, "active note does not exist: {note_id}")
            }
            Self::InvalidFileName { path } => {
                write!(formatter, "note rename requires one plain file name: {}", path.display())
            }
            Self::FileMove { from, to, source } => write!(
                formatter,
                "could not move note file from {} to {}: {source}",
                from.display(),
                to.display()
            ),
            Self::CatalogAfterFileMove { from_relative_path, to_relative_path, source } => write!(
                formatter,
                "note file moved from {} to {} but catalog update failed; reconciliation can recover it: {source}",
                from_relative_path.display(),
                to_relative_path.display()
            ),
            Self::AutomaticNameExhausted { directory } => {
                write!(formatter, "no automatic note name is available in {}", directory.display())
            }
            Self::StaleTitleRevision { note_id, expected, actual } => write!(
                formatter,
                "title revision for {note_id} is stale: expected {expected}, actual {actual}"
            ),
            Self::MarkdownReferenceRead { path, source } => {
                write!(formatter, "could not verify Markdown links in {}: {source}", path.display())
            }
            Self::MarkdownLinksWouldBreak { target_relative_path, source_relative_paths } => {
                write!(
                    formatter,
                    "renaming {} would break Markdown links from: {}",
                    target_relative_path.display(),
                    source_relative_paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Self::Encryption { source } => {
                write!(formatter, "encrypted note creation failed: {source}")
            }
            Self::EncryptedStorageRequiresMarkdown => {
                formatter.write_str("encrypted note storage requires Markdown")
            }
        }
    }
}

impl std::error::Error for NoteCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog { source } => Some(source),
            Self::Workspace(source) => Some(source),
            Self::FileWrite { source, .. }
            | Self::FileMetadata { source, .. }
            | Self::FileMove { source, .. }
            | Self::MarkdownReferenceRead { source, .. } => Some(source),
            Self::CatalogAfterFileWrite { source, .. } => Some(source),
            Self::CatalogAfterFileMove { source, .. } => Some(source.as_ref()),
            Self::Encryption { source } => Some(source),
            Self::TargetDirectoryMissing { .. }
            | Self::TargetDirectoryNotDirectory { .. }
            | Self::NoteNotFound { .. }
            | Self::InvalidFileName { .. }
            | Self::AutomaticNameExhausted { .. }
            | Self::StaleTitleRevision { .. }
            | Self::MarkdownLinksWouldBreak { .. }
            | Self::EncryptedStorageRequiresMarkdown => None,
        }
    }
}

/// 执行单个领域命令；调用方负责将它放到产品的 I/O effect 边界之后。
pub fn execute_note_command(
    workspace: &Workspace,
    catalog: &Catalog,
    command: NoteCommand,
) -> Result<NoteCommandResult, NoteCommandError> {
    match command {
        NoteCommand::CreateConfigured(request) => {
            create_configured_note(workspace, catalog, request)
        }
        NoteCommand::UpdateTitle(request) => update_note_title(workspace, catalog, request),
        NoteCommand::Move(request) => move_note(workspace, catalog, request),
    }
}

/// 启动时恢复 SQLite 与文件系统之间未完成的路径操作；歧义状态绝不自动猜测。
pub fn recover_note_path_operations(
    workspace: &Workspace,
    catalog: &Catalog,
) -> Result<NotePathRecoveryReport, NotePathRecoveryError> {
    let operations =
        catalog.unfinished_note_path_operations().map_err(NotePathRecoveryError::Catalog)?;
    let mut report = NotePathRecoveryReport::default();
    for operation in operations {
        let note = catalog
            .active_note(operation.note_id)
            .map_err(NotePathRecoveryError::Catalog)?
            .ok_or_else(|| ambiguous_recovery_error(&operation, "对应的活动笔记不存在"))?;
        let source_path = workspace
            .resolve_relative_path(&operation.source_relative_path)
            .map_err(NotePathRecoveryError::Workspace)?;
        let target_path = workspace
            .resolve_relative_path(&operation.target_relative_path)
            .map_err(NotePathRecoveryError::Workspace)?;
        let source_exists = source_path.symlink_metadata().is_ok();
        let target_exists = target_path.symlink_metadata().is_ok();
        if note.relative_path == operation.source_relative_path {
            recover_catalog_at_source(
                catalog,
                &operation,
                &source_path,
                &target_path,
                source_exists,
                target_exists,
            )?;
            report.rolled_back_operations += 1;
            continue;
        }
        if note.relative_path == operation.target_relative_path {
            if source_exists || !target_exists {
                return Err(ambiguous_recovery_error(
                    &operation,
                    "Catalog 已指向目标，但磁盘源/目标状态不唯一",
                ));
            }
            catalog
                .update_note_path_operation_state(
                    operation.operation_id,
                    NotePathOperationState::Committed,
                )
                .map_err(NotePathRecoveryError::Catalog)?;
            report.committed_operations += 1;
            continue;
        }
        return Err(ambiguous_recovery_error(
            &operation,
            "Catalog 当前路径既不是操作源也不是操作目标",
        ));
    }
    Ok(report)
}

fn recover_catalog_at_source(
    catalog: &Catalog,
    operation: &NotePathOperation,
    source_path: &Path,
    target_path: &Path,
    source_exists: bool,
    target_exists: bool,
) -> Result<(), NotePathRecoveryError> {
    match (source_exists, target_exists) {
        (true, false) => {}
        (false, true) => move_file_no_replace(target_path, source_path).map_err(|source| {
            NotePathRecoveryError::FileMove {
                from: target_path.to_path_buf(),
                to: source_path.to_path_buf(),
                source,
            }
        })?,
        (true, true) => {
            return Err(ambiguous_recovery_error(operation, "源文件和目标文件同时存在"));
        }
        (false, false) => {
            return Err(ambiguous_recovery_error(operation, "源文件和目标文件都不存在"));
        }
    }
    catalog
        .update_note_path_operation_state(
            operation.operation_id,
            NotePathOperationState::RolledBack,
        )
        .map_err(NotePathRecoveryError::Catalog)
}

fn ambiguous_recovery_error(
    operation: &NotePathOperation,
    reason: &'static str,
) -> NotePathRecoveryError {
    NotePathRecoveryError::AmbiguousOperation {
        operation_id: operation.operation_id,
        source_relative_path: operation.source_relative_path.clone(),
        target_relative_path: operation.target_relative_path.clone(),
        reason,
    }
}

fn update_note_title(
    workspace: &Workspace,
    catalog: &Catalog,
    request: UpdateNoteTitleRequest,
) -> Result<NoteCommandResult, NoteCommandError> {
    let note = active_note(catalog, request.note_id)?;
    let metadata = catalog
        .note_file_name_metadata(request.note_id)
        .map_err(|source| NoteCommandError::Catalog { source })?
        .ok_or(NoteCommandError::NoteNotFound { note_id: request.note_id })?;
    if request.expected_title_revision != metadata.title_revision {
        return Err(stale_title_revision_error(&request, metadata.title_revision));
    }
    let title = normalized_note_title(&request.title);
    match metadata.binding {
        NoteFileNameBinding::LegacyUnmanaged | NoteFileNameBinding::Opaque => {
            commit_title_without_relocation(catalog, note, &request, title)
        }
        NoteFileNameBinding::TitleBound { .. } => {
            update_title_bound_note(workspace, catalog, note, &request, title)
        }
    }
}

fn update_title_bound_note(
    workspace: &Workspace,
    catalog: &Catalog,
    note: CatalogNote,
    request: &UpdateNoteTitleRequest,
    title: String,
) -> Result<NoteCommandResult, NoteCommandError> {
    let parent_directory = note
        .relative_path
        .parent()
        .ok_or_else(|| NoteCommandError::InvalidFileName { path: note.relative_path.clone() })?;
    let absolute_directory = if parent_directory.as_os_str().is_empty() {
        workspace.root().to_path_buf()
    } else {
        workspace.resolve_relative_path(parent_directory).map_err(NoteCommandError::Workspace)?
    };
    let catalog_paths = catalog
        .active_notes()
        .map_err(|source| NoteCommandError::Catalog { source })?
        .into_iter()
        .map(|catalog_note| catalog_note.relative_path)
        .collect::<Vec<_>>();
    let Some(allocation) = allocate_title_bound_file_name(
        &absolute_directory,
        parent_directory,
        &normalize_title_file_stem(&title),
        note.kind,
        Some(&note.relative_path),
        &catalog_paths,
    )
    .map_err(|source| NoteCommandError::FileMetadata {
        path: absolute_directory.clone(),
        source,
    })?
    else {
        return Err(NoteCommandError::AutomaticNameExhausted { directory: absolute_directory });
    };
    let target_relative_path = parent_directory.join(allocation.file_name);
    if target_relative_path == note.relative_path {
        return commit_title_bound_without_relocation(
            catalog,
            note,
            request,
            title,
            allocation.disambiguator,
        );
    }
    ensure_no_markdown_links_would_break(workspace, catalog, &note.relative_path)?;
    relocate_title_bound_note(
        workspace,
        catalog,
        note,
        request,
        title,
        target_relative_path,
        allocation.disambiguator,
    )
}

fn ensure_no_markdown_links_would_break(
    workspace: &Workspace,
    catalog: &Catalog,
    target_relative_path: &Path,
) -> Result<(), NoteCommandError> {
    let mut source_relative_paths = Vec::new();
    for source_note in
        catalog.active_notes().map_err(|source| NoteCommandError::Catalog { source })?
    {
        if !matches!(source_note.kind, DocumentKind::Markdown | DocumentKind::Mindmap) {
            continue;
        }
        let source_path = workspace
            .resolve_relative_path(&source_note.relative_path)
            .map_err(NoteCommandError::Workspace)?;
        let markdown = fs::read_to_string(&source_path).map_err(|source| {
            NoteCommandError::MarkdownReferenceRead { path: source_path, source }
        })?;
        if crate::extract_markdown_path_references(&source_note.relative_path, &markdown)
            .iter()
            .any(|reference| reference.target_relative_path == target_relative_path)
        {
            source_relative_paths.push(source_note.relative_path);
        }
    }
    source_relative_paths.sort();
    source_relative_paths.dedup();
    if source_relative_paths.is_empty() {
        return Ok(());
    }
    Err(NoteCommandError::MarkdownLinksWouldBreak {
        target_relative_path: target_relative_path.to_path_buf(),
        source_relative_paths,
    })
}

fn commit_title_without_relocation(
    catalog: &Catalog,
    mut note: CatalogNote,
    request: &UpdateNoteTitleRequest,
    title: String,
) -> Result<NoteCommandResult, NoteCommandError> {
    let revision = catalog
        .commit_title_metadata(note.note_id, request.expected_title_revision, &title)
        .map_err(|source| NoteCommandError::Catalog { source })?;
    if revision.is_none() {
        return Err(current_stale_title_revision_error(catalog, request)?);
    }
    note.title = title;
    Ok(NoteCommandResult {
        note,
        previous_relative_path: None,
        outcome: NoteCommandOutcome::TitleUpdated,
        created_access: None,
    })
}

fn commit_title_bound_without_relocation(
    catalog: &Catalog,
    mut note: CatalogNote,
    request: &UpdateNoteTitleRequest,
    title: String,
    disambiguator: u32,
) -> Result<NoteCommandResult, NoteCommandError> {
    let revision = catalog
        .commit_title_bound_path(
            note.note_id,
            request.expected_title_revision,
            &title,
            &note.relative_path,
            disambiguator,
        )
        .map_err(|source| NoteCommandError::Catalog { source })?;
    if revision.is_none() {
        return Err(current_stale_title_revision_error(catalog, request)?);
    }
    note.title = title;
    Ok(NoteCommandResult {
        note,
        previous_relative_path: None,
        outcome: NoteCommandOutcome::TitleUpdated,
        created_access: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn relocate_title_bound_note(
    workspace: &Workspace,
    catalog: &Catalog,
    mut note: CatalogNote,
    request: &UpdateNoteTitleRequest,
    title: String,
    target_relative_path: PathBuf,
    disambiguator: u32,
) -> Result<NoteCommandResult, NoteCommandError> {
    let source_relative_path = note.relative_path.clone();
    let source_path = workspace
        .resolve_relative_path(&source_relative_path)
        .map_err(NoteCommandError::Workspace)?;
    let target_path = workspace
        .resolve_relative_path(&target_relative_path)
        .map_err(NoteCommandError::Workspace)?;
    let operation = NotePathOperation {
        operation_id: uuid::Uuid::new_v4(),
        note_id: note.note_id,
        kind: NotePathOperationKind::TitleRename,
        source_relative_path: source_relative_path.clone(),
        target_relative_path: target_relative_path.clone(),
        expected_title_revision: request.expected_title_revision,
        state: NotePathOperationState::Prepared,
    };
    catalog
        .prepare_note_path_operation(&operation)
        .map_err(|source| NoteCommandError::Catalog { source })?;
    if let Err(source) = move_file_no_replace(&source_path, &target_path) {
        catalog
            .update_note_path_operation_state(
                operation.operation_id,
                NotePathOperationState::RolledBack,
            )
            .map_err(|source| NoteCommandError::Catalog { source })?;
        return Err(NoteCommandError::FileMove { from: source_path, to: target_path, source });
    }
    if let Err(source) = catalog
        .update_note_path_operation_state(operation.operation_id, NotePathOperationState::Moved)
    {
        rollback_title_relocation(catalog, &operation, &source_path, &target_path)?;
        return Err(NoteCommandError::Catalog { source });
    }
    let commit_result = catalog.commit_title_bound_path(
        note.note_id,
        request.expected_title_revision,
        &title,
        &target_relative_path,
        disambiguator,
    );
    match commit_result {
        Ok(Some(_)) => {}
        Ok(None) => {
            rollback_title_relocation(catalog, &operation, &source_path, &target_path)?;
            return Err(current_stale_title_revision_error(catalog, request)?);
        }
        Err(source) => {
            rollback_title_relocation(catalog, &operation, &source_path, &target_path)?;
            return Err(NoteCommandError::CatalogAfterFileMove {
                from_relative_path: source_relative_path,
                to_relative_path: target_relative_path,
                source: Box::new(source),
            });
        }
    }
    catalog
        .update_note_path_operation_state(operation.operation_id, NotePathOperationState::Committed)
        .map_err(|source| NoteCommandError::Catalog { source })?;
    note.title = title;
    note.relative_path = target_relative_path;
    Ok(NoteCommandResult {
        note,
        previous_relative_path: Some(source_relative_path),
        outcome: NoteCommandOutcome::TitleUpdated,
        created_access: None,
    })
}

fn rollback_title_relocation(
    catalog: &Catalog,
    operation: &NotePathOperation,
    source_path: &Path,
    target_path: &Path,
) -> Result<(), NoteCommandError> {
    move_file_no_replace(target_path, source_path).map_err(|source| {
        NoteCommandError::FileMove {
            from: target_path.to_path_buf(),
            to: source_path.to_path_buf(),
            source,
        }
    })?;
    catalog
        .update_note_path_operation_state(
            operation.operation_id,
            NotePathOperationState::RolledBack,
        )
        .map_err(|source| NoteCommandError::Catalog { source })
}

fn current_stale_title_revision_error(
    catalog: &Catalog,
    request: &UpdateNoteTitleRequest,
) -> Result<NoteCommandError, NoteCommandError> {
    let metadata = catalog
        .note_file_name_metadata(request.note_id)
        .map_err(|source| NoteCommandError::Catalog { source })?
        .ok_or(NoteCommandError::NoteNotFound { note_id: request.note_id })?;
    Ok(stale_title_revision_error(request, metadata.title_revision))
}

fn stale_title_revision_error(request: &UpdateNoteTitleRequest, actual: u64) -> NoteCommandError {
    NoteCommandError::StaleTitleRevision {
        note_id: request.note_id,
        expected: request.expected_title_revision,
        actual,
    }
}

fn normalized_note_title(title: &str) -> String {
    let normalized = title.trim();
    if normalized.is_empty() { DEFAULT_NOTE_TITLE.to_owned() } else { normalized.to_owned() }
}

fn create_configured_note(
    workspace: &Workspace,
    catalog: &Catalog,
    request: ConfiguredCreateNoteRequest,
) -> Result<NoteCommandResult, NoteCommandError> {
    let target_directory =
        resolve_target_directory(workspace, request.target_directory.as_deref())?;
    let prepared_contents = prepare_note_contents(request.kind, request.storage)?;
    loop {
        let catalog_relative_paths = catalog
            .active_notes()
            .map_err(|source| NoteCommandError::Catalog { source })?
            .into_iter()
            .map(|note| note.relative_path)
            .collect::<Vec<_>>();
        let Some(allocation) = allocate_title_bound_file_name(
            &target_directory.absolute_path,
            &target_directory.relative_path,
            DEFAULT_NOTE_TITLE,
            request.kind,
            None,
            &catalog_relative_paths,
        )
        .map_err(|source| NoteCommandError::FileMetadata {
            path: target_directory.absolute_path.clone(),
            source,
        })?
        else {
            return Err(NoteCommandError::AutomaticNameExhausted {
                directory: target_directory.absolute_path,
            });
        };
        let file_name = allocation.file_name;
        let relative_path = target_directory.relative_path.join(&file_name);
        let absolute_path =
            workspace.resolve_relative_path(&relative_path).map_err(NoteCommandError::Workspace)?;
        match create_note_file(&absolute_path, prepared_contents.serialized()) {
            Ok(()) => {
                let note = created_catalog_note(
                    request.kind,
                    relative_path.clone(),
                    DEFAULT_NOTE_TITLE,
                    &prepared_contents,
                    &absolute_path,
                )?;
                if let Err(source) = catalog.create_active_note(
                    &note,
                    prepared_contents.encryption(),
                    prepared_contents.title_initialization(request.kind),
                ) {
                    return Err(catalog_creation_error(
                        &absolute_path,
                        relative_path,
                        &prepared_contents,
                        source,
                    ));
                }
                return Ok(NoteCommandResult {
                    note,
                    previous_relative_path: None,
                    outcome: NoteCommandOutcome::Created,
                    created_access: Some(prepared_contents.created_access()),
                });
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(NoteCommandError::FileWrite { path: absolute_path, source });
            }
        }
    }
}

fn move_note(
    workspace: &Workspace,
    catalog: &Catalog,
    request: MoveNoteRequest,
) -> Result<NoteCommandResult, NoteCommandError> {
    let note = active_note(catalog, request.note_id)?;
    let target_directory = resolve_target_directory(workspace, Some(&request.target_directory))?;
    let file_name = note
        .relative_path
        .file_name()
        .ok_or_else(|| NoteCommandError::InvalidFileName { path: note.relative_path.clone() })?;
    let target_relative_path = target_directory.relative_path.join(file_name);
    relocate_note(workspace, catalog, note, target_relative_path)
}

fn active_note(catalog: &Catalog, note_id: NoteId) -> Result<CatalogNote, NoteCommandError> {
    catalog
        .active_note(note_id)
        .map_err(|source| NoteCommandError::Catalog { source })?
        .ok_or(NoteCommandError::NoteNotFound { note_id })
}

fn relocate_note(
    workspace: &Workspace,
    catalog: &Catalog,
    mut note: CatalogNote,
    target_relative_path: PathBuf,
) -> Result<NoteCommandResult, NoteCommandError> {
    let source_relative_path = note.relative_path.clone();
    if source_relative_path == target_relative_path {
        return Ok(NoteCommandResult {
            note,
            previous_relative_path: None,
            outcome: NoteCommandOutcome::Moved,
            created_access: None,
        });
    }
    let source_path = workspace
        .resolve_relative_path(&source_relative_path)
        .map_err(NoteCommandError::Workspace)?;
    let target_path = workspace
        .resolve_relative_path(&target_relative_path)
        .map_err(NoteCommandError::Workspace)?;
    move_file_no_replace(&source_path, &target_path).map_err(|source| {
        NoteCommandError::FileMove { from: source_path, to: target_path, source }
    })?;
    catalog.update_active_note_path(note.note_id, &target_relative_path).map_err(|source| {
        NoteCommandError::CatalogAfterFileMove {
            from_relative_path: source_relative_path.clone(),
            to_relative_path: target_relative_path.clone(),
            source: Box::new(source),
        }
    })?;
    note.relative_path = target_relative_path;
    Ok(NoteCommandResult {
        note,
        previous_relative_path: Some(source_relative_path),
        outcome: NoteCommandOutcome::Moved,
        created_access: None,
    })
}

struct TargetDirectory {
    relative_path: PathBuf,
    absolute_path: PathBuf,
}

fn resolve_target_directory(
    workspace: &Workspace,
    requested_directory: Option<&Path>,
) -> Result<TargetDirectory, NoteCommandError> {
    let relative_path = requested_directory.map_or_else(PathBuf::new, Path::to_path_buf);
    let absolute_path = match requested_directory {
        Some(directory) => {
            workspace.resolve_relative_path(directory).map_err(NoteCommandError::Workspace)?
        }
        None => workspace.root().to_path_buf(),
    };
    let metadata = fs::metadata(&absolute_path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => {
            NoteCommandError::TargetDirectoryMissing { path: absolute_path.clone() }
        }
        _ => NoteCommandError::FileMetadata { path: absolute_path.clone(), source },
    })?;
    if !metadata.is_dir() {
        return Err(NoteCommandError::TargetDirectoryNotDirectory { path: absolute_path });
    }
    Ok(TargetDirectory { relative_path, absolute_path })
}

fn initial_contents(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text | DocumentKind::Markdown => "",
        DocumentKind::Mindmap => "#",
    }
}

fn create_note_file(path: &Path, contents: &[u8]) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

enum PreparedNoteContents {
    Unencrypted { plaintext: &'static str },
    Encrypted { serialized: Vec<u8>, session: Arc<textora_encryption::UnlockedNoteSession> },
}

impl PreparedNoteContents {
    fn serialized(&self) -> &[u8] {
        match self {
            Self::Unencrypted { plaintext } => plaintext.as_bytes(),
            Self::Encrypted { serialized, .. } => serialized,
        }
    }

    fn encryption(&self) -> NoteEncryption {
        match self {
            Self::Unencrypted { .. } => NoteEncryption::Unencrypted,
            Self::Encrypted { .. } => NoteEncryption::Encrypted,
        }
    }

    fn title_initialization(&self, kind: DocumentKind) -> crate::TitleInitialization {
        match self {
            Self::Encrypted { .. } => crate::TitleInitialization::Independent,
            Self::Unencrypted { .. } if kind == DocumentKind::Text => {
                crate::TitleInitialization::Independent
            }
            Self::Unencrypted { .. } => crate::TitleInitialization::AwaitingFirstCommit,
        }
    }

    fn created_access(&self) -> CreatedNoteAccess {
        match self {
            Self::Unencrypted { .. } => CreatedNoteAccess::Unencrypted,
            Self::Encrypted { session, .. } => {
                CreatedNoteAccess::Encrypted { session: Arc::clone(session) }
            }
        }
    }
}

fn prepare_note_contents(
    kind: DocumentKind,
    storage: CreateNoteStorage,
) -> Result<PreparedNoteContents, NoteCommandError> {
    match storage {
        CreateNoteStorage::Unencrypted => {
            Ok(PreparedNoteContents::Unencrypted { plaintext: initial_contents(kind) })
        }
        CreateNoteStorage::Encrypted { password } => {
            if kind != DocumentKind::Markdown {
                return Err(NoteCommandError::EncryptedStorageRequiresMarkdown);
            }
            let created = textora_encryption::create_encrypted_markdown(
                password.as_ref(),
                initial_contents(kind).as_bytes(),
            )
            .map_err(|source| NoteCommandError::Encryption { source })?;
            let (serialized, session) = created.into_parts();
            Ok(PreparedNoteContents::Encrypted { serialized, session: Arc::new(session) })
        }
    }
}

fn catalog_creation_error(
    absolute_path: &Path,
    relative_path: PathBuf,
    contents: &PreparedNoteContents,
    source: CatalogError,
) -> NoteCommandError {
    let PreparedNoteContents::Encrypted { session, .. } = contents else {
        return NoteCommandError::CatalogAfterFileWrite { relative_path, source };
    };
    if remove_matching_encrypted_file(absolute_path, session.document_id()).is_ok() {
        return NoteCommandError::Catalog { source };
    }
    NoteCommandError::CatalogAfterFileWrite { relative_path, source }
}

fn remove_matching_encrypted_file(path: &Path, document_id: uuid::Uuid) -> std::io::Result<()> {
    let serialized = fs::read(path)?;
    let header = textora_encryption::inspect_encrypted_markdown(&serialized)
        .map_err(std::io::Error::other)?;
    if header.document_id != document_id {
        return Err(std::io::Error::other(
            "encrypted creation rollback identity no longer matches",
        ));
    }
    fs::remove_file(path)
}

fn created_catalog_note(
    kind: DocumentKind,
    relative_path: PathBuf,
    title: &str,
    contents: &PreparedNoteContents,
    absolute_path: &Path,
) -> Result<CatalogNote, NoteCommandError> {
    let metadata = fs::metadata(absolute_path).map_err(|source| {
        NoteCommandError::FileMetadata { path: absolute_path.to_path_buf(), source }
    })?;
    let (stored_title, excerpt) = match contents {
        PreparedNoteContents::Unencrypted { plaintext } => {
            let summary = parse_note_text_summary(kind, title, plaintext);
            (summary.title, summary.excerpt)
        }
        PreparedNoteContents::Encrypted { .. } => (title.to_owned(), String::new()),
    };
    Ok(CatalogNote {
        note_id: NoteId::generate(),
        relative_path,
        kind,
        title: stored_title,
        excerpt,
        modified_at: metadata.modified().map_err(|source| NoteCommandError::FileMetadata {
            path: absolute_path.to_path_buf(),
            source,
        })?,
        file_size: metadata.len(),
        content_hash: blake3::hash(contents.serialized()).as_bytes().to_vec(),
        starred: false,
    })
}

#[cfg(test)]
mod create {
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use crate::{Catalog, DocumentKind, Workspace};

    use super::{
        ConfiguredCreateNoteRequest, CreateNoteStorage, NoteCommand, execute_note_command,
    };

    #[test]
    fn create_places_each_kind_in_the_requested_workspace_directory() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(directory.path().join("nested")).expect("nested fixture should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        let markdown = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                storage: CreateNoteStorage::Unencrypted,
            }),
        )
        .expect("markdown note should be created");
        let mindmap = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Mindmap,
                target_directory: Some("nested".into()),
                storage: CreateNoteStorage::Unencrypted,
            }),
        )
        .expect("mindmap note should be created");

        assert_eq!(markdown.note.relative_path, std::path::PathBuf::from("无标题.md"));
        assert_eq!(mindmap.note.relative_path, std::path::PathBuf::from("nested/无标题.mmap.md"));
        assert_eq!(markdown.note.title, "无标题");
        assert_eq!(mindmap.note.title, "无标题");
        assert_eq!(
            fs::read_to_string(directory.path().join(&mindmap.note.relative_path))
                .expect("mindmap note should be readable"),
            "#"
        );
        assert_eq!(catalog.active_notes().expect("created notes should be indexed").len(), 2);
        for note in [&markdown.note, &mindmap.note] {
            assert_eq!(
                catalog
                    .note_editor_metadata(note.note_id)
                    .expect("created metadata should query")
                    .expect("created metadata should exist")
                    .title_initialization,
                crate::TitleInitialization::AwaitingFirstCommit
            );
        }
    }

    #[test]
    fn concurrent_creation_never_overwrites_an_existing_untitled_file() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let root = workspace.root().to_path_buf();
        let barrier = Arc::new(Barrier::new(2));
        let worker_inputs = (0..2)
            .map(|_| {
                let catalog =
                    Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
                        .expect("catalog should open before concurrent command execution");
                (workspace.clone(), catalog)
            })
            .collect::<Vec<_>>();
        let workers = worker_inputs
            .into_iter()
            .map(|(workspace, catalog)| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    execute_note_command(
                        &workspace,
                        &catalog,
                        NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                            kind: DocumentKind::Markdown,
                            target_directory: None,
                            storage: CreateNoteStorage::Unencrypted,
                        }),
                    )
                    .expect("concurrent note creation should choose a free name")
                })
            })
            .collect::<Vec<_>>();
        let created = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker should not panic"))
            .collect::<Vec<_>>();

        assert_ne!(created[0].note.relative_path, created[1].note.relative_path);
        assert!(root.join("无标题.md").is_file());
        assert!(root.join("无标题 (2).md").is_file());
    }

    #[test]
    fn create_rejects_the_reserved_metadata_directory() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        assert!(
            execute_note_command(
                &workspace,
                &catalog,
                NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                    kind: DocumentKind::Markdown,
                    target_directory: Some(".notora".into()),
                    storage: CreateNoteStorage::Unencrypted,
                }),
            )
            .is_err()
        );
    }

    #[test]
    fn encrypted_creation_writes_ciphertext_and_returns_an_unlocked_session() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let request = ConfiguredCreateNoteRequest {
            kind: DocumentKind::Markdown,
            target_directory: None,
            storage: CreateNoteStorage::encrypted(encryption_password()),
        };
        assert!(!format!("{request:?}").contains("test-password"));

        let result =
            execute_note_command(&workspace, &catalog, NoteCommand::CreateConfigured(request))
                .expect("encrypted note should be created");
        let note_path = directory.path().join(&result.note.relative_path);
        let serialized = fs::read(&note_path).expect("encrypted note should be readable");
        let header = textora_encryption::inspect_encrypted_markdown(&serialized)
            .expect("created file should be an encrypted envelope");
        let unlocked =
            textora_encryption::unlock_encrypted_markdown(&serialized, &encryption_password())
                .expect("created note should unlock with its password");
        let metadata = catalog
            .note_editor_metadata(result.note.note_id)
            .expect("created metadata should query")
            .expect("created metadata should exist");

        assert_eq!(result.note.relative_path, std::path::PathBuf::from("无标题.md"));
        assert_eq!(result.note.excerpt, "");
        assert_eq!(unlocked.plaintext(), "");
        assert_eq!(metadata.encryption, crate::NoteEncryption::Encrypted);
        assert_eq!(metadata.title_initialization, crate::TitleInitialization::Independent);
        assert_eq!(
            metadata.file_name_binding,
            crate::NoteFileNameBinding::TitleBound { disambiguator: 1 }
        );
        assert!(matches!(
            result.created_access,
            Some(super::CreatedNoteAccess::Encrypted { session })
                if session.document_id() == header.document_id
        ));
    }

    #[test]
    fn encrypted_storage_rejects_non_markdown_without_creating_a_file() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        let result = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Text,
                target_directory: None,
                storage: CreateNoteStorage::encrypted(encryption_password()),
            }),
        );

        assert!(matches!(result, Err(super::NoteCommandError::EncryptedStorageRequiresMarkdown)));
        assert!(!directory.path().join("无标题.txt").exists());
    }

    #[test]
    fn encrypted_creation_removes_its_file_when_catalog_insert_fails() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog_path = workspace.metadata_directory().join("catalog.sqlite3");
        let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");
        let trigger_connection =
            rusqlite::Connection::open(&catalog_path).expect("trigger connection should open");
        trigger_connection
            .execute_batch(
                "CREATE TRIGGER reject_encrypted_creation
                 BEFORE INSERT ON notes
                 BEGIN
                     SELECT RAISE(ABORT, 'injected catalog failure');
                 END;",
            )
            .expect("catalog failure trigger should install");
        drop(trigger_connection);

        let result = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                storage: CreateNoteStorage::encrypted(encryption_password()),
            }),
        );

        assert!(matches!(result, Err(super::NoteCommandError::Catalog { .. })));
        assert!(!directory.path().join("无标题.md").exists());
        assert!(catalog.active_notes().expect("catalog should remain readable").is_empty());
    }

    #[test]
    fn create_reserves_a_catalog_path_even_when_its_file_is_temporarily_missing() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let request = ConfiguredCreateNoteRequest {
            kind: DocumentKind::Markdown,
            target_directory: None,
            storage: CreateNoteStorage::Unencrypted,
        };
        let first = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(request.clone()),
        )
        .expect("first note should be created");
        fs::remove_file(directory.path().join(&first.note.relative_path))
            .expect("missing-file fixture should remove only the entity");

        let second =
            execute_note_command(&workspace, &catalog, NoteCommand::CreateConfigured(request))
                .expect("second note should avoid the catalog reservation");

        assert_eq!(second.note.relative_path, std::path::PathBuf::from("无标题 (2).md"));
    }

    fn encryption_password() -> textora_encryption::EncryptionPassword {
        textora_encryption::EncryptionPassword::new("test-password".to_owned())
            .expect("test password should satisfy policy")
    }
}

#[cfg(test)]
mod move_note {
    use std::fs;

    use crate::{Catalog, DocumentKind, Workspace};

    use super::{
        ConfiguredCreateNoteRequest, CreateNoteStorage, MoveNoteRequest, NoteCommand,
        execute_note_command,
    };

    #[test]
    fn move_keeps_the_note_id_and_updates_its_relative_path() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(directory.path().join("archive"))
            .expect("archive fixture should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Mindmap,
                target_directory: None,
                storage: CreateNoteStorage::Unencrypted,
            }),
        )
        .expect("note fixture should be created");

        let moved = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::Move(MoveNoteRequest {
                note_id: created.note.note_id,
                target_directory: "archive".into(),
            }),
        )
        .expect("move should succeed");

        assert_eq!(moved.note.note_id, created.note.note_id);
        assert_eq!(moved.note.relative_path, std::path::PathBuf::from("archive/无标题.mmap.md"));
        assert!(directory.path().join(&moved.note.relative_path).is_file());
    }

    #[test]
    fn move_rejects_reserved_or_missing_destinations_without_moving_the_source() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(directory.path().join("occupied"))
            .expect("occupied directory fixture should be created");
        fs::write(directory.path().join("occupied/无标题.txt"), "do not replace")
            .expect("occupied note fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Text,
                target_directory: None,
                storage: CreateNoteStorage::Unencrypted,
            }),
        )
        .expect("note fixture should be created");

        for target_directory in [".notora", "does-not-exist", "occupied"] {
            assert!(
                execute_note_command(
                    &workspace,
                    &catalog,
                    NoteCommand::Move(MoveNoteRequest {
                        note_id: created.note.note_id,
                        target_directory: target_directory.into(),
                    }),
                )
                .is_err()
            );
        }
        assert!(directory.path().join("无标题.txt").is_file());
        assert_eq!(
            fs::read_to_string(directory.path().join("occupied/无标题.txt"))
                .expect("occupied target should remain readable"),
            "do not replace"
        );
    }
}

#[cfg(test)]
mod update_title {
    use std::fs;

    use crate::{Catalog, DocumentKind, Workspace};

    use super::{
        ConfiguredCreateNoteRequest, CreateNoteStorage, NoteCommand, UpdateNoteTitleRequest,
        execute_note_command,
    };

    fn create_markdown(workspace: &Workspace, catalog: &Catalog) -> super::NoteCommandResult {
        execute_note_command(
            workspace,
            catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                storage: CreateNoteStorage::Unencrypted,
            }),
        )
        .expect("markdown fixture should be created")
    }

    #[test]
    fn title_updates_rename_files_and_allocate_stable_duplicate_suffixes() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let first = create_markdown(&workspace, &catalog);
        let second = create_markdown(&workspace, &catalog);

        let first_updated = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                note_id: first.note.note_id,
                expected_title_revision: 0,
                title: "项目计划".to_owned(),
            }),
        )
        .expect("first title should update");
        let second_updated = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                note_id: second.note.note_id,
                expected_title_revision: 0,
                title: "项目计划".to_owned(),
            }),
        )
        .expect("duplicate title should update");

        assert_eq!(first_updated.note.note_id, first.note.note_id);
        assert_eq!(first_updated.note.title, "项目计划");
        assert_eq!(first_updated.note.relative_path, std::path::PathBuf::from("项目计划.md"));
        assert_eq!(second_updated.note.title, "项目计划");
        assert_eq!(second_updated.note.relative_path, std::path::PathBuf::from("项目计划 (2).md"));
        assert!(directory.path().join("项目计划.md").is_file());
        assert!(directory.path().join("项目计划 (2).md").is_file());
    }

    #[test]
    fn equivalent_file_stems_only_update_title_metadata_and_stale_revision_is_rejected() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = create_markdown(&workspace, &catalog);
        let first = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                note_id: created.note.note_id,
                expected_title_revision: 0,
                title: "A/B".to_owned(),
            }),
        )
        .expect("first title should update");
        let equivalent = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                note_id: created.note.note_id,
                expected_title_revision: 1,
                title: "A:B".to_owned(),
            }),
        )
        .expect("equivalent stem should update without moving");

        assert_eq!(first.note.relative_path, std::path::PathBuf::from("A B.md"));
        assert_eq!(equivalent.note.relative_path, first.note.relative_path);
        assert_eq!(equivalent.previous_relative_path, None);
        assert_eq!(equivalent.note.title, "A:B");
        assert!(matches!(
            execute_note_command(
                &workspace,
                &catalog,
                NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                    note_id: created.note.note_id,
                    expected_title_revision: 1,
                    title: "Stale".to_owned(),
                }),
            ),
            Err(super::NoteCommandError::StaleTitleRevision { actual: 2, .. })
        ));
        assert!(directory.path().join("A B.md").is_file());
        assert!(!directory.path().join("Stale.md").exists());
        assert_eq!(
            fs::read_to_string(directory.path().join("A B.md"))
                .expect("unchanged note should remain readable"),
            ""
        );
    }

    #[test]
    fn title_rename_is_blocked_when_a_parsed_markdown_link_would_break() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let target = create_markdown(&workspace, &catalog);
        let source = create_markdown(&workspace, &catalog);
        fs::write(workspace.root().join(&source.note.relative_path), "[target](无标题.md)")
            .expect("link source should be written");

        let result = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                note_id: target.note.note_id,
                expected_title_revision: 0,
                title: "新标题".to_owned(),
            }),
        );

        assert!(matches!(
            result,
            Err(super::NoteCommandError::MarkdownLinksWouldBreak {
                target_relative_path,
                source_relative_paths,
            }) if target_relative_path == std::path::Path::new("无标题.md")
                && source_relative_paths == vec![source.note.relative_path]
        ));
        assert!(workspace.root().join("无标题.md").is_file());
        assert!(!workspace.root().join("新标题.md").exists());
    }

    #[test]
    fn markdown_syntax_inside_code_does_not_block_title_rename() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let target = create_markdown(&workspace, &catalog);
        let source = create_markdown(&workspace, &catalog);
        fs::write(workspace.root().join(&source.note.relative_path), "`[example](无标题.md)`")
            .expect("code fixture should be written");

        let result = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::UpdateTitle(UpdateNoteTitleRequest {
                note_id: target.note.note_id,
                expected_title_revision: 0,
                title: "新标题".to_owned(),
            }),
        )
        .expect("code syntax should not be treated as a link");

        assert_eq!(result.note.relative_path, std::path::Path::new("新标题.md"));
        assert!(workspace.root().join("新标题.md").is_file());
    }
}

#[cfg(test)]
mod path_recovery {
    use std::fs;

    use crate::catalog::{NotePathOperation, NotePathOperationKind, NotePathOperationState};
    use crate::{Catalog, DocumentKind, Workspace};

    use super::{
        ConfiguredCreateNoteRequest, CreateNoteStorage, NoteCommand, NotePathRecoveryError,
        execute_note_command, recover_note_path_operations,
    };

    fn create_markdown(workspace: &Workspace, catalog: &Catalog) -> super::NoteCommandResult {
        execute_note_command(
            workspace,
            catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                storage: CreateNoteStorage::Unencrypted,
            }),
        )
        .expect("recovery fixture should be created")
    }

    fn prepare_operation(
        catalog: &Catalog,
        note_id: crate::NoteId,
        source: &str,
        target: &str,
        state: NotePathOperationState,
    ) -> NotePathOperation {
        let operation = NotePathOperation {
            operation_id: uuid::Uuid::new_v4(),
            note_id,
            kind: NotePathOperationKind::TitleRename,
            source_relative_path: source.into(),
            target_relative_path: target.into(),
            expected_title_revision: 0,
            state: NotePathOperationState::Prepared,
        };
        catalog.prepare_note_path_operation(&operation).expect("recovery operation should prepare");
        if state == NotePathOperationState::Moved {
            catalog
                .update_note_path_operation_state(operation.operation_id, state)
                .expect("recovery operation should become moved");
        }
        operation
    }

    #[test]
    fn startup_recovery_rolls_back_a_file_moved_before_catalog_commit() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = create_markdown(&workspace, &catalog);
        prepare_operation(
            &catalog,
            created.note.note_id,
            "无标题.md",
            "恢复目标.md",
            NotePathOperationState::Moved,
        );
        fs::rename(workspace.root().join("无标题.md"), workspace.root().join("恢复目标.md"))
            .expect("fixture should stop after the file move");

        let report = recover_note_path_operations(&workspace, &catalog)
            .expect("uncommitted move should roll back");

        assert_eq!(report.rolled_back_operations, 1);
        assert!(workspace.root().join("无标题.md").is_file());
        assert!(!workspace.root().join("恢复目标.md").exists());
        assert!(
            catalog
                .unfinished_note_path_operations()
                .expect("unfinished operations should query")
                .is_empty()
        );
    }

    #[test]
    fn startup_recovery_confirms_catalog_commit_before_operation_state_update() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = create_markdown(&workspace, &catalog);
        prepare_operation(
            &catalog,
            created.note.note_id,
            "无标题.md",
            "已提交.md",
            NotePathOperationState::Moved,
        );
        fs::rename(workspace.root().join("无标题.md"), workspace.root().join("已提交.md"))
            .expect("fixture should move the file");
        catalog
            .update_active_note_path(created.note.note_id, std::path::Path::new("已提交.md"))
            .expect("fixture should commit the catalog path");

        let report = recover_note_path_operations(&workspace, &catalog)
            .expect("committed move should be confirmed");

        assert_eq!(report.committed_operations, 1);
        assert!(workspace.root().join("已提交.md").is_file());
        assert!(
            catalog
                .unfinished_note_path_operations()
                .expect("unfinished operations should query")
                .is_empty()
        );
    }

    #[test]
    fn startup_recovery_refuses_to_choose_when_source_and_target_both_exist() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = create_markdown(&workspace, &catalog);
        prepare_operation(
            &catalog,
            created.note.note_id,
            "无标题.md",
            "冲突.md",
            NotePathOperationState::Moved,
        );
        fs::write(workspace.root().join("冲突.md"), "conflict")
            .expect("conflicting target should be written");

        assert!(matches!(
            recover_note_path_operations(&workspace, &catalog),
            Err(NotePathRecoveryError::AmbiguousOperation { .. })
        ));
        assert!(workspace.root().join("无标题.md").is_file());
        assert!(workspace.root().join("冲突.md").is_file());
        assert_eq!(
            catalog
                .unfinished_note_path_operations()
                .expect("ambiguous operation should remain")
                .len(),
            1
        );
    }
}
