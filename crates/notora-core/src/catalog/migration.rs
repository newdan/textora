use rusqlite::{Connection, Transaction};

use super::{CatalogError, CatalogError::UnsupportedSchema};

pub const CATALOG_SCHEMA_VERSION: u32 = 4;

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE notes (
    note_id TEXT PRIMARY KEY NOT NULL,
    relative_path TEXT NOT NULL UNIQUE,
    kind INTEGER NOT NULL,
    title TEXT NOT NULL,
    excerpt TEXT NOT NULL,
    created_ns INTEGER NOT NULL DEFAULT 0 CHECK (created_ns >= 0),
    modified_ns INTEGER NOT NULL,
    file_size INTEGER NOT NULL,
    content_hash BLOB NOT NULL,
    starred INTEGER NOT NULL DEFAULT 0 CHECK (starred IN (0, 1)),
    encryption INTEGER NOT NULL DEFAULT 0 CHECK (encryption IN (0, 1)),
    lifecycle INTEGER NOT NULL DEFAULT 0 CHECK (lifecycle IN (0, 1))
);

CREATE INDEX notes_active_path_index ON notes(lifecycle, relative_path);
CREATE INDEX notes_starred_modified_index ON notes(lifecycle, starred, modified_ns DESC);

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
"#;

const FULL_TEXT_SEARCH_SCHEMA: &str = r#"
CREATE VIRTUAL TABLE note_search USING fts5(
    note_id UNINDEXED,
    title,
    relative_path,
    body,
    tags,
    tokenize = 'trigram'
);

CREATE INDEX notes_active_modified_path_index
ON notes(lifecycle, modified_ns DESC, relative_path ASC, note_id ASC);
"#;

const MISSING_FILE_CONFIRMATION_SCHEMA: &str = r#"
ALTER TABLE notes
ADD COLUMN missing_scan_count INTEGER NOT NULL DEFAULT 0
CHECK (missing_scan_count >= 0);
"#;

const FTS5_TRIGRAM_CAPABILITY_PROBE: &str = "CREATE VIRTUAL TABLE temp.notora_fts5_trigram_probe USING fts5(contents, tokenize = 'trigram');";
const FTS5_TRIGRAM_CAPABILITY_CLEANUP: &str = "DROP TABLE temp.notora_fts5_trigram_probe;";

pub(super) fn verify_fts5_trigram_support(connection: &Connection) -> Result<(), CatalogError> {
    connection
        .execute_batch(FTS5_TRIGRAM_CAPABILITY_PROBE)
        .map_err(fts5_trigram_capability_error)?;
    connection.execute_batch(FTS5_TRIGRAM_CAPABILITY_CLEANUP).map_err(fts5_trigram_capability_error)
}

pub(super) fn migrate(connection: &mut Connection) -> Result<(), CatalogError> {
    let schema_version = schema_version(connection)?;
    if schema_version > CATALOG_SCHEMA_VERSION {
        return Err(UnsupportedSchema { found: schema_version });
    }
    if schema_version == CATALOG_SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .map_err(|source| CatalogError::sql("migration transaction start", source))?;
    apply_pending_migrations(&transaction, schema_version)?;
    transaction.commit().map_err(|source| CatalogError::sql("migration transaction commit", source))
}

pub(super) fn schema_version(connection: &Connection) -> Result<u32, CatalogError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| CatalogError::sql("schema version read", source))
}

fn apply_pending_migrations(
    transaction: &Transaction<'_>,
    mut schema_version: u32,
) -> Result<(), CatalogError> {
    if schema_version == 0 {
        transaction
            .execute_batch(INITIAL_SCHEMA)
            .map_err(|source| CatalogError::sql("initial schema migration", source))?;
        transaction
            .pragma_update(None, "user_version", 1_u32)
            .map_err(|source| CatalogError::sql("schema version write", source))?;
        schema_version = 1;
    }

    if schema_version == 1 {
        transaction
            .execute_batch(FULL_TEXT_SEARCH_SCHEMA)
            .map_err(full_text_search_schema_error)?;
        transaction
            .pragma_update(None, "user_version", 2_u32)
            .map_err(|source| CatalogError::sql("schema version write", source))?;
        schema_version = 2;
    }

    if schema_version == 2 {
        transaction
            .execute_batch(MISSING_FILE_CONFIRMATION_SCHEMA)
            .map_err(|source| CatalogError::sql("missing file confirmation migration", source))?;
        transaction
            .pragma_update(None, "user_version", 3_u32)
            .map_err(|source| CatalogError::sql("schema version write", source))?;
        schema_version = 3;
    }

    if schema_version == 3 {
        apply_editor_metadata_migration(transaction)?;
        transaction
            .pragma_update(None, "user_version", CATALOG_SCHEMA_VERSION)
            .map_err(|source| CatalogError::sql("schema version write", source))?;
        return Ok(());
    }

    Err(UnsupportedSchema { found: schema_version })
}

fn apply_editor_metadata_migration(transaction: &Transaction<'_>) -> Result<(), CatalogError> {
    if !table_has_column(transaction, "notes", "created_ns")? {
        transaction
            .execute_batch(
                "ALTER TABLE notes ADD COLUMN created_ns INTEGER NOT NULL DEFAULT 0 CHECK (created_ns >= 0);",
            )
            .map_err(|source| CatalogError::sql("created time migration", source))?;
    }
    if !table_has_column(transaction, "notes", "encryption")? {
        transaction
            .execute_batch(
                "ALTER TABLE notes ADD COLUMN encryption INTEGER NOT NULL DEFAULT 0 CHECK (encryption IN (0, 1));",
            )
            .map_err(|source| CatalogError::sql("encryption migration", source))?;
    }
    transaction
        .execute("UPDATE notes SET created_ns = modified_ns WHERE created_ns = 0", [])
        .map_err(|source| CatalogError::sql("created time backfill", source))?;
    Ok(())
}

fn table_has_column(
    transaction: &Transaction<'_>,
    table_name: &str,
    column_name: &str,
) -> Result<bool, CatalogError> {
    transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2
            )",
            [table_name, column_name],
            |row| row.get(0),
        )
        .map_err(|source| CatalogError::sql("schema column lookup", source))
}

fn fts5_trigram_capability_error(source: rusqlite::Error) -> CatalogError {
    CatalogError::Fts5TrigramUnavailable { source }
}

fn full_text_search_schema_error(source: rusqlite::Error) -> CatalogError {
    if is_missing_fts5_trigram_support(&source) {
        return CatalogError::Fts5TrigramUnavailable { source };
    }

    CatalogError::sql("full-text search schema migration", source)
}

fn is_missing_fts5_trigram_support(source: &rusqlite::Error) -> bool {
    let diagnostic = source.to_string();
    diagnostic.contains("no such module: fts5")
        || diagnostic.contains("no such tokenizer: trigram")
        || diagnostic.contains("unknown tokenizer: trigram")
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        CATALOG_SCHEMA_VERSION, FULL_TEXT_SEARCH_SCHEMA, INITIAL_SCHEMA, migrate, schema_version,
        verify_fts5_trigram_support,
    };

    #[test]
    fn bundled_sqlite_supports_fts5_trigram() {
        let connection = Connection::open_in_memory().expect("in-memory catalog should open");

        verify_fts5_trigram_support(&connection)
            .expect("bundled SQLite must support the FTS5 trigram tokenizer");
    }

    #[test]
    fn version_one_catalog_migrates_to_fts_schema_atomically() {
        let mut connection = Connection::open_in_memory().expect("in-memory catalog should open");
        connection
            .execute_batch(INITIAL_SCHEMA)
            .expect("version one schema fixture should initialize");
        connection
            .pragma_update(None, "user_version", 1_u32)
            .expect("version one schema fixture should be marked");

        migrate(&mut connection).expect("FTS schema migration should succeed");

        assert_eq!(
            schema_version(&connection).expect("schema version should be readable"),
            CATALOG_SCHEMA_VERSION
        );
        let fts_definition: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'note_search'",
                [],
                |row| row.get(0),
            )
            .expect("FTS table definition should exist");
        assert!(fts_definition.contains("tokenize = 'trigram'"));
        let paging_index_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'notes_active_modified_path_index')",
                [],
                |row| row.get(0),
            )
            .expect("paging index lookup should succeed");
        assert!(paging_index_exists);
    }

    #[test]
    fn version_two_catalog_adds_missing_file_confirmation_state() {
        let mut connection = Connection::open_in_memory().expect("in-memory catalog should open");
        connection
            .execute_batch(INITIAL_SCHEMA)
            .expect("version one schema fixture should initialize");
        connection
            .execute_batch(FULL_TEXT_SEARCH_SCHEMA)
            .expect("version two FTS fixture should initialize");
        connection
            .pragma_update(None, "user_version", 2_u32)
            .expect("version two fixture should be marked");

        migrate(&mut connection).expect("missing confirmation migration should succeed");

        assert_eq!(
            schema_version(&connection).expect("schema version should be readable"),
            CATALOG_SCHEMA_VERSION
        );
        let missing_scan_count_column_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('notes') WHERE name = 'missing_scan_count')",
                [],
                |row| row.get(0),
            )
            .expect("missing confirmation column should be queryable");
        assert!(missing_scan_count_column_exists);
    }

    #[test]
    fn version_three_catalog_backfills_editor_metadata_without_changing_modified_time() {
        let mut connection = Connection::open_in_memory().expect("in-memory catalog should open");
        connection.execute_batch(INITIAL_SCHEMA).expect("legacy schema fixture should initialize");
        connection
            .pragma_update(None, "user_version", 3_u32)
            .expect("version three fixture should be marked");
        connection
            .execute(
                "INSERT INTO notes (
                    note_id, relative_path, kind, title, excerpt, modified_ns, file_size, content_hash, lifecycle
                ) VALUES ('legacy', 'legacy.md', 2, 'Legacy', '', 123, 0, X'', 0)",
                [],
            )
            .expect("legacy note should insert");

        migrate(&mut connection).expect("version three should migrate");

        let editor_metadata: (i64, i64) = connection
            .query_row(
                "SELECT created_ns, encryption FROM notes WHERE note_id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("editor metadata should be readable");
        assert_eq!(editor_metadata, (123, 0));
    }

    #[test]
    fn failed_fts_migration_keeps_the_version_one_schema_usable() {
        let mut connection = Connection::open_in_memory().expect("in-memory catalog should open");
        connection
            .execute_batch(INITIAL_SCHEMA)
            .expect("version one schema fixture should initialize");
        connection
            .execute_batch("CREATE TABLE note_search (note_id TEXT NOT NULL);")
            .expect("conflicting FTS fixture should initialize");
        connection
            .pragma_update(None, "user_version", 1_u32)
            .expect("version one schema fixture should be marked");

        assert!(matches!(
            migrate(&mut connection),
            Err(super::CatalogError::Sql { operation: "full-text search schema migration", .. })
        ));
        assert_eq!(schema_version(&connection).expect("schema version should remain readable"), 1);
        let paging_index_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'notes_active_modified_path_index')",
                [],
                |row| row.get(0),
            )
            .expect("paging index lookup should succeed");
        assert!(!paging_index_exists);
    }
}
