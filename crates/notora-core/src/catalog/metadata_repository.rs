use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Transaction, params};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::{NoteId, TagId, TagSummary};

use super::{Catalog, CatalogError};

const ACTIVE_NOTE_LIFECYCLE: i64 = 0;
const SEARCH_TAG_SEPARATOR: &str = "\n";

/// 左侧标签导航所需的稳定标签身份、展示名和活动笔记数量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagWithActiveNoteCount {
    pub tag_id: TagId,
    pub display_name: String,
    pub active_note_count: u64,
}

/// 左侧树所需的预计算导航数据；UI 不需要、也不得访问 catalog。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogNavigationTree {
    pub directories: Vec<PathBuf>,
    pub tags: Vec<TagWithActiveNoteCount>,
}

impl Catalog {
    /// 在同一个事务中切换活动笔记的星标，并返回切换后的值。
    pub fn toggle_note_starred(&self, note_id: NoteId) -> Result<bool, CatalogError> {
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("star toggle transaction start", source))?;
        let updated_rows = transaction
            .execute(
                "UPDATE notes SET starred = 1 - starred WHERE note_id = ?1 AND lifecycle = ?2",
                params![note_id.to_string(), ACTIVE_NOTE_LIFECYCLE],
            )
            .map_err(|source| CatalogError::sql("star toggle", source))?;
        if updated_rows != 1 {
            return Err(CatalogError::InvalidStoredValue {
                column: "active_note_id",
                value: note_id.to_string(),
            });
        }
        let starred = transaction
            .query_row(
                "SELECT starred FROM notes WHERE note_id = ?1",
                [note_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|source| CatalogError::sql("star toggle result", source))?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("star toggle transaction commit", source))?;
        match starred {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(CatalogError::InvalidStoredValue {
                column: "starred",
                value: value.to_string(),
            }),
        }
    }

    /// 新建标签；规范化名唯一，展示名保留规范化后的用户输入。
    pub fn create_tag(&self, display_name: &str) -> Result<TagSummary, CatalogError> {
        let tag_name = TagName::parse(display_name)?;
        let tag = TagSummary { tag_id: TagId::generate(), display_name: tag_name.display };
        self.connection()
            .execute(
                "INSERT INTO tags (tag_id, normalized_name, display_name) VALUES (?1, ?2, ?3)",
                params![tag.tag_id.to_string(), tag_name.normalized, tag.display_name],
            )
            .map_err(|source| CatalogError::sql("tag creation", source))?;
        Ok(tag)
    }

    /// 重命名标签；冲突时数据库事务不会改写原标签。
    pub fn rename_tag(&self, tag_id: TagId, display_name: &str) -> Result<(), CatalogError> {
        let tag_name = TagName::parse(display_name)?;
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("tag rename transaction start", source))?;
        let affected_note_ids = note_ids_for_tag(&transaction, tag_id)?;
        let updated_rows = transaction
            .execute(
                "UPDATE tags SET normalized_name = ?1, display_name = ?2 WHERE tag_id = ?3",
                params![tag_name.normalized, tag_name.display, tag_id.to_string()],
            )
            .map_err(|source| CatalogError::sql("tag rename", source))?;
        if updated_rows != 1 {
            return Err(CatalogError::InvalidStoredValue {
                column: "tag_id",
                value: tag_id.to_string(),
            });
        }
        refresh_search_tags(&transaction, &affected_note_ids)?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("tag rename transaction commit", source))
    }

    /// 删除标签及其关联；笔记本身及其他 metadata 保留。
    pub fn delete_tag(&self, tag_id: TagId) -> Result<bool, CatalogError> {
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("tag deletion transaction start", source))?;
        let affected_note_ids = note_ids_for_tag(&transaction, tag_id)?;
        let deleted_rows = transaction
            .execute("DELETE FROM tags WHERE tag_id = ?1", [tag_id.to_string()])
            .map_err(|source| CatalogError::sql("tag deletion", source))?;
        refresh_search_tags(&transaction, &affected_note_ids)?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("tag deletion transaction commit", source))?;
        Ok(deleted_rows == 1)
    }

    /// 为笔记关联标签。已有关联是成功且不改变状态。
    pub fn attach_tag(&self, note_id: NoteId, tag_id: TagId) -> Result<bool, CatalogError> {
        self.update_tag_attachment(note_id, tag_id, true)
    }

    /// 移除笔记的标签关联。缺失关联是成功且不改变状态。
    pub fn detach_tag(&self, note_id: NoteId, tag_id: TagId) -> Result<bool, CatalogError> {
        self.update_tag_attachment(note_id, tag_id, false)
    }

    /// 读取指定标签，供改名后的导航选择保持稳定 ID。
    pub fn tag(&self, tag_id: TagId) -> Result<Option<TagSummary>, CatalogError> {
        self.connection()
            .query_row(
                "SELECT tag_id, display_name FROM tags WHERE tag_id = ?1",
                [tag_id.to_string()],
                tag_summary_from_row,
            )
            .optional()
            .map_err(|source| CatalogError::sql("tag read", source))
    }

    /// 获取笔记的标签；Trash 笔记的 metadata 同样可读取以支持恢复。
    pub fn tags_for_note(&self, note_id: NoteId) -> Result<Vec<TagSummary>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT t.tag_id, t.display_name FROM tags AS t
                 JOIN note_tags AS nt ON nt.tag_id = t.tag_id
                 WHERE nt.note_id = ?1 ORDER BY t.normalized_name ASC",
            )
            .map_err(|source| CatalogError::sql("note tags query preparation", source))?;
        statement
            .query_map([note_id.to_string()], tag_summary_from_row)
            .map_err(|source| CatalogError::sql("note tags query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("note tags row read", source))
    }

    /// 获取标签导航 badge。Trash 笔记和只关联 Trash 的标签不会贡献数量。
    pub fn tags_with_active_note_counts(
        &self,
    ) -> Result<Vec<TagWithActiveNoteCount>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT t.tag_id, t.display_name, COUNT(n.note_id)
                 FROM tags AS t
                 LEFT JOIN note_tags AS nt ON nt.tag_id = t.tag_id
                 LEFT JOIN notes AS n ON n.note_id = nt.note_id AND n.lifecycle = ?1
                 GROUP BY t.tag_id, t.display_name, t.normalized_name
                 ORDER BY t.normalized_name ASC",
            )
            .map_err(|source| CatalogError::sql("tag badges query preparation", source))?;
        statement
            .query_map([ACTIVE_NOTE_LIFECYCLE], |row| {
                let tag_id: String = row.get(0)?;
                let active_note_count: i64 = row.get(2)?;
                Ok((tag_id, row.get::<_, String>(1)?, active_note_count))
            })
            .map_err(|source| CatalogError::sql("tag badges query", source))?
            .map(|row| {
                let (tag_id, display_name, active_note_count) =
                    row.map_err(|source| CatalogError::sql("tag badge row read", source))?;
                let tag_id = Uuid::parse_str(&tag_id).map(TagId::from).map_err(|_| {
                    CatalogError::InvalidStoredValue { column: "tag_id", value: tag_id }
                })?;
                let active_note_count = u64::try_from(active_note_count).map_err(|_| {
                    CatalogError::InvalidStoredValue {
                        column: "active_note_count",
                        value: active_note_count.to_string(),
                    }
                })?;
                Ok(TagWithActiveNoteCount { tag_id, display_name, active_note_count })
            })
            .collect()
    }

    /// 构造活动工作区的目录树与标签导航数据。仅返回含支持笔记的目录及其祖先。
    pub fn navigation_tree(&self) -> Result<CatalogNavigationTree, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT relative_path FROM notes WHERE lifecycle = ?1 ORDER BY relative_path ASC",
            )
            .map_err(|source| {
                CatalogError::sql("navigation directory query preparation", source)
            })?;
        let note_paths = statement
            .query_map([ACTIVE_NOTE_LIFECYCLE], |row| row.get::<_, String>(0))
            .map_err(|source| CatalogError::sql("navigation directory query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("navigation directory row read", source))?;
        let mut directories = std::collections::BTreeSet::new();
        for note_path in note_paths {
            let mut parent = Path::new(&note_path).parent();
            while let Some(directory) = parent {
                if directory.as_os_str().is_empty() {
                    break;
                }
                directories.insert(directory.to_path_buf());
                parent = directory.parent();
            }
        }
        Ok(CatalogNavigationTree {
            directories: directories.into_iter().collect(),
            tags: self.tags_with_active_note_counts()?,
        })
    }

    fn update_tag_attachment(
        &self,
        note_id: NoteId,
        tag_id: TagId,
        attach: bool,
    ) -> Result<bool, CatalogError> {
        if attach {
            let transaction = self
                .connection()
                .unchecked_transaction()
                .map_err(|source| CatalogError::sql("tag attachment transaction start", source))?;
            let changed_rows = transaction
                .execute(
                    "INSERT OR IGNORE INTO note_tags (note_id, tag_id)
                     SELECT ?1, ?2
                     WHERE EXISTS (SELECT 1 FROM notes WHERE note_id = ?1)
                       AND EXISTS (SELECT 1 FROM tags WHERE tag_id = ?2)",
                    params![note_id.to_string(), tag_id.to_string()],
                )
                .map_err(|source| CatalogError::sql("tag attachment", source))?;
            refresh_search_tags(&transaction, &[note_id.to_string()])?;
            transaction
                .commit()
                .map_err(|source| CatalogError::sql("tag attachment transaction commit", source))?;
            return Ok(changed_rows == 1);
        }
        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("tag detachment transaction start", source))?;
        let changed_rows = transaction
            .execute(
                "DELETE FROM note_tags WHERE note_id = ?1 AND tag_id = ?2",
                params![note_id.to_string(), tag_id.to_string()],
            )
            .map_err(|source| CatalogError::sql("tag detachment", source))?;
        refresh_search_tags(&transaction, &[note_id.to_string()])?;
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("tag detachment transaction commit", source))?;
        Ok(changed_rows == 1)
    }
}

fn note_ids_for_tag(
    transaction: &Transaction<'_>,
    tag_id: TagId,
) -> Result<Vec<String>, CatalogError> {
    let mut statement = transaction
        .prepare("SELECT note_id FROM note_tags WHERE tag_id = ?1 ORDER BY note_id")
        .map_err(|source| CatalogError::sql("tag note query preparation", source))?;
    statement
        .query_map([tag_id.to_string()], |row| row.get::<_, String>(0))
        .map_err(|source| CatalogError::sql("tag note query", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| CatalogError::sql("tag note row read", source))
}

fn refresh_search_tags(
    transaction: &Transaction<'_>,
    note_ids: &[String],
) -> Result<(), CatalogError> {
    for note_id in note_ids {
        let tags = search_tags_for_note(transaction, note_id)?;
        transaction
            .execute(
                "UPDATE note_search SET tags = ?1 WHERE note_id = ?2",
                params![tags.join(SEARCH_TAG_SEPARATOR), note_id],
            )
            .map_err(|source| CatalogError::sql("search tag refresh", source))?;
    }
    Ok(())
}

fn search_tags_for_note(
    transaction: &Transaction<'_>,
    note_id: &str,
) -> Result<Vec<String>, CatalogError> {
    let mut statement = transaction
        .prepare(
            "SELECT t.display_name FROM tags AS t
             JOIN note_tags AS nt ON nt.tag_id = t.tag_id
             WHERE nt.note_id = ?1 ORDER BY t.normalized_name ASC",
        )
        .map_err(|source| CatalogError::sql("search tag query preparation", source))?;
    statement
        .query_map([note_id], |row| row.get::<_, String>(0))
        .map_err(|source| CatalogError::sql("search tag query", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| CatalogError::sql("search tag row read", source))
}

struct TagName {
    normalized: String,
    display: String,
}

impl TagName {
    fn parse(display_name: &str) -> Result<Self, CatalogError> {
        let display = display_name.trim().nfc().collect::<String>();
        if display.is_empty() {
            return Err(CatalogError::InvalidStoredValue {
                column: "tag_display_name",
                value: "empty tag name".to_owned(),
            });
        }
        Ok(Self { normalized: display.to_lowercase(), display })
    }
}

fn tag_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TagSummary> {
    let tag_id: String = row.get(0)?;
    let tag_id = Uuid::parse_str(&tag_id).map(TagId::from).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            "invalid tag identifier".into(),
        )
    })?;
    Ok(TagSummary { tag_id, display_name: row.get(1)? })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::catalog::SearchIndexEntry;
    use crate::{Catalog, CatalogNote, DocumentKind, NoteId};

    fn insert_note(catalog: &Catalog, note_id: NoteId, path: &str) {
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: path.into(),
                kind: DocumentKind::Markdown,
                title: path.to_owned(),
                excerpt: "fixture".to_owned(),
                modified_at: UNIX_EPOCH + Duration::from_secs(1),
                file_size: 7,
                content_hash: vec![1, 2, 3],
                starred: false,
            })
            .expect("fixture note should persist");
    }

    #[test]
    fn metadata_edits_are_idempotent_and_badges_exclude_trashed_notes() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let active_note_id = NoteId::generate();
        let trashed_note_id = NoteId::generate();
        insert_note(&catalog, active_note_id, "active.md");
        insert_note(&catalog, trashed_note_id, "trashed.md");

        assert!(catalog.toggle_note_starred(active_note_id).expect("star should toggle"));
        let tag = catalog.create_tag("  Plan  ").expect("tag should create");
        assert_eq!(tag.display_name, "Plan");
        assert!(catalog.attach_tag(active_note_id, tag.tag_id).expect("first attach should work"));
        assert!(
            !catalog
                .attach_tag(active_note_id, tag.tag_id)
                .expect("duplicate attach is idempotent")
        );
        assert!(
            catalog
                .attach_tag(trashed_note_id, tag.tag_id)
                .expect("trashed metadata should persist")
        );
        catalog
            .connection()
            .execute(
                "UPDATE notes SET lifecycle = 1 WHERE note_id = ?1",
                [trashed_note_id.to_string()],
            )
            .expect("fixture should trash note");

        assert_eq!(
            catalog.tags_with_active_note_counts().expect("tag badges should query"),
            vec![super::TagWithActiveNoteCount {
                tag_id: tag.tag_id,
                display_name: "Plan".to_owned(),
                active_note_count: 1
            }]
        );
    }

    #[test]
    fn unicode_normalized_tag_names_are_unique_and_renames_are_atomic() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let first_tag = catalog.create_tag("Café").expect("first tag should create");
        assert!(catalog.create_tag("Cafe\u{301}").is_err());
        let second_tag = catalog.create_tag("Archive").expect("second tag should create");

        assert!(catalog.rename_tag(second_tag.tag_id, "Cafe\u{301}").is_err());
        assert_eq!(
            catalog.tag(first_tag.tag_id).expect("tag should read").expect("tag should remain"),
            first_tag
        );
        assert_eq!(
            catalog.tag(second_tag.tag_id).expect("tag should read").expect("tag should remain"),
            second_tag
        );
    }

    #[test]
    fn navigation_tree_only_includes_active_note_directories_and_preserves_tag_identity() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let active_note_id = NoteId::generate();
        let trashed_note_id = NoteId::generate();
        insert_note(&catalog, active_note_id, "work/plans/active.md");
        insert_note(&catalog, trashed_note_id, "archive/trashed.md");
        let tag = catalog.create_tag("Plan").expect("tag should create");
        assert!(catalog.attach_tag(active_note_id, tag.tag_id).expect("tag should attach"));
        catalog
            .connection()
            .execute(
                "UPDATE notes SET lifecycle = 1 WHERE note_id = ?1",
                [trashed_note_id.to_string()],
            )
            .expect("fixture should trash note");

        assert_eq!(
            catalog.navigation_tree().expect("navigation tree should query"),
            super::CatalogNavigationTree {
                directories: vec!["work".into(), "work/plans".into()],
                tags: vec![super::TagWithActiveNoteCount {
                    tag_id: tag.tag_id,
                    display_name: "Plan".to_owned(),
                    active_note_count: 1,
                }],
            }
        );
    }

    #[test]
    fn tag_mutations_keep_full_text_search_tags_consistent_without_rescanning_the_body() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        insert_note(&catalog, note_id, "note.md");
        catalog
            .index_note_batch(&[SearchIndexEntry {
                note_id,
                title: "Unrelated".to_owned(),
                relative_path: "note.md".into(),
                body: "body without the tag names".to_owned(),
                tags: Vec::new(),
            }])
            .expect("fixture search entry should persist");
        let tag = catalog.create_tag("Roadmap").expect("tag should create");

        catalog.attach_tag(note_id, tag.tag_id).expect("tag should attach");
        let attached_matches =
            catalog.search_active_notes("roadmap", 10).expect("attached tag should search");
        assert_eq!(attached_matches.len(), 1);
        assert_eq!(attached_matches[0].note_id, note_id);

        catalog.rename_tag(tag.tag_id, "Archive").expect("tag should rename");
        assert!(
            catalog.search_active_notes("roadmap", 10).expect("old tag should search").is_empty()
        );
        let renamed_matches =
            catalog.search_active_notes("archive", 10).expect("renamed tag should search");
        assert_eq!(renamed_matches.len(), 1);
        assert_eq!(renamed_matches[0].note_id, note_id);

        catalog.detach_tag(note_id, tag.tag_id).expect("tag should detach");
        assert!(
            catalog
                .search_active_notes("archive", 10)
                .expect("detached tag should search")
                .is_empty()
        );
    }
}
