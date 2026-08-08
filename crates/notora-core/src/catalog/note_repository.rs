use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, Row, params};
use uuid::Uuid;

use crate::domain::{NoteEncryption, TitleInitialization};
use crate::{DocumentKind, NoteId};

use super::{Catalog, CatalogError};

const DOCUMENT_KIND_TEXT: i64 = 1;
const DOCUMENT_KIND_MARKDOWN: i64 = 2;
const DOCUMENT_KIND_MINDMAP: i64 = 3;
const ACTIVE_NOTE_LIFECYCLE: i64 = 0;
const TRASHED_NOTE_LIFECYCLE: i64 = 1;
const MISSING_SCAN_CONFIRMATION_COUNT: i64 = 2;
const NOTE_ENCRYPTION_UNENCRYPTED: i64 = 0;
const NOTE_ENCRYPTION_ENCRYPTED: i64 = 1;

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

pub use crate::domain::NoteEditorMetadata;

/// 一条已移入工作区回收站的笔记记录；路径始终相对于工作区根。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub note_id: NoteId,
    pub original_relative_path: PathBuf,
    pub trash_relative_path: PathBuf,
    pub deleted_at: SystemTime,
}

impl Catalog {
    /// 原子插入由新建命令产生的笔记；正式标签由正文外的 metadata 流程维护。
    pub fn create_active_note(
        &self,
        note: &CatalogNote,
        encryption: NoteEncryption,
        title_initialization: TitleInitialization,
    ) -> Result<(), CatalogError> {
        let modified_nanoseconds = system_time_to_nanoseconds(note.modified_at)?;
        let file_size =
            i64::try_from(note.file_size).map_err(|_| CatalogError::InvalidStoredValue {
                column: "file_size",
                value: note.file_size.to_string(),
            })?;
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("note creation transaction start", source))?;
        transaction
            .execute(
                "INSERT INTO notes (
                    note_id, relative_path, kind, title, excerpt, created_ns, modified_ns, file_size, content_hash, encryption, lifecycle
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10)",
                params![
                    note.note_id.to_string(),
                    note.relative_path.to_string_lossy(),
                    document_kind_to_database(note.kind),
                    note.title,
                    note.excerpt,
                    modified_nanoseconds,
                    file_size,
                    note.content_hash,
                    note_encryption_to_database(encryption),
                    ACTIVE_NOTE_LIFECYCLE,
                ],
            )
            .map_err(|source| CatalogError::sql("created note insert", source))?;
        if title_initialization == TitleInitialization::AwaitingFirstCommit {
            transaction
                .execute(
                    "INSERT INTO note_title_initializations (note_id) VALUES (?1)",
                    [note.note_id.to_string()],
                )
                .map_err(|source| CatalogError::sql("title initialization state insert", source))?;
        }
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("note creation transaction commit", source))?;
        Ok(())
    }

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
                    note_id, relative_path, kind, title, excerpt, created_ns, modified_ns, file_size, content_hash, encryption, lifecycle
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT(note_id) DO UPDATE SET
                    relative_path = excluded.relative_path,
                    kind = excluded.kind,
                    excerpt = excluded.excerpt,
                    modified_ns = excluded.modified_ns,
                    file_size = excluded.file_size,
                    content_hash = excluded.content_hash,
                    missing_scan_count = 0",
                params![
                    note.note_id.to_string(),
                    note.relative_path.to_string_lossy(),
                    document_kind_to_database(note.kind),
                    note.title,
                    note.excerpt,
                    modified_nanoseconds,
                    file_size,
                    note.content_hash,
                    NOTE_ENCRYPTION_UNENCRYPTED,
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

    /// 按稳定 ID 精确读取活动笔记，避免文件命令以路径猜测来源。
    pub fn active_note(&self, note_id: NoteId) -> Result<Option<CatalogNote>, CatalogError> {
        let stored_note = self
            .connection()
            .query_row(
                "SELECT note_id, relative_path, kind, title, excerpt, modified_ns, file_size, content_hash, starred
                 FROM notes
                 WHERE note_id = ?1 AND lifecycle = ?2",
                params![note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
                stored_note_from_row,
            )
            .optional()
            .map_err(|source| CatalogError::sql("active note query", source))?;
        stored_note.map(CatalogNote::try_from).transpose()
    }

    /// 读取编辑区需要的创建时间与持久化加密状态；不改变扫描器的 `CatalogNote`。
    pub fn note_editor_metadata(
        &self,
        note_id: NoteId,
    ) -> Result<Option<NoteEditorMetadata>, CatalogError> {
        self.connection()
            .query_row(
                "SELECT note_id, created_ns, modified_ns, encryption,
                        EXISTS(SELECT 1 FROM note_title_initializations initialization
                               WHERE initialization.note_id = notes.note_id)
                 FROM notes WHERE note_id = ?1",
                [note_id.to_string()],
                editor_metadata_from_row,
            )
            .optional()
            .map_err(|source| CatalogError::sql("note editor metadata query", source))?
            .map(NoteEditorMetadata::try_from)
            .transpose()
    }

    /// 更新已经独立的 Notora 标题，不触碰正文。
    pub fn update_note_title(&self, note_id: NoteId, title: &str) -> Result<(), CatalogError> {
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("note title transaction start", source))?;
        update_stored_note_title(&transaction, note_id, title)?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("note title transaction commit", source))
    }

    /// 原子竞争一次性标题初始化；返回 `true` 表示本次提交获胜。
    pub fn complete_title_initialization(
        &self,
        note_id: NoteId,
        title: Option<&str>,
    ) -> Result<bool, CatalogError> {
        let transaction = self.connection().unchecked_transaction().map_err(|source| {
            CatalogError::sql("title initialization transaction start", source)
        })?;
        let removed_rows = transaction
            .execute(
                "DELETE FROM note_title_initializations WHERE note_id = ?1",
                [note_id.to_string()],
            )
            .map_err(|source| CatalogError::sql("title initialization claim", source))?;
        if removed_rows == 0 {
            transaction
                .commit()
                .map_err(|source| CatalogError::sql("title initialization no-op commit", source))?;
            return Ok(false);
        }
        if let Some(title) = title {
            update_stored_note_title(&transaction, note_id, title)?;
        }
        transaction.commit().map_err(|source| {
            CatalogError::sql("title initialization transaction commit", source)
        })?;
        Ok(true)
    }

    /// 在文件系统移动成功后更新活动笔记的相对路径，保持其 `NoteId` 不变。
    pub fn update_active_note_path(
        &self,
        note_id: NoteId,
        relative_path: &Path,
    ) -> Result<(), CatalogError> {
        let updated_rows = self
            .connection()
            .execute(
                "UPDATE notes
                 SET relative_path = ?1
                 WHERE note_id = ?2 AND lifecycle = ?3",
                params![
                    relative_path.to_string_lossy(),
                    note_id.to_string(),
                    ACTIVE_NOTE_LIFECYCLE,
                ],
            )
            .map_err(|source| CatalogError::sql("note path update", source))?;
        if updated_rows == 1 {
            return Ok(());
        }

        Err(CatalogError::InvalidStoredValue { column: "note_id", value: note_id.to_string() })
    }

    /// 记录一次完整扫描观察到的缺失笔记，并只删除连续两次完整扫描都缺失的行。
    ///
    /// 在两次扫描之间重新出现的笔记会清除确认计数，避免 watcher 的瞬态 rename 或
    /// 原子替换事件直接删除 catalog identity 与用户 metadata。
    pub fn reconcile_active_note_presence(
        &self,
        present_note_ids: &[NoteId],
        missing_note_ids: &[NoteId],
    ) -> Result<usize, CatalogError> {
        let transaction = self.connection().unchecked_transaction().map_err(|source| {
            CatalogError::sql("missing note confirmation transaction start", source)
        })?;
        for note_id in present_note_ids {
            transaction
                .execute(
                    "UPDATE notes
                     SET missing_scan_count = 0
                     WHERE note_id = ?1 AND lifecycle = ?2 AND missing_scan_count > 0",
                    params![note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
                )
                .map_err(|source| CatalogError::sql("missing note confirmation reset", source))?;
        }

        let mut removed_count = 0;
        for note_id in missing_note_ids {
            let note_id = note_id.to_string();
            let updated_rows = transaction
                .execute(
                    "UPDATE notes
                     SET missing_scan_count = missing_scan_count + 1
                     WHERE note_id = ?1 AND lifecycle = ?2",
                    params![note_id, ACTIVE_NOTE_LIFECYCLE],
                )
                .map_err(|source| CatalogError::sql("missing note confirmation update", source))?;
            if updated_rows == 0 {
                continue;
            }
            let confirmation_count: i64 = transaction
                .query_row(
                    "SELECT missing_scan_count FROM notes WHERE note_id = ?1",
                    [&note_id],
                    |row| row.get(0),
                )
                .map_err(|source| CatalogError::sql("missing note confirmation read", source))?;
            if confirmation_count < MISSING_SCAN_CONFIRMATION_COUNT {
                continue;
            }
            transaction
                .execute("DELETE FROM note_search WHERE note_id = ?1", [&note_id])
                .map_err(|source| CatalogError::sql("confirmed missing search cleanup", source))?;
            transaction
                .execute(
                    "DELETE FROM notes WHERE note_id = ?1 AND lifecycle = ?2",
                    params![note_id, ACTIVE_NOTE_LIFECYCLE],
                )
                .map_err(|source| CatalogError::sql("confirmed missing note cleanup", source))?;
            removed_count += 1;
        }
        if removed_count > 0 {
            transaction
                .execute(
                    "DELETE FROM tags
                     WHERE NOT EXISTS (
                         SELECT 1 FROM note_tags WHERE note_tags.tag_id = tags.tag_id
                     )",
                    [],
                )
                .map_err(|source| CatalogError::sql("orphaned tag cleanup", source))?;
        }
        transaction.commit().map_err(|source| {
            CatalogError::sql("missing note confirmation transaction commit", source)
        })?;
        Ok(removed_count)
    }

    /// 将已完成磁盘移动的活动笔记标记为 Trash；metadata 不会被删除。
    pub fn record_note_trashed(
        &self,
        note_id: NoteId,
        trash_relative_path: &Path,
        deleted_at: SystemTime,
    ) -> Result<TrashEntry, CatalogError> {
        let deleted_at_nanoseconds = system_time_to_nanoseconds(deleted_at)?;
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("trash record transaction start", source))?;
        let original_relative_path: String = transaction
            .query_row(
                "SELECT relative_path FROM notes WHERE note_id = ?1 AND lifecycle = ?2",
                params![note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
                |row| row.get(0),
            )
            .map_err(|source| CatalogError::sql("trash source note query", source))?;
        let updated_rows = transaction
            .execute(
                "UPDATE notes SET lifecycle = ?1, relative_path = ?2 WHERE note_id = ?3 AND lifecycle = ?4",
                params![
                    TRASHED_NOTE_LIFECYCLE,
                    trash_relative_path.to_string_lossy(),
                    note_id.to_string(),
                    ACTIVE_NOTE_LIFECYCLE,
                ],
            )
            .map_err(|source| CatalogError::sql("trash note lifecycle update", source))?;
        if updated_rows != 1 {
            return Err(CatalogError::InvalidStoredValue {
                column: "active_note_id",
                value: note_id.to_string(),
            });
        }
        transaction
            .execute(
                "INSERT INTO trash_entries (
                    note_id, original_relative_path, trash_relative_path, deleted_at_ns
                ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    note_id.to_string(),
                    original_relative_path,
                    trash_relative_path.to_string_lossy(),
                    deleted_at_nanoseconds,
                ],
            )
            .map_err(|source| CatalogError::sql("trash entry insert", source))?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("trash record transaction commit", source))?;
        Ok(TrashEntry {
            note_id,
            original_relative_path: original_relative_path.into(),
            trash_relative_path: trash_relative_path.to_path_buf(),
            deleted_at,
        })
    }

    /// 读取精确 Trash entry；不存在时返回 `None`，调用方不得用路径猜测回收目标。
    pub fn trash_entry(&self, note_id: NoteId) -> Result<Option<TrashEntry>, CatalogError> {
        self.connection()
            .query_row(
                "SELECT note_id, original_relative_path, trash_relative_path, deleted_at_ns
                 FROM trash_entries WHERE note_id = ?1",
                [note_id.to_string()],
                trash_entry_from_row,
            )
            .optional()
            .map_err(|source| CatalogError::sql("trash entry query", source))
    }

    /// 解析当前 Trash 的固定目标列表，供批量清空在执行前冻结范围。
    pub fn trash_entries(&self) -> Result<Vec<TrashEntry>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT note_id, original_relative_path, trash_relative_path, deleted_at_ns
                 FROM trash_entries ORDER BY deleted_at_ns ASC, note_id ASC",
            )
            .map_err(|source| CatalogError::sql("trash entries query preparation", source))?;
        statement
            .query_map([], trash_entry_from_row)
            .map_err(|source| CatalogError::sql("trash entries query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("trash entries row read", source))
    }

    /// 在完成磁盘 restore 后恢复 catalog 生命周期和原相对路径。
    pub fn restore_trashed_note(&self, note_id: NoteId) -> Result<TrashEntry, CatalogError> {
        self.restore_trashed_note_to_path(note_id, None)
    }

    /// 在完成磁盘 restore 后恢复 catalog 生命周期；可显式指定冲突后的新相对路径。
    pub fn restore_trashed_note_to_path(
        &self,
        note_id: NoteId,
        restored_relative_path: Option<&Path>,
    ) -> Result<TrashEntry, CatalogError> {
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("trash restore transaction start", source))?;
        let entry = transaction
            .query_row(
                "SELECT note_id, original_relative_path, trash_relative_path, deleted_at_ns
                 FROM trash_entries WHERE note_id = ?1",
                [note_id.to_string()],
                trash_entry_from_row,
            )
            .map_err(|source| CatalogError::sql("trash restore entry query", source))?;
        let updated_rows = transaction
            .execute(
                "UPDATE notes SET lifecycle = ?1, relative_path = ?2 WHERE note_id = ?3 AND lifecycle = ?4",
                params![
                    ACTIVE_NOTE_LIFECYCLE,
                    restored_relative_path
                        .unwrap_or(&entry.original_relative_path)
                        .to_string_lossy(),
                    note_id.to_string(),
                    TRASHED_NOTE_LIFECYCLE,
                ],
            )
            .map_err(|source| CatalogError::sql("trash restore note update", source))?;
        if updated_rows != 1 {
            return Err(CatalogError::InvalidStoredValue {
                column: "trashed_note_id",
                value: note_id.to_string(),
            });
        }
        transaction
            .execute("DELETE FROM trash_entries WHERE note_id = ?1", [note_id.to_string()])
            .map_err(|source| CatalogError::sql("trash entry delete after restore", source))?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("trash restore transaction commit", source))?;
        Ok(entry)
    }

    /// 在文件已进入受控删除暂存位置后，永久删除其精确 catalog entry 与 metadata。
    pub fn permanently_delete_trashed_note(
        &self,
        note_id: NoteId,
    ) -> Result<TrashEntry, CatalogError> {
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("permanent deletion transaction start", source))?;
        let entry = transaction
            .query_row(
                "SELECT note_id, original_relative_path, trash_relative_path, deleted_at_ns
                 FROM trash_entries WHERE note_id = ?1",
                [note_id.to_string()],
                trash_entry_from_row,
            )
            .map_err(|source| CatalogError::sql("permanent deletion entry query", source))?;
        transaction
            .execute("DELETE FROM note_search WHERE note_id = ?1", [note_id.to_string()])
            .map_err(|source| CatalogError::sql("permanent deletion search cleanup", source))?;
        let deleted_rows = transaction
            .execute(
                "DELETE FROM notes WHERE note_id = ?1 AND lifecycle = ?2",
                params![note_id.to_string(), TRASHED_NOTE_LIFECYCLE],
            )
            .map_err(|source| CatalogError::sql("permanent deletion note cleanup", source))?;
        if deleted_rows != 1 {
            return Err(CatalogError::InvalidStoredValue {
                column: "trashed_note_id",
                value: note_id.to_string(),
            });
        }
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("permanent deletion transaction commit", source))?;
        Ok(entry)
    }
}

fn note_encryption_to_database(encryption: NoteEncryption) -> i64 {
    match encryption {
        NoteEncryption::Unencrypted => NOTE_ENCRYPTION_UNENCRYPTED,
        NoteEncryption::Encrypted => NOTE_ENCRYPTION_ENCRYPTED,
    }
}

#[derive(Debug)]
struct StoredEditorMetadata {
    note_id: String,
    created_nanoseconds: i64,
    modified_nanoseconds: i64,
    encryption: i64,
    title_initialization_pending: bool,
}

impl TryFrom<StoredEditorMetadata> for NoteEditorMetadata {
    type Error = CatalogError;

    fn try_from(stored_metadata: StoredEditorMetadata) -> Result<Self, Self::Error> {
        let note_id =
            Uuid::parse_str(&stored_metadata.note_id).map(NoteId::from).map_err(|_| {
                CatalogError::InvalidStoredValue {
                    column: "note_id",
                    value: stored_metadata.note_id,
                }
            })?;
        let created_at =
            nanoseconds_to_system_time(stored_metadata.created_nanoseconds).map_err(|_| {
                CatalogError::InvalidStoredValue {
                    column: "created_ns",
                    value: stored_metadata.created_nanoseconds.to_string(),
                }
            })?;
        let modified_at = nanoseconds_to_system_time(stored_metadata.modified_nanoseconds)?;
        let encryption = note_encryption_from_database(stored_metadata.encryption)?;
        let title_initialization = if stored_metadata.title_initialization_pending {
            TitleInitialization::AwaitingFirstCommit
        } else {
            TitleInitialization::Independent
        };
        Ok(Self { note_id, created_at, modified_at, encryption, title_initialization })
    }
}

fn editor_metadata_from_row(row: &Row<'_>) -> rusqlite::Result<StoredEditorMetadata> {
    Ok(StoredEditorMetadata {
        note_id: row.get(0)?,
        created_nanoseconds: row.get(1)?,
        modified_nanoseconds: row.get(2)?,
        encryption: row.get(3)?,
        title_initialization_pending: row.get(4)?,
    })
}

fn update_stored_note_title(
    transaction: &rusqlite::Transaction<'_>,
    note_id: NoteId,
    title: &str,
) -> Result<(), CatalogError> {
    let updated_rows = transaction
        .execute(
            "UPDATE notes SET title = ?1 WHERE note_id = ?2 AND lifecycle = ?3",
            params![title, note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
        )
        .map_err(|source| CatalogError::sql("note title update", source))?;
    if updated_rows != 1 {
        return Err(CatalogError::InvalidStoredValue {
            column: "active_note_id",
            value: note_id.to_string(),
        });
    }
    transaction
        .execute(
            "UPDATE note_search SET title = ?1 WHERE note_id = ?2",
            params![title, note_id.to_string()],
        )
        .map_err(|source| CatalogError::sql("note title search refresh", source))?;
    Ok(())
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

fn trash_entry_from_row(row: &Row<'_>) -> rusqlite::Result<TrashEntry> {
    let note_id: String = row.get(0)?;
    let note_id = Uuid::parse_str(&note_id).map(NoteId::from).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            "invalid note identifier".into(),
        )
    })?;
    let deleted_at_nanoseconds: i64 = row.get(3)?;
    let deleted_at = nanoseconds_to_system_time(deleted_at_nanoseconds).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Integer,
            error.to_string().into(),
        )
    })?;
    Ok(TrashEntry {
        note_id,
        original_relative_path: row.get::<_, String>(1)?.into(),
        trash_relative_path: row.get::<_, String>(2)?.into(),
        deleted_at,
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

fn note_encryption_from_database(value: i64) -> Result<NoteEncryption, CatalogError> {
    match value {
        NOTE_ENCRYPTION_UNENCRYPTED => Ok(NoteEncryption::Unencrypted),
        NOTE_ENCRYPTION_ENCRYPTED => Ok(NoteEncryption::Encrypted),
        _ => {
            Err(CatalogError::InvalidStoredValue { column: "encryption", value: value.to_string() })
        }
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

    use super::{CatalogNote, note_encryption_from_database};
    use crate::domain::{NoteEditorMetadata, NoteEncryption, TitleInitialization};
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
            vec![CatalogNote { starred: true, ..catalog_note(note_id, "renamed.md", "First") }]
        );
    }

    #[test]
    fn title_initialization_is_explicit_and_completes_with_the_first_title_commit() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .create_active_note(
                &catalog_note(note_id, "new.md", "未命名 1"),
                NoteEncryption::Unencrypted,
                TitleInitialization::AwaitingFirstCommit,
            )
            .expect("new note should be created in title initialization state");

        assert_eq!(
            catalog
                .note_editor_metadata(note_id)
                .expect("metadata should query")
                .expect("metadata should exist")
                .title_initialization,
            TitleInitialization::AwaitingFirstCommit
        );

        catalog
            .complete_title_initialization(note_id, Some("项目路线图"))
            .expect("first title commit should complete initialization");

        let note = catalog
            .active_note(note_id)
            .expect("note lookup should succeed")
            .expect("note should remain active");
        assert_eq!(note.title, "项目路线图");
        assert_eq!(
            catalog
                .note_editor_metadata(note_id)
                .expect("metadata should query")
                .expect("metadata should exist")
                .title_initialization,
            TitleInitialization::Independent
        );
    }

    #[test]
    fn later_initialization_claim_cannot_overwrite_the_first_winner() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .create_active_note(
                &catalog_note(note_id, "race.md", "未命名 1"),
                NoteEncryption::Unencrypted,
                TitleInitialization::AwaitingFirstCommit,
            )
            .expect("new note should await its first title commit");

        assert!(
            catalog
                .complete_title_initialization(note_id, Some("正文先提交"))
                .expect("first claim should succeed")
        );
        assert!(
            !catalog
                .complete_title_initialization(note_id, Some("标题栏后提交"))
                .expect("later claim should be a successful no-op")
        );
        assert_eq!(
            catalog
                .active_note(note_id)
                .expect("note lookup should succeed")
                .expect("note should remain active")
                .title,
            "正文先提交"
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

    #[test]
    fn editor_metadata_round_trips_creation_time_and_encryption_without_changing_scan_model() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .upsert_active_note(&catalog_note(note_id, "metadata.md", "Metadata"))
            .expect("note should insert");

        let metadata = catalog
            .note_editor_metadata(note_id)
            .expect("metadata should query")
            .expect("metadata should exist");
        assert_eq!(
            metadata,
            NoteEditorMetadata {
                note_id,
                created_at: UNIX_EPOCH + Duration::from_secs(1),
                modified_at: UNIX_EPOCH + Duration::from_secs(1),
                encryption: NoteEncryption::Unencrypted,
                title_initialization: TitleInitialization::Independent,
            }
        );
    }

    #[test]
    fn invalid_encryption_value_is_rejected_instead_of_being_treated_as_plaintext() {
        assert!(matches!(
            note_encryption_from_database(9),
            Err(crate::CatalogError::InvalidStoredValue { column: "encryption", .. })
        ));
    }
}
