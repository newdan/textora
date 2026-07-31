use rusqlite::{Connection, Transaction};

use super::{CatalogError, CatalogError::UnsupportedSchema};

pub const CATALOG_SCHEMA_VERSION: u32 = 1;

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE notes (
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
    schema_version: u32,
) -> Result<(), CatalogError> {
    if schema_version == 0 {
        transaction
            .execute_batch(INITIAL_SCHEMA)
            .map_err(|source| CatalogError::sql("initial schema migration", source))?;
        transaction
            .pragma_update(None, "user_version", CATALOG_SCHEMA_VERSION)
            .map_err(|source| CatalogError::sql("schema version write", source))?;
        return Ok(());
    }

    Err(UnsupportedSchema { found: schema_version })
}
