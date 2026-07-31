use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Row, params};
use uuid::Uuid;

use crate::{DocumentKind, NoteId};

use super::{Catalog, CatalogError};

const DOCUMENT_KIND_TEXT: i64 = 1;
const DOCUMENT_KIND_MARKDOWN: i64 = 2;
const DOCUMENT_KIND_MINDMAP: i64 = 3;
const ACTIVE_NOTE_LIFECYCLE: i64 = 0;

/// 已由扫描器读取并准备写入 catalog 的活动笔记记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogNote {
    pub note_id: NoteId,
    pub relative_path: PathBuf,
    pub kind: DocumentKind,
    pub title: String,
    pub excerpt: String,
    pub modified_at: SystemTime,
    pub file_size: u64,
    pub content_hash: Vec<u8>,
    pub starred: bool,
}

impl Catalog {
    /// 插入新笔记，或按稳定 `NoteId` 更新扫描得到的派生字段。
    ///
    /// 星标属于用户 metadata，扫描更新不得覆盖它。
    pub fn upsert_active_note(&self, note: &CatalogNote) -> Result<(), CatalogError> {
        let modified_nanoseconds = system_time_to_nanoseconds(note.modified_at)?;
        let file_size =
            i64::try_from(note.file_size).map_err(|_| CatalogError::InvalidStoredValue {
                column: "file_size",
                value: note.file_size.to_string(),
            })?;
        self.connection()
            .execute(
                "INSERT INTO notes (
                    note_id, relative_path, kind, title, excerpt, modified_ns, file_size, content_hash, lifecycle
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(note_id) DO UPDATE SET
                    relative_path = excluded.relative_path,
                    kind = excluded.kind,
                    title = excluded.title,
                    excerpt = excluded.excerpt,
                    modified_ns = excluded.modified_ns,
                    file_size = excluded.file_size,
                    content_hash = excluded.content_hash",
                params![
                    note.note_id.to_string(),
                    note.relative_path.to_string_lossy(),
                    document_kind_to_database(note.kind),
                    note.title,
                    note.excerpt,
                    modified_nanoseconds,
                    file_size,
                    note.content_hash,
                    ACTIVE_NOTE_LIFECYCLE,
                ],
            )
            .map_err(|source| CatalogError::sql("note upsert", source))?;
        Ok(())
    }

    pub fn active_notes(&self) -> Result<Vec<CatalogNote>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT note_id, relative_path, kind, title, excerpt, modified_ns, file_size, content_hash, starred
                 FROM notes
                 WHERE lifecycle = ?1
                 ORDER BY relative_path ASC",
            )
            .map_err(|source| CatalogError::sql("active notes query preparation", source))?;
        let stored_notes = statement
            .query_map([ACTIVE_NOTE_LIFECYCLE], stored_note_from_row)
            .map_err(|source| CatalogError::sql("active notes query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("active notes row read", source))?;

        stored_notes.into_iter().map(CatalogNote::try_from).collect()
    }
}

#[derive(Debug)]
struct StoredNote {
    note_id: String,
    relative_path: String,
    kind: i64,
    title: String,
    excerpt: String,
    modified_nanoseconds: i64,
    file_size: i64,
    content_hash: Vec<u8>,
    starred: i64,
}

impl TryFrom<StoredNote> for CatalogNote {
    type Error = CatalogError;

    fn try_from(stored_note: StoredNote) -> Result<Self, Self::Error> {
        let note_id = Uuid::parse_str(&stored_note.note_id).map(NoteId::from).map_err(|_| {
            CatalogError::InvalidStoredValue { column: "note_id", value: stored_note.note_id }
        })?;
        let kind = document_kind_from_database(stored_note.kind)?;
        let modified_at = nanoseconds_to_system_time(stored_note.modified_nanoseconds)?;
        let file_size =
            u64::try_from(stored_note.file_size).map_err(|_| CatalogError::InvalidStoredValue {
                column: "file_size",
                value: stored_note.file_size.to_string(),
            })?;
        let starred = match stored_note.starred {
            0 => false,
            1 => true,
            value => {
                return Err(CatalogError::InvalidStoredValue {
                    column: "starred",
                    value: value.to_string(),
                });
            }
        };

        Ok(Self {
            note_id,
            relative_path: stored_note.relative_path.into(),
            kind,
            title: stored_note.title,
            excerpt: stored_note.excerpt,
            modified_at,
            file_size,
            content_hash: stored_note.content_hash,
            starred,
        })
    }
}

fn stored_note_from_row(row: &Row<'_>) -> rusqlite::Result<StoredNote> {
    Ok(StoredNote {
        note_id: row.get(0)?,
        relative_path: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        excerpt: row.get(4)?,
        modified_nanoseconds: row.get(5)?,
        file_size: row.get(6)?,
        content_hash: row.get(7)?,
        starred: row.get(8)?,
    })
}

fn document_kind_to_database(kind: DocumentKind) -> i64 {
    match kind {
        DocumentKind::Text => DOCUMENT_KIND_TEXT,
        DocumentKind::Markdown => DOCUMENT_KIND_MARKDOWN,
        DocumentKind::Mindmap => DOCUMENT_KIND_MINDMAP,
    }
}

fn document_kind_from_database(value: i64) -> Result<DocumentKind, CatalogError> {
    match value {
        DOCUMENT_KIND_TEXT => Ok(DocumentKind::Text),
        DOCUMENT_KIND_MARKDOWN => Ok(DocumentKind::Markdown),
        DOCUMENT_KIND_MINDMAP => Ok(DocumentKind::Mindmap),
        _ => Err(CatalogError::InvalidStoredValue { column: "kind", value: value.to_string() }),
    }
}

fn system_time_to_nanoseconds(value: SystemTime) -> Result<i64, CatalogError> {
    let duration =
        value.duration_since(UNIX_EPOCH).map_err(|_| CatalogError::InvalidStoredValue {
            column: "modified_at",
            value: "before UNIX epoch".to_owned(),
        })?;
    i64::try_from(duration.as_nanos()).map_err(|_| CatalogError::InvalidStoredValue {
        column: "modified_at",
        value: "outside SQLite nanosecond range".to_owned(),
    })
}

fn nanoseconds_to_system_time(value: i64) -> Result<SystemTime, CatalogError> {
    let duration = Duration::from_nanos(u64::try_from(value).map_err(|_| {
        CatalogError::InvalidStoredValue { column: "modified_ns", value: value.to_string() }
    })?);
    UNIX_EPOCH.checked_add(duration).ok_or_else(|| CatalogError::InvalidStoredValue {
        column: "modified_ns",
        value: value.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::CatalogNote;
    use crate::{Catalog, DocumentKind, NoteId};

    fn catalog_note(note_id: NoteId, relative_path: &str, title: &str) -> CatalogNote {
        CatalogNote {
            note_id,
            relative_path: relative_path.into(),
            kind: DocumentKind::Markdown,
            title: title.to_owned(),
            excerpt: "excerpt".to_owned(),
            modified_at: UNIX_EPOCH + Duration::from_secs(1),
            file_size: 8,
            content_hash: vec![1, 2, 3],
            starred: false,
        }
    }

    #[test]
    fn upsert_keeps_note_id_stable_and_preserves_starred_metadata() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .upsert_active_note(&catalog_note(note_id, "first.md", "First"))
            .expect("initial note should insert");
        catalog
            .connection()
            .execute("UPDATE notes SET starred = 1 WHERE note_id = ?1", [note_id.to_string()])
            .expect("star metadata fixture should update");
        catalog
            .upsert_active_note(&catalog_note(note_id, "renamed.md", "Renamed"))
            .expect("same note should update after rename");

        assert_eq!(
            catalog.active_notes().expect("active notes should load"),
            vec![CatalogNote { starred: true, ..catalog_note(note_id, "renamed.md", "Renamed") }]
        );
    }

    #[test]
    fn distinct_note_ids_cannot_claim_the_same_relative_path() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        catalog
            .upsert_active_note(&catalog_note(NoteId::generate(), "duplicate.md", "First"))
            .expect("initial note should insert");

        assert!(
            catalog
                .upsert_active_note(&catalog_note(NoteId::generate(), "duplicate.md", "Second"))
                .is_err()
        );
    }
}
