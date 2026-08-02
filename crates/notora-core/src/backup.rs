use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags};

use crate::Catalog;

const BACKUP_FILE_EXTENSION: &str = "sqlite3";
const BACKUP_STEP_PAGE_COUNT: i32 = 64;
const BACKUP_STEP_PAUSE: Duration = Duration::from_millis(10);
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// catalog 备份的保留策略；始终至少保留一份已完成备份。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupRetention {
    maximum_backups: NonZeroUsize,
}

impl BackupRetention {
    pub fn keep_latest(maximum_backups: usize) -> Option<Self> {
        NonZeroUsize::new(maximum_backups).map(|maximum_backups| Self { maximum_backups })
    }

    fn maximum_backups(self) -> usize {
        self.maximum_backups.get()
    }
}

/// SQLite 在线备份或备份目录维护失败。
#[derive(Debug)]
pub enum CatalogBackupError {
    Io { operation: &'static str, path: PathBuf, source: std::io::Error },
    Sql { operation: &'static str, source: rusqlite::Error },
    InvalidSystemTime,
    InvalidBackup { path: PathBuf },
}

impl std::fmt::Display for CatalogBackupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { operation, path, source } => {
                write!(
                    formatter,
                    "catalog backup {operation} failed for {}: {source}",
                    path.display()
                )
            }
            Self::Sql { operation, source } => {
                write!(formatter, "catalog backup {operation} failed: {source}")
            }
            Self::InvalidSystemTime => {
                formatter.write_str("catalog backup clock precedes the Unix epoch")
            }
            Self::InvalidBackup { path } => {
                write!(formatter, "catalog backup is invalid: {}", path.display())
            }
        }
    }
}

impl std::error::Error for CatalogBackupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Sql { source, .. } => Some(source),
            Self::InvalidSystemTime | Self::InvalidBackup { .. } => None,
        }
    }
}

/// 使用 SQLite online backup API 创建一致的单文件 catalog 备份，并在成功后按策略清理旧文件。
pub fn create_catalog_backup(
    catalog: &Catalog,
    backup_directory: &Path,
    retention: BackupRetention,
) -> Result<PathBuf, CatalogBackupError> {
    create_catalog_backup_from_connection(catalog.connection(), backup_directory, retention)
}

/// 在 migration 前从尚未由 `Catalog` 打开的数据库创建一致性 backup。
///
/// 读取端以只读方式打开，仍使用 SQLite online backup API，因此不会复制 WAL 文件集合。
pub fn create_catalog_backup_from_path(
    catalog_path: &Path,
    backup_directory: &Path,
    retention: BackupRetention,
) -> Result<PathBuf, CatalogBackupError> {
    let source = Connection::open_with_flags(catalog_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| CatalogBackupError::Sql { operation: "open source", source })?;
    create_catalog_backup_from_connection(&source, backup_directory, retention)
}

fn create_catalog_backup_from_connection(
    source: &Connection,
    backup_directory: &Path,
    retention: BackupRetention,
) -> Result<PathBuf, CatalogBackupError> {
    fs::create_dir_all(backup_directory)
        .map_err(|source| io_error("create directory", backup_directory, source))?;
    let backup_path = backup_path(backup_directory)?;
    let temporary_path = temporary_backup_path(backup_directory)?;
    let mut temporary_guard = TemporaryBackupPath::new(temporary_path.clone());
    {
        let mut destination = Connection::open(&temporary_path)
            .map_err(|source| CatalogBackupError::Sql { operation: "open destination", source })?;
        let backup = Backup::new(source, &mut destination).map_err(|source| {
            CatalogBackupError::Sql { operation: "initialize online backup", source }
        })?;
        backup
            .run_to_completion(BACKUP_STEP_PAGE_COUNT, BACKUP_STEP_PAUSE, None)
            .map_err(|source| CatalogBackupError::Sql { operation: "copy catalog", source })?;
        drop(backup);
        validate_connection(&destination, &temporary_path)?;
    }
    fs::rename(&temporary_path, &backup_path)
        .map_err(|source| io_error("publish", &backup_path, source))?;
    temporary_guard.keep();
    prune_catalog_backups(backup_directory, retention)?;
    Ok(backup_path)
}

/// 返回最新且通过 SQLite integrity check 的备份；损坏备份会被跳过而非用于恢复。
pub fn latest_valid_catalog_backup(
    backup_directory: &Path,
) -> Result<Option<PathBuf>, CatalogBackupError> {
    let backups = catalog_backup_paths(backup_directory)?;
    for backup_path in backups.into_iter().rev() {
        if validate_backup_path(&backup_path).is_ok() {
            return Ok(Some(backup_path));
        }
    }
    Ok(None)
}

/// 从单文件一致性备份创建新的 catalog 文件。目标仅在完整复制和 integrity check 后替换。
pub fn restore_catalog_backup(
    backup_path: &Path,
    catalog_path: &Path,
) -> Result<(), CatalogBackupError> {
    validate_backup_path(backup_path)?;
    let parent = catalog_path
        .parent()
        .ok_or_else(|| CatalogBackupError::InvalidBackup { path: catalog_path.to_path_buf() })?;
    fs::create_dir_all(parent)
        .map_err(|source| io_error("create recovery directory", parent, source))?;
    let temporary_path = parent
        .join(format!(".catalog-recovery-{}.tmp", BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let mut temporary_guard = TemporaryBackupPath::new(temporary_path.clone());
    {
        let source =
            Connection::open_with_flags(backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
                |source| CatalogBackupError::Sql { operation: "open recovery source", source },
            )?;
        let mut destination = Connection::open(&temporary_path).map_err(|source| {
            CatalogBackupError::Sql { operation: "open recovery destination", source }
        })?;
        let backup = Backup::new(&source, &mut destination).map_err(|source| {
            CatalogBackupError::Sql { operation: "initialize recovery backup", source }
        })?;
        backup.run_to_completion(BACKUP_STEP_PAGE_COUNT, BACKUP_STEP_PAUSE, None).map_err(
            |source| CatalogBackupError::Sql { operation: "copy recovery backup", source },
        )?;
        drop(backup);
        validate_connection(&destination, &temporary_path)?;
    }
    fs::rename(&temporary_path, catalog_path)
        .map_err(|source| io_error("publish recovery", catalog_path, source))?;
    temporary_guard.keep();
    remove_stale_catalog_sidecars(catalog_path)?;
    Ok(())
}

/// 移除已被隔离或替换 catalog 留下的 WAL/SHM sidecar，避免新数据库重用旧日志。
pub(crate) fn remove_stale_catalog_sidecars(catalog_path: &Path) -> Result<(), CatalogBackupError> {
    let Some(file_name) = catalog_path.file_name() else {
        return Ok(());
    };
    let parent = catalog_path.parent().expect("catalog path with file name must have a parent");
    for suffix in ["-wal", "-shm"] {
        let sidecar = parent.join(format!("{}{}", file_name.to_string_lossy(), suffix));
        if !sidecar.exists() {
            continue;
        }
        fs::remove_file(&sidecar)
            .map_err(|source| io_error("remove stale WAL sidecar", &sidecar, source))?;
    }
    Ok(())
}

fn prune_catalog_backups(
    backup_directory: &Path,
    retention: BackupRetention,
) -> Result<(), CatalogBackupError> {
    let backups = catalog_backup_paths(backup_directory)?;
    let retained_start = backups.len().saturating_sub(retention.maximum_backups());
    for stale_backup in &backups[..retained_start] {
        fs::remove_file(stale_backup).map_err(|source| io_error("prune", stale_backup, source))?;
    }
    Ok(())
}

fn catalog_backup_paths(backup_directory: &Path) -> Result<Vec<PathBuf>, CatalogBackupError> {
    if !backup_directory.exists() {
        return Ok(Vec::new());
    }
    let mut backups = fs::read_dir(backup_directory)
        .map_err(|source| io_error("read directory", backup_directory, source))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(std::ffi::OsStr::new(BACKUP_FILE_EXTENSION)))
        .collect::<Vec<_>>();
    backups.sort();
    Ok(backups)
}

fn validate_backup_path(path: &Path) -> Result<(), CatalogBackupError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| CatalogBackupError::Sql { operation: "open backup", source })?;
    validate_connection(&connection, path)
}

fn validate_connection(connection: &Connection, path: &Path) -> Result<(), CatalogBackupError> {
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|source| CatalogBackupError::Sql { operation: "integrity check", source })?;
    if integrity == "ok" {
        return Ok(());
    }
    Err(CatalogBackupError::InvalidBackup { path: path.to_path_buf() })
}

fn backup_path(backup_directory: &Path) -> Result<PathBuf, CatalogBackupError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CatalogBackupError::InvalidSystemTime)?
        .as_nanos();
    let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(backup_directory
        .join(format!("catalog-{timestamp:020}-{sequence:020}.{BACKUP_FILE_EXTENSION}")))
}

fn temporary_backup_path(backup_directory: &Path) -> Result<PathBuf, CatalogBackupError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CatalogBackupError::InvalidSystemTime)?
        .as_nanos();
    let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(backup_directory.join(format!(".catalog-{timestamp:020}-{sequence:020}.tmp")))
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> CatalogBackupError {
    CatalogBackupError::Io { operation, path: path.to_path_buf(), source }
}

struct TemporaryBackupPath {
    path: PathBuf,
    should_remove: bool,
}

impl TemporaryBackupPath {
    fn new(path: PathBuf) -> Self {
        Self { path, should_remove: true }
    }

    fn keep(&mut self) {
        self.should_remove = false;
    }
}

impl Drop for TemporaryBackupPath {
    fn drop(&mut self) {
        if self.should_remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::{Catalog, CatalogNote, DocumentKind, NoteId};

    #[test]
    fn sqlite_backup_keeps_catalog_metadata_and_prunes_old_backups() {
        let directory = tempfile::tempdir().expect("test directory should exist");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: "note.md".into(),
                kind: DocumentKind::Markdown,
                title: "Note".to_owned(),
                excerpt: "fixture".to_owned(),
                modified_at: UNIX_EPOCH + Duration::from_secs(1),
                file_size: 7,
                content_hash: vec![1, 2, 3],
                starred: false,
            })
            .expect("fixture note should persist");
        assert!(catalog.toggle_note_starred(note_id).expect("fixture star should toggle"));
        let backup_directory = directory.path().join("backups");

        super::create_catalog_backup(
            &catalog,
            &backup_directory,
            super::BackupRetention::keep_latest(1).expect("positive retention should be valid"),
        )
        .expect("first backup should complete");
        super::create_catalog_backup(
            &catalog,
            &backup_directory,
            super::BackupRetention::keep_latest(1).expect("positive retention should be valid"),
        )
        .expect("second backup should complete");

        let backup_path = super::latest_valid_catalog_backup(&backup_directory)
            .expect("backup directory should scan")
            .expect("a valid backup should remain");
        let backup_catalog = Catalog::open(&backup_path).expect("backup catalog should open");
        assert!(
            backup_catalog
                .active_note(note_id)
                .expect("backup note should read")
                .expect("note should exist")
                .starred
        );
        assert_eq!(
            std::fs::read_dir(&backup_directory)
                .expect("backups should list")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension() == Some(std::ffi::OsStr::new("sqlite3")))
                .count(),
            1
        );
    }

    #[test]
    fn migration_backup_can_copy_a_catalog_before_it_is_opened_by_the_repository() {
        let directory = tempfile::tempdir().expect("test directory should exist");
        let catalog_path = directory.path().join("catalog.sqlite3");
        let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");
        drop(catalog);
        let backup_directory = directory.path().join("backups");

        let backup_path = super::create_catalog_backup_from_path(
            &catalog_path,
            &backup_directory,
            super::BackupRetention::keep_latest(1).expect("positive retention should be valid"),
        )
        .expect("migration backup should complete");

        assert_eq!(
            super::latest_valid_catalog_backup(&backup_directory)
                .expect("backup directory should scan"),
            Some(backup_path)
        );
    }
}
