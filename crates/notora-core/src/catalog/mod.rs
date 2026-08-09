mod card_repository;
mod metadata_repository;
mod migration;
mod note_repository;
mod search_repository;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;

pub use card_repository::{CatalogCard, CatalogCardCursor, CatalogCardPage};
pub use metadata_repository::{CatalogNavigationTree, TagWithActiveNoteCount};
pub use migration::CATALOG_SCHEMA_VERSION;
pub use note_repository::{
    CatalogNote, NotePathOperation, NotePathOperationKind, NotePathOperationState, TrashEntry,
};
pub use search_repository::SearchIndexEntry;

/// 已完成基础迁移的工作区 catalog 连接。
pub struct Catalog {
    connection: Connection,
}

static CORRUPT_CATALOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// catalog 打开后的恢复结果，调用方仍需扫描正文以重建派生字段和 FTS。
pub enum CatalogOpenOutcome {
    Opened(Catalog),
    RecoveredFromBackup { catalog: Catalog, backup_path: PathBuf },
    RebuiltWithoutMetadata { catalog: Catalog, corrupt_path: PathBuf },
}

impl CatalogOpenOutcome {
    pub fn catalog(&self) -> &Catalog {
        match self {
            Self::Opened(catalog)
            | Self::RecoveredFromBackup { catalog, .. }
            | Self::RebuiltWithoutMetadata { catalog, .. } => catalog,
        }
    }

    pub fn into_catalog(self) -> Catalog {
        match self {
            Self::Opened(catalog)
            | Self::RecoveredFromBackup { catalog, .. }
            | Self::RebuiltWithoutMetadata { catalog, .. } => catalog,
        }
    }
}

#[derive(Debug)]
pub enum CatalogRecoveryError {
    Backup(crate::CatalogBackupError),
    Catalog(CatalogError),
    Io { path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for CatalogRecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backup(source) => write!(formatter, "catalog backup recovery failed: {source}"),
            Self::Catalog(source) => write!(formatter, "catalog recovery open failed: {source}"),
            Self::Io { path, source } => {
                write!(formatter, "catalog recovery I/O failed for {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for CatalogRecoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backup(source) => Some(source),
            Self::Catalog(source) => Some(source),
            Self::Io { source, .. } => Some(source),
        }
    }
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        let mut connection = Connection::open(path)
            .map_err(|source| CatalogError::Open { path: path.to_path_buf(), source })?;
        verify_catalog_integrity(&connection, path)?;
        configure_connection(&connection)?;
        migration::verify_fts5_trigram_support(&connection)?;
        migration::migrate(&mut connection)?;

        Ok(Self { connection })
    }

    /// 判断一个既有 catalog 是否需要 schema migration；不存在的 catalog 不需要备份。
    pub fn migration_required(path: &Path) -> Result<bool, CatalogError> {
        if !path.exists() {
            return Ok(false);
        }
        let connection = Connection::open(path)
            .map_err(|source| CatalogError::Open { path: path.to_path_buf(), source })?;
        let schema_version = migration::schema_version(&connection)?;
        Ok(schema_version < CATALOG_SCHEMA_VERSION)
    }

    /// 先尝试正常打开；损坏时从最新有效备份恢复，否则保留损坏副本并创建空 catalog。
    pub fn open_or_recover(
        path: &Path,
        backup_directory: &Path,
    ) -> Result<CatalogOpenOutcome, CatalogRecoveryError> {
        let initial_error = match Self::open(path) {
            Ok(catalog) => return Ok(CatalogOpenOutcome::Opened(catalog)),
            Err(error) => error,
        };
        if !is_catalog_corruption(&initial_error) {
            return Err(CatalogRecoveryError::Catalog(initial_error));
        }
        let corrupt_path = quarantine_corrupt_catalog(path)?;
        if let Some(backup_path) = crate::latest_valid_catalog_backup(backup_directory)
            .map_err(CatalogRecoveryError::Backup)?
        {
            crate::restore_catalog_backup(&backup_path, path)
                .map_err(CatalogRecoveryError::Backup)?;
            let catalog = Self::open(path).map_err(CatalogRecoveryError::Catalog)?;
            return Ok(CatalogOpenOutcome::RecoveredFromBackup { catalog, backup_path });
        }
        crate::backup::remove_stale_catalog_sidecars(path).map_err(CatalogRecoveryError::Backup)?;
        let catalog = Self::open(path).map_err(CatalogRecoveryError::Catalog)?;
        Ok(CatalogOpenOutcome::RebuiltWithoutMetadata { catalog, corrupt_path })
    }

    pub fn schema_version(&self) -> Result<u32, CatalogError> {
        migration::schema_version(&self.connection)
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }
}

fn is_catalog_corruption(error: &CatalogError) -> bool {
    match error {
        CatalogError::Open { source, .. } | CatalogError::Sql { source, .. } => {
            let diagnostic = source.to_string().to_lowercase();
            diagnostic.contains("malformed")
                || diagnostic.contains("not a database")
                || diagnostic.contains("database disk image is malformed")
                || diagnostic.contains("file is not a database")
        }
        CatalogError::Integrity { .. } => true,
        CatalogError::Fts5TrigramUnavailable { .. }
        | CatalogError::UnsupportedSchema { .. }
        | CatalogError::InvalidStoredValue { .. } => false,
    }
}

fn quarantine_corrupt_catalog(path: &Path) -> Result<PathBuf, CatalogRecoveryError> {
    let corrupt_path = path.with_extension(format!(
        "sqlite3.corrupt.{}",
        CORRUPT_CATALOG_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if path.exists() {
        std::fs::rename(path, &corrupt_path)
            .map_err(|source| CatalogRecoveryError::Io { path: path.to_path_buf(), source })?;
    }
    Ok(corrupt_path)
}

#[derive(Debug)]
pub enum CatalogError {
    Open { path: std::path::PathBuf, source: rusqlite::Error },
    Sql { operation: &'static str, source: rusqlite::Error },
    Fts5TrigramUnavailable { source: rusqlite::Error },
    Integrity { path: PathBuf, diagnostic: String },
    UnsupportedSchema { found: u32 },
    InvalidStoredValue { column: &'static str, value: String },
}

impl CatalogError {
    pub(crate) fn sql(operation: &'static str, source: rusqlite::Error) -> Self {
        Self::Sql { operation, source }
    }
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { path, source } => {
                write!(formatter, "catalog open failed for {}: {source}", path.display())
            }
            Self::Sql { operation, source } => {
                write!(formatter, "catalog {operation} failed: {source}")
            }
            Self::Fts5TrigramUnavailable { source } => {
                write!(
                    formatter,
                    "catalog requires SQLite FTS5 with the trigram tokenizer: {source}"
                )
            }
            Self::Integrity { path, diagnostic } => {
                write!(
                    formatter,
                    "catalog integrity check failed for {}: {diagnostic}",
                    path.display()
                )
            }
            Self::UnsupportedSchema { found } => {
                write!(formatter, "unsupported catalog schema version: {found}")
            }
            Self::InvalidStoredValue { column, value } => {
                write!(formatter, "catalog contains invalid {column}: {value}")
            }
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. }
            | Self::Sql { source, .. }
            | Self::Fts5TrigramUnavailable { source } => Some(source),
            Self::Integrity { .. }
            | Self::UnsupportedSchema { .. }
            | Self::InvalidStoredValue { .. } => None,
        }
    }
}

fn configure_connection(connection: &Connection) -> Result<(), CatalogError> {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;",
        )
        .map_err(|source| CatalogError::sql("connection configuration", source))
}

fn verify_catalog_integrity(connection: &Connection, path: &Path) -> Result<(), CatalogError> {
    let diagnostic: String =
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|source| CatalogError::sql("catalog integrity check", source))?;
    if diagnostic == "ok" {
        return Ok(());
    }
    Err(CatalogError::Integrity { path: path.to_path_buf(), diagnostic })
}

#[cfg(test)]
mod tests {
    use super::{CATALOG_SCHEMA_VERSION, Catalog};

    #[test]
    fn catalog_open_applies_initial_schema_once() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog_path = directory.path().join("catalog.sqlite3");
        let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");

        assert_eq!(
            catalog.schema_version().expect("schema version should be readable"),
            CATALOG_SCHEMA_VERSION
        );
        let notes_table_exists: bool = catalog
            .connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'notes')",
                [],
                |row| row.get(0),
            )
            .expect("notes table lookup should succeed");
        assert!(notes_table_exists);
        let journal_mode: String = catalog
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal mode should be readable");
        assert_eq!(journal_mode, "wal");

        let reopened = Catalog::open(&catalog_path).expect("migrated catalog should reopen");
        assert_eq!(
            reopened.schema_version().expect("schema version should remain readable"),
            CATALOG_SCHEMA_VERSION
        );
    }

    #[test]
    fn catalog_rejects_a_future_schema_version() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog_path = directory.path().join("catalog.sqlite3");
        let connection = rusqlite::Connection::open(&catalog_path)
            .expect("future schema fixture database should open");
        let future_schema_version = CATALOG_SCHEMA_VERSION + 1;
        connection
            .pragma_update(None, "user_version", future_schema_version)
            .expect("future schema version should be written");

        assert!(matches!(
            Catalog::open(&catalog_path),
            Err(super::CatalogError::UnsupportedSchema { found }) if found == future_schema_version
        ));
    }

    #[test]
    fn migration_required_is_false_for_missing_or_current_catalogs() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog_path = directory.path().join("catalog.sqlite3");
        assert!(
            !Catalog::migration_required(&catalog_path).expect("missing catalog is not migrated")
        );
        let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");
        drop(catalog);
        assert!(
            !Catalog::migration_required(&catalog_path)
                .expect("current catalog needs no migration")
        );
    }

    #[test]
    fn migration_required_detects_an_older_schema_version_before_open() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog_path = directory.path().join("catalog.sqlite3");
        let connection = rusqlite::Connection::open(&catalog_path)
            .expect("old schema fixture database should open");
        connection
            .pragma_update(None, "user_version", 1_u32)
            .expect("old schema version should be written");

        assert!(Catalog::migration_required(&catalog_path).expect("older schema should migrate"));
    }

    #[test]
    fn catalog_recovery_restores_damaged_catalog_from_the_latest_valid_backup() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog_path = directory.path().join("catalog.sqlite3");
        let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");
        let backups = directory.path().join("backups");
        crate::create_catalog_backup(
            &catalog,
            &backups,
            crate::BackupRetention::keep_latest(1).expect("positive retention should be valid"),
        )
        .expect("backup should complete");
        drop(catalog);
        std::fs::write(&catalog_path, "not a sqlite database")
            .expect("fixture should corrupt catalog");

        assert!(matches!(
            Catalog::open_or_recover(&catalog_path, &backups),
            Ok(super::CatalogOpenOutcome::RecoveredFromBackup { .. })
        ));
    }

    #[test]
    fn catalog_recovery_without_backup_quarantines_damage_and_discards_stale_wal_sidecars() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog_path = directory.path().join("catalog.sqlite3");
        std::fs::write(&catalog_path, "not a sqlite database")
            .expect("fixture should corrupt catalog");
        std::fs::write(directory.path().join("catalog.sqlite3-wal"), "stale wal")
            .expect("fixture should create a stale WAL");
        std::fs::write(directory.path().join("catalog.sqlite3-shm"), "stale shm")
            .expect("fixture should create a stale SHM");

        let outcome = Catalog::open_or_recover(&catalog_path, &directory.path().join("backups"))
            .expect("recovery without a backup should rebuild an empty catalog");

        assert!(matches!(outcome, super::CatalogOpenOutcome::RebuiltWithoutMetadata { .. }));
        assert_ne!(
            std::fs::read(directory.path().join("catalog.sqlite3-wal"))
                .expect("rebuilt catalog should own any current WAL"),
            b"stale wal"
        );
        assert_ne!(
            std::fs::read(directory.path().join("catalog.sqlite3-shm"))
                .expect("rebuilt catalog should own any current SHM"),
            b"stale shm"
        );
    }
}
