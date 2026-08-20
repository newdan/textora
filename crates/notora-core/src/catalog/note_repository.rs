use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, Row, params};
use uuid::Uuid;

use crate::domain::{
    NoteEncryption, NoteFileNameBinding, NoteFileNameMetadata, TitleInitialization,
};
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
const FILE_NAME_BINDING_LEGACY_UNMANAGED: i64 = 0;
const FILE_NAME_BINDING_TITLE_BOUND: i64 = 1;
const FILE_NAME_BINDING_OPAQUE: i64 = 2;
const PATH_OPERATION_KIND_TITLE_RENAME: i64 = 0;
const PATH_OPERATION_KIND_DIRECTORY_MOVE: i64 = 1;
const PATH_OPERATION_KIND_EXTERNAL_RENAME: i64 = 2;
const PATH_OPERATION_KIND_MIGRATION: i64 = 3;
const PATH_OPERATION_STATE_PREPARED: i64 = 0;
const PATH_OPERATION_STATE_MOVED: i64 = 1;
const PATH_OPERATION_STATE_COMMITTED: i64 = 2;
const PATH_OPERATION_STATE_ROLLED_BACK: i64 = 3;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePathOperationKind {
    TitleRename,
    DirectoryMove,
    ExternalRename,
    Migration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePathOperationState {
    Prepared,
    Moved,
    Committed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotePathOperation {
    pub operation_id: Uuid,
    pub note_id: NoteId,
    pub kind: NotePathOperationKind,
    pub source_relative_path: PathBuf,
    pub target_relative_path: PathBuf,
    pub expected_title_revision: u64,
    pub state: NotePathOperationState,
}

impl Catalog {
    /// 接受已经在 Finder 中发生的路径变化，并在 basename 改变时反向更新普通笔记标题。
    pub fn apply_external_note_relocation(
        &self,
        note_id: NoteId,
        relative_path: &Path,
        external_title: Option<&str>,
    ) -> Result<(), CatalogError> {
        let transaction = self.connection().unchecked_transaction().map_err(|source| {
            CatalogError::sql("external note relocation transaction start", source)
        })?;
        let binding: i64 = transaction
            .query_row(
                "SELECT file_name_binding FROM notes WHERE note_id = ?1 AND lifecycle = ?2",
                params![note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
                |row| row.get(0),
            )
            .map_err(|source| CatalogError::sql("external note relocation binding read", source))?;
        let title_may_change = binding != FILE_NAME_BINDING_OPAQUE;
        if let Some(title) = external_title.filter(|_| title_may_change) {
            transaction
                .execute(
                    "UPDATE notes
                     SET relative_path = ?1,
                         title = ?2,
                         file_name_binding = ?3,
                         file_name_disambiguator = 1,
                         title_revision = title_revision + 1,
                         missing_scan_count = 0
                     WHERE note_id = ?4 AND lifecycle = ?5",
                    params![
                        relative_path.to_string_lossy(),
                        title,
                        FILE_NAME_BINDING_TITLE_BOUND,
                        note_id.to_string(),
                        ACTIVE_NOTE_LIFECYCLE,
                    ],
                )
                .map_err(|source| CatalogError::sql("external note relocation update", source))?;
            transaction
                .execute(
                    "UPDATE note_search SET title = ?1, relative_path = ?2 WHERE note_id = ?3",
                    params![title, relative_path.to_string_lossy(), note_id.to_string()],
                )
                .map_err(|source| {
                    CatalogError::sql("external note relocation search refresh", source)
                })?;
        } else {
            transaction
                .execute(
                    "UPDATE notes SET relative_path = ?1, missing_scan_count = 0
                     WHERE note_id = ?2 AND lifecycle = ?3",
                    params![
                        relative_path.to_string_lossy(),
                        note_id.to_string(),
                        ACTIVE_NOTE_LIFECYCLE,
                    ],
                )
                .map_err(|source| CatalogError::sql("external note move update", source))?;
            transaction
                .execute(
                    "UPDATE note_search SET relative_path = ?1 WHERE note_id = ?2",
                    params![relative_path.to_string_lossy(), note_id.to_string()],
                )
                .map_err(|source| CatalogError::sql("external note move search refresh", source))?;
        }
        transaction.commit().map_err(|source| {
            CatalogError::sql("external note relocation transaction commit", source)
        })
    }

    /// 原子提交不允许泄漏到路径的标题；`None` 表示 revision 已过期。
    pub fn commit_title_metadata(
        &self,
        note_id: NoteId,
        expected_title_revision: u64,
        title: &str,
    ) -> Result<Option<u64>, CatalogError> {
        let expected_revision = i64::try_from(expected_title_revision).map_err(|_| {
            CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: expected_title_revision.to_string(),
            }
        })?;
        let next_title_revision = expected_title_revision.checked_add(1).ok_or_else(|| {
            CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: expected_title_revision.to_string(),
            }
        })?;
        let next_revision =
            i64::try_from(next_title_revision).map_err(|_| CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: next_title_revision.to_string(),
            })?;
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("title metadata transaction start", source))?;
        let updated_rows = transaction
            .execute(
                "UPDATE notes SET title = ?1, title_revision = ?2
                 WHERE note_id = ?3 AND lifecycle = ?4 AND title_revision = ?5",
                params![
                    title,
                    next_revision,
                    note_id.to_string(),
                    ACTIVE_NOTE_LIFECYCLE,
                    expected_revision,
                ],
            )
            .map_err(|source| CatalogError::sql("title metadata note update", source))?;
        if updated_rows != 1 {
            return Ok(None);
        }
        transaction
            .execute(
                "UPDATE note_search SET title = ?1 WHERE note_id = ?2",
                params![title, note_id.to_string()],
            )
            .map_err(|source| CatalogError::sql("title metadata search refresh", source))?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("title metadata transaction commit", source))?;
        Ok(Some(next_title_revision))
    }

    /// 原子提交标题派生路径；`None` 表示调用方的 title revision 已过期。
    pub fn commit_title_bound_path(
        &self,
        note_id: NoteId,
        expected_title_revision: u64,
        title: &str,
        relative_path: &Path,
        disambiguator: u32,
    ) -> Result<Option<u64>, CatalogError> {
        if disambiguator == 0 {
            return Err(CatalogError::InvalidStoredValue {
                column: "file_name_disambiguator",
                value: disambiguator.to_string(),
            });
        }
        let expected_revision = i64::try_from(expected_title_revision).map_err(|_| {
            CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: expected_title_revision.to_string(),
            }
        })?;
        let next_title_revision = expected_title_revision.checked_add(1).ok_or_else(|| {
            CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: expected_title_revision.to_string(),
            }
        })?;
        let next_revision =
            i64::try_from(next_title_revision).map_err(|_| CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: next_title_revision.to_string(),
            })?;
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("title path commit transaction start", source))?;
        let actual_revision: Option<i64> = transaction
            .query_row(
                "SELECT title_revision FROM notes WHERE note_id = ?1 AND lifecycle = ?2",
                params![note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| CatalogError::sql("title revision read", source))?;
        let Some(actual_revision) = actual_revision else {
            return Err(CatalogError::InvalidStoredValue {
                column: "active_note_id",
                value: note_id.to_string(),
            });
        };
        if actual_revision != expected_revision {
            return Ok(None);
        }

        let updated_rows = transaction
            .execute(
                "UPDATE notes
                 SET title = ?1,
                     relative_path = ?2,
                     file_name_binding = ?3,
                     file_name_disambiguator = ?4,
                     title_revision = ?5
                 WHERE note_id = ?6 AND lifecycle = ?7 AND title_revision = ?8",
                params![
                    title,
                    relative_path.to_string_lossy(),
                    FILE_NAME_BINDING_TITLE_BOUND,
                    i64::from(disambiguator),
                    next_revision,
                    note_id.to_string(),
                    ACTIVE_NOTE_LIFECYCLE,
                    expected_revision,
                ],
            )
            .map_err(|source| CatalogError::sql("title path note update", source))?;
        if updated_rows != 1 {
            return Ok(None);
        }
        transaction
            .execute(
                "UPDATE note_search SET title = ?1, relative_path = ?2 WHERE note_id = ?3",
                params![title, relative_path.to_string_lossy(), note_id.to_string()],
            )
            .map_err(|source| CatalogError::sql("title path search refresh", source))?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("title path commit transaction commit", source))?;
        Ok(Some(next_title_revision))
    }

    pub fn prepare_note_path_operation(
        &self,
        operation: &NotePathOperation,
    ) -> Result<(), CatalogError> {
        if operation.state != NotePathOperationState::Prepared {
            return Err(CatalogError::InvalidStoredValue {
                column: "note_path_operation_state",
                value: format!("{:?}", operation.state),
            });
        }
        let expected_title_revision =
            i64::try_from(operation.expected_title_revision).map_err(|_| {
                CatalogError::InvalidStoredValue {
                    column: "expected_title_revision",
                    value: operation.expected_title_revision.to_string(),
                }
            })?;
        self.connection()
            .execute(
                "INSERT INTO note_path_operations (
                    operation_id, note_id, kind, source_relative_path, target_relative_path,
                    expected_title_revision, state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    operation.operation_id.to_string(),
                    operation.note_id.to_string(),
                    note_path_operation_kind_to_database(operation.kind),
                    operation.source_relative_path.to_string_lossy(),
                    operation.target_relative_path.to_string_lossy(),
                    expected_title_revision,
                    note_path_operation_state_to_database(operation.state),
                ],
            )
            .map_err(|source| CatalogError::sql("note path operation preparation", source))?;
        Ok(())
    }

    pub fn update_note_path_operation_state(
        &self,
        operation_id: Uuid,
        state: NotePathOperationState,
    ) -> Result<(), CatalogError> {
        let updated_rows = self
            .connection()
            .execute(
                "UPDATE note_path_operations SET state = ?1 WHERE operation_id = ?2",
                params![note_path_operation_state_to_database(state), operation_id.to_string()],
            )
            .map_err(|source| CatalogError::sql("note path operation state update", source))?;
        if updated_rows == 1 {
            return Ok(());
        }
        Err(CatalogError::InvalidStoredValue {
            column: "operation_id",
            value: operation_id.to_string(),
        })
    }

    pub fn unfinished_note_path_operations(&self) -> Result<Vec<NotePathOperation>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT operation_id, note_id, kind, source_relative_path, target_relative_path,
                        expected_title_revision, state
                 FROM note_path_operations
                 WHERE state IN (?1, ?2)
                 ORDER BY rowid ASC",
            )
            .map_err(|source| {
                CatalogError::sql("unfinished note path operations query preparation", source)
            })?;
        let operations = statement
            .query_map(
                params![PATH_OPERATION_STATE_PREPARED, PATH_OPERATION_STATE_MOVED],
                stored_note_path_operation_from_row,
            )
            .map_err(|source| CatalogError::sql("unfinished note path operations query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| {
                CatalogError::sql("unfinished note path operations row read", source)
            })?;
        operations.into_iter().map(NotePathOperation::try_from).collect()
    }

    pub fn note_file_name_metadata(
        &self,
        note_id: NoteId,
    ) -> Result<Option<NoteFileNameMetadata>, CatalogError> {
        self.connection()
            .query_row(
                "SELECT note_id, file_name_binding, file_name_disambiguator, title_revision
                 FROM notes WHERE note_id = ?1",
                [note_id.to_string()],
                |row| {
                    Ok(StoredFileNameMetadata {
                        note_id: row.get(0)?,
                        binding: row.get(1)?,
                        disambiguator: row.get(2)?,
                        title_revision: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|source| CatalogError::sql("note file name metadata query", source))?
            .map(NoteFileNameMetadata::try_from)
            .transpose()
    }

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
                    note_id, relative_path, kind, title, excerpt, created_ns, modified_ns,
                    file_size, content_hash, encryption, lifecycle, file_name_binding,
                    file_name_disambiguator, title_revision
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, 1, 0)",
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
                    FILE_NAME_BINDING_TITLE_BOUND,
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
        self.upsert_active_note_with_encryption(note, NoteEncryption::Unencrypted)
    }

    pub(crate) fn upsert_discovered_note(
        &self,
        note: &CatalogNote,
        encryption: NoteEncryption,
    ) -> Result<(), CatalogError> {
        self.upsert_active_note_with_encryption(note, encryption)
    }

    fn upsert_active_note_with_encryption(
        &self,
        note: &CatalogNote,
        encryption: NoteEncryption,
    ) -> Result<(), CatalogError> {
        let modified_nanoseconds = system_time_to_nanoseconds(note.modified_at)?;
        let file_size =
            i64::try_from(note.file_size).map_err(|_| CatalogError::InvalidStoredValue {
                column: "file_size",
                value: note.file_size.to_string(),
            })?;
        self.connection()
            .execute(
                "INSERT INTO notes (
                    note_id, relative_path, kind, title, excerpt, created_ns, modified_ns,
                    file_size, content_hash, encryption, lifecycle, file_name_binding,
                    file_name_disambiguator, title_revision
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, 1, 0)
                ON CONFLICT(note_id) DO UPDATE SET
                    relative_path = excluded.relative_path,
                    kind = excluded.kind,
                    excerpt = excluded.excerpt,
                    modified_ns = excluded.modified_ns,
                    file_size = excluded.file_size,
                    content_hash = excluded.content_hash,
                    encryption = excluded.encryption,
                    file_name_binding = excluded.file_name_binding,
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
                    note_encryption_to_database(encryption),
                    ACTIVE_NOTE_LIFECYCLE,
                    FILE_NAME_BINDING_TITLE_BOUND,
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
                               WHERE initialization.note_id = notes.note_id),
                        file_name_binding, file_name_disambiguator, title_revision
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

fn note_path_operation_kind_to_database(kind: NotePathOperationKind) -> i64 {
    match kind {
        NotePathOperationKind::TitleRename => PATH_OPERATION_KIND_TITLE_RENAME,
        NotePathOperationKind::DirectoryMove => PATH_OPERATION_KIND_DIRECTORY_MOVE,
        NotePathOperationKind::ExternalRename => PATH_OPERATION_KIND_EXTERNAL_RENAME,
        NotePathOperationKind::Migration => PATH_OPERATION_KIND_MIGRATION,
    }
}

fn note_path_operation_state_to_database(state: NotePathOperationState) -> i64 {
    match state {
        NotePathOperationState::Prepared => PATH_OPERATION_STATE_PREPARED,
        NotePathOperationState::Moved => PATH_OPERATION_STATE_MOVED,
        NotePathOperationState::Committed => PATH_OPERATION_STATE_COMMITTED,
        NotePathOperationState::RolledBack => PATH_OPERATION_STATE_ROLLED_BACK,
    }
}

#[derive(Debug)]
struct StoredNotePathOperation {
    operation_id: String,
    note_id: String,
    kind: i64,
    source_relative_path: String,
    target_relative_path: String,
    expected_title_revision: i64,
    state: i64,
}

impl TryFrom<StoredNotePathOperation> for NotePathOperation {
    type Error = CatalogError;

    fn try_from(stored: StoredNotePathOperation) -> Result<Self, Self::Error> {
        let operation_id = Uuid::parse_str(&stored.operation_id).map_err(|_| {
            CatalogError::InvalidStoredValue { column: "operation_id", value: stored.operation_id }
        })?;
        let note_id = Uuid::parse_str(&stored.note_id).map(NoteId::from).map_err(|_| {
            CatalogError::InvalidStoredValue { column: "note_id", value: stored.note_id }
        })?;
        let kind = match stored.kind {
            PATH_OPERATION_KIND_TITLE_RENAME => NotePathOperationKind::TitleRename,
            PATH_OPERATION_KIND_DIRECTORY_MOVE => NotePathOperationKind::DirectoryMove,
            PATH_OPERATION_KIND_EXTERNAL_RENAME => NotePathOperationKind::ExternalRename,
            PATH_OPERATION_KIND_MIGRATION => NotePathOperationKind::Migration,
            _ => {
                return Err(CatalogError::InvalidStoredValue {
                    column: "note_path_operation_kind",
                    value: stored.kind.to_string(),
                });
            }
        };
        let expected_title_revision =
            u64::try_from(stored.expected_title_revision).map_err(|_| {
                CatalogError::InvalidStoredValue {
                    column: "expected_title_revision",
                    value: stored.expected_title_revision.to_string(),
                }
            })?;
        let state = match stored.state {
            PATH_OPERATION_STATE_PREPARED => NotePathOperationState::Prepared,
            PATH_OPERATION_STATE_MOVED => NotePathOperationState::Moved,
            PATH_OPERATION_STATE_COMMITTED => NotePathOperationState::Committed,
            PATH_OPERATION_STATE_ROLLED_BACK => NotePathOperationState::RolledBack,
            _ => {
                return Err(CatalogError::InvalidStoredValue {
                    column: "note_path_operation_state",
                    value: stored.state.to_string(),
                });
            }
        };
        Ok(Self {
            operation_id,
            note_id,
            kind,
            source_relative_path: stored.source_relative_path.into(),
            target_relative_path: stored.target_relative_path.into(),
            expected_title_revision,
            state,
        })
    }
}

fn stored_note_path_operation_from_row(row: &Row<'_>) -> rusqlite::Result<StoredNotePathOperation> {
    Ok(StoredNotePathOperation {
        operation_id: row.get(0)?,
        note_id: row.get(1)?,
        kind: row.get(2)?,
        source_relative_path: row.get(3)?,
        target_relative_path: row.get(4)?,
        expected_title_revision: row.get(5)?,
        state: row.get(6)?,
    })
}

#[derive(Debug)]
struct StoredFileNameMetadata {
    note_id: String,
    binding: i64,
    disambiguator: i64,
    title_revision: i64,
}

impl TryFrom<StoredFileNameMetadata> for NoteFileNameMetadata {
    type Error = CatalogError;

    fn try_from(stored: StoredFileNameMetadata) -> Result<Self, Self::Error> {
        let note_id = Uuid::parse_str(&stored.note_id).map(NoteId::from).map_err(|_| {
            CatalogError::InvalidStoredValue { column: "note_id", value: stored.note_id }
        })?;
        let disambiguator =
            u32::try_from(stored.disambiguator).map_err(|_| CatalogError::InvalidStoredValue {
                column: "file_name_disambiguator",
                value: stored.disambiguator.to_string(),
            })?;
        if disambiguator == 0 {
            return Err(CatalogError::InvalidStoredValue {
                column: "file_name_disambiguator",
                value: stored.disambiguator.to_string(),
            });
        }
        let binding = match stored.binding {
            FILE_NAME_BINDING_LEGACY_UNMANAGED => NoteFileNameBinding::LegacyUnmanaged,
            FILE_NAME_BINDING_TITLE_BOUND => NoteFileNameBinding::TitleBound { disambiguator },
            FILE_NAME_BINDING_OPAQUE => NoteFileNameBinding::Opaque,
            _ => {
                return Err(CatalogError::InvalidStoredValue {
                    column: "file_name_binding",
                    value: stored.binding.to_string(),
                });
            }
        };
        let title_revision =
            u64::try_from(stored.title_revision).map_err(|_| CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: stored.title_revision.to_string(),
            })?;
        Ok(Self { note_id, binding, title_revision })
    }
}

#[derive(Debug)]
struct StoredEditorMetadata {
    note_id: String,
    created_nanoseconds: i64,
    modified_nanoseconds: i64,
    encryption: i64,
    title_initialization_pending: bool,
    file_name_binding: i64,
    file_name_disambiguator: i64,
    title_revision: i64,
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
        let file_name_binding = note_file_name_binding_from_database(
            stored_metadata.file_name_binding,
            stored_metadata.file_name_disambiguator,
        )?;
        let title_revision = u64::try_from(stored_metadata.title_revision).map_err(|_| {
            CatalogError::InvalidStoredValue {
                column: "title_revision",
                value: stored_metadata.title_revision.to_string(),
            }
        })?;
        Ok(Self {
            note_id,
            created_at,
            modified_at,
            encryption,
            title_initialization,
            file_name_binding,
            title_revision,
        })
    }
}

fn editor_metadata_from_row(row: &Row<'_>) -> rusqlite::Result<StoredEditorMetadata> {
    Ok(StoredEditorMetadata {
        note_id: row.get(0)?,
        created_nanoseconds: row.get(1)?,
        modified_nanoseconds: row.get(2)?,
        encryption: row.get(3)?,
        title_initialization_pending: row.get(4)?,
        file_name_binding: row.get(5)?,
        file_name_disambiguator: row.get(6)?,
        title_revision: row.get(7)?,
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

fn note_file_name_binding_from_database(
    binding: i64,
    disambiguator: i64,
) -> Result<NoteFileNameBinding, CatalogError> {
    let disambiguator =
        u32::try_from(disambiguator).map_err(|_| CatalogError::InvalidStoredValue {
            column: "file_name_disambiguator",
            value: disambiguator.to_string(),
        })?;
    if disambiguator == 0 {
        return Err(CatalogError::InvalidStoredValue {
            column: "file_name_disambiguator",
            value: disambiguator.to_string(),
        });
    }
    match binding {
        FILE_NAME_BINDING_LEGACY_UNMANAGED => Ok(NoteFileNameBinding::LegacyUnmanaged),
        FILE_NAME_BINDING_TITLE_BOUND => Ok(NoteFileNameBinding::TitleBound { disambiguator }),
        FILE_NAME_BINDING_OPAQUE => Ok(NoteFileNameBinding::Opaque),
        _ => Err(CatalogError::InvalidStoredValue {
            column: "file_name_binding",
            value: binding.to_string(),
        }),
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
    use crate::domain::{
        NoteEditorMetadata, NoteEncryption, NoteFileNameBinding, NoteFileNameMetadata,
        TitleInitialization,
    };
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
                file_name_binding: NoteFileNameBinding::TitleBound { disambiguator: 1 },
                title_revision: 0,
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

    #[test]
    fn new_scanned_and_configured_notes_persist_file_name_binding_metadata() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let scanned_note_id = NoteId::generate();
        let configured_note_id = NoteId::generate();
        let encrypted_note_id = NoteId::generate();
        catalog
            .upsert_active_note(&catalog_note(scanned_note_id, "imported.md", "imported"))
            .expect("scanned note should insert");
        catalog
            .create_active_note(
                &catalog_note(configured_note_id, "无标题.md", "无标题"),
                NoteEncryption::Unencrypted,
                TitleInitialization::AwaitingFirstCommit,
            )
            .expect("configured note should insert");
        catalog
            .create_active_note(
                &catalog_note(encrypted_note_id, "secret.md", "secret"),
                NoteEncryption::Encrypted,
                TitleInitialization::Independent,
            )
            .expect("encrypted fixture should insert");

        for note_id in [scanned_note_id, configured_note_id] {
            assert_eq!(
                catalog.note_file_name_metadata(note_id).expect("file name metadata should query"),
                Some(NoteFileNameMetadata {
                    note_id,
                    binding: NoteFileNameBinding::TitleBound { disambiguator: 1 },
                    title_revision: 0,
                })
            );
        }
        assert_eq!(
            catalog
                .note_file_name_metadata(encrypted_note_id)
                .expect("encrypted file name metadata should query"),
            Some(NoteFileNameMetadata {
                note_id: encrypted_note_id,
                binding: NoteFileNameBinding::TitleBound { disambiguator: 1 },
                title_revision: 0,
            })
        );
    }

    #[test]
    fn path_operation_states_round_trip_and_finished_operations_leave_the_pending_list() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .upsert_active_note(&catalog_note(note_id, "old.md", "Old"))
            .expect("operation note fixture should insert");
        let operation = super::NotePathOperation {
            operation_id: uuid::Uuid::new_v4(),
            note_id,
            kind: super::NotePathOperationKind::TitleRename,
            source_relative_path: "old.md".into(),
            target_relative_path: "new.md".into(),
            expected_title_revision: 0,
            state: super::NotePathOperationState::Prepared,
        };

        catalog.prepare_note_path_operation(&operation).expect("path operation should be prepared");
        assert_eq!(
            catalog.unfinished_note_path_operations().expect("prepared operations should query"),
            vec![operation.clone()]
        );

        catalog
            .update_note_path_operation_state(
                operation.operation_id,
                super::NotePathOperationState::Moved,
            )
            .expect("path operation should become moved");
        assert_eq!(
            catalog.unfinished_note_path_operations().expect("moved operations should query")[0]
                .state,
            super::NotePathOperationState::Moved
        );

        catalog
            .update_note_path_operation_state(
                operation.operation_id,
                super::NotePathOperationState::Committed,
            )
            .expect("path operation should become committed");
        assert!(
            catalog
                .unfinished_note_path_operations()
                .expect("finished operations should query")
                .is_empty()
        );
    }

    #[test]
    fn only_one_unfinished_path_operation_can_claim_a_note_or_target() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let first_note_id = NoteId::generate();
        let second_note_id = NoteId::generate();
        for (note_id, path) in [(first_note_id, "first.md"), (second_note_id, "second.md")] {
            catalog
                .upsert_active_note(&catalog_note(note_id, path, path))
                .expect("operation note fixture should insert");
        }
        let operation = |note_id, source: &str| super::NotePathOperation {
            operation_id: uuid::Uuid::new_v4(),
            note_id,
            kind: super::NotePathOperationKind::DirectoryMove,
            source_relative_path: source.into(),
            target_relative_path: "shared.md".into(),
            expected_title_revision: 0,
            state: super::NotePathOperationState::Prepared,
        };

        catalog
            .prepare_note_path_operation(&operation(first_note_id, "first.md"))
            .expect("first operation should prepare");
        assert!(
            catalog.prepare_note_path_operation(&operation(first_note_id, "first.md")).is_err()
        );
        assert!(
            catalog.prepare_note_path_operation(&operation(second_note_id, "second.md")).is_err()
        );
    }

    #[test]
    fn title_path_commit_updates_note_naming_metadata_and_search_in_one_transaction() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        catalog
            .upsert_active_note(&catalog_note(note_id, "old.md", "Old"))
            .expect("title update note fixture should insert");
        catalog
            .index_note_batch(&[crate::catalog::SearchIndexEntry {
                note_id,
                title: "Old".to_owned(),
                relative_path: "old.md".into(),
                body: "body remains".to_owned(),
                tags: vec!["tag remains".to_owned()],
            }])
            .expect("search fixture should index");

        assert_eq!(
            catalog
                .commit_title_bound_path(note_id, 0, "New", std::path::Path::new("New (2).md"), 2)
                .expect("title path should commit"),
            Some(1)
        );
        let note = catalog
            .active_note(note_id)
            .expect("updated note should query")
            .expect("updated note should exist");
        assert_eq!(note.title, "New");
        assert_eq!(note.relative_path, std::path::PathBuf::from("New (2).md"));
        assert_eq!(
            catalog
                .note_file_name_metadata(note_id)
                .expect("updated naming metadata should query")
                .expect("updated naming metadata should exist"),
            NoteFileNameMetadata {
                note_id,
                binding: NoteFileNameBinding::TitleBound { disambiguator: 2 },
                title_revision: 1,
            }
        );
        let indexed: (String, String, String, String) = catalog
            .connection()
            .query_row(
                "SELECT title, relative_path, body, tags FROM note_search WHERE note_id = ?1",
                [note_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("updated search row should query");
        assert_eq!(
            indexed,
            (
                "New".to_owned(),
                "New (2).md".to_owned(),
                "body remains".to_owned(),
                "tag remains".to_owned(),
            )
        );
    }

    #[test]
    fn stale_revision_and_path_conflict_leave_title_path_and_revision_unchanged() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        let occupied_note_id = NoteId::generate();
        catalog
            .upsert_active_note(&catalog_note(note_id, "old.md", "Old"))
            .expect("title update note fixture should insert");
        catalog
            .upsert_active_note(&catalog_note(occupied_note_id, "occupied.md", "Occupied"))
            .expect("occupied path fixture should insert");

        assert_eq!(
            catalog
                .commit_title_bound_path(note_id, 9, "Stale", std::path::Path::new("stale.md"), 1)
                .expect("stale revision should be a typed no-op"),
            None
        );
        assert!(
            catalog
                .commit_title_bound_path(
                    note_id,
                    0,
                    "Partial",
                    std::path::Path::new("occupied.md"),
                    1,
                )
                .is_err()
        );

        let note = catalog
            .active_note(note_id)
            .expect("unchanged note should query")
            .expect("unchanged note should exist");
        assert_eq!(note.title, "Old");
        assert_eq!(note.relative_path, std::path::PathBuf::from("old.md"));
        assert_eq!(
            catalog
                .note_file_name_metadata(note_id)
                .expect("unchanged naming metadata should query")
                .expect("unchanged naming metadata should exist")
                .title_revision,
            0
        );
    }
}
