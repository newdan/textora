mod card_repository;
mod migration;
mod note_repository;
mod search_repository;

use std::path::Path;

use rusqlite::Connection;

pub use card_repository::{CatalogCard, CatalogCardCursor, CatalogCardPage};
pub use migration::CATALOG_SCHEMA_VERSION;
pub use note_repository::CatalogNote;
pub use search_repository::SearchIndexEntry;

/// 已完成基础迁移的工作区 catalog 连接。
pub struct Catalog {
    connection: Connection,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        let mut connection = Connection::open(path)
            .map_err(|source| CatalogError::Open { path: path.to_path_buf(), source })?;
        configure_connection(&connection)?;
        migration::verify_fts5_trigram_support(&connection)?;
        migration::migrate(&mut connection)?;

        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> Result<u32, CatalogError> {
        migration::schema_version(&self.connection)
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[derive(Debug)]
pub enum CatalogError {
    Open { path: std::path::PathBuf, source: rusqlite::Error },
    Sql { operation: &'static str, source: rusqlite::Error },
    Fts5TrigramUnavailable { source: rusqlite::Error },
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
            Self::UnsupportedSchema { .. } | Self::InvalidStoredValue { .. } => None,
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
}
