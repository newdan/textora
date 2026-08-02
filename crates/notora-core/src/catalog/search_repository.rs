use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::{Row, Transaction, params};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use crate::NoteId;

use super::{Catalog, CatalogError};

const TAG_SEPARATOR: &str = "\n";
const DELETE_ALL_SEARCH_INDEX_ENTRIES: &str = "DELETE FROM note_search";
const ACTIVE_NOTE_LIFECYCLE: i64 = 0;
const MINIMUM_TRIGRAM_QUERY_GRAPHEMES: usize = 3;
const SHORT_QUERY_BODY_CANDIDATE_LIMIT: usize = 128;

const TITLE_MATCH_WEIGHT: i64 = 160;
const PATH_MATCH_WEIGHT: i64 = 120;
const TAG_EXACT_MATCH_WEIGHT: i64 = 140;
const TAG_PREFIX_MATCH_WEIGHT: i64 = 100;
const BODY_MATCH_WEIGHT: i64 = 40;

const FULL_TEXT_CANDIDATES_SQL: &str = "
SELECT n.note_id, n.title, n.relative_path, n.modified_ns, s.body, s.tags
FROM note_search AS s
JOIN notes AS n ON n.note_id = s.note_id
WHERE n.lifecycle = ?1 AND note_search MATCH ?2";

const SHORT_QUERY_STRUCTURED_CANDIDATES_SQL: &str = "
SELECT n.note_id, n.title, n.relative_path, n.modified_ns, s.body, s.tags
FROM note_search AS s
JOIN notes AS n ON n.note_id = s.note_id
WHERE n.lifecycle = ?1
  AND (
      instr(lower(n.title), ?2) > 0
      OR instr(lower(n.relative_path), ?2) > 0
      OR EXISTS (
          SELECT 1
          FROM note_tags AS nt
          JOIN tags AS t ON t.tag_id = nt.tag_id
          WHERE nt.note_id = n.note_id
            AND (t.normalized_name = ?2 OR t.normalized_name LIKE ?3 ESCAPE '\\')
      )
  )";

const SHORT_QUERY_BODY_CANDIDATES_SQL: &str = "
SELECT n.note_id, n.title, n.relative_path, n.modified_ns, s.body, s.tags
FROM note_search AS s
JOIN notes AS n ON n.note_id = s.note_id
WHERE n.lifecycle = ?1
ORDER BY n.modified_ns DESC, n.relative_path ASC, n.note_id ASC
LIMIT ?2";

/// 后台索引器从笔记正文和 catalog metadata 构造的全文索引输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchIndexEntry {
    pub note_id: NoteId,
    pub title: String,
    pub relative_path: PathBuf,
    pub body: String,
    pub tags: Vec<String>,
}

/// 由全文检索命中的活动笔记身份，已经按固定相关性规则排序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub note_id: NoteId,
}

impl Catalog {
    /// 在一个事务中替换多个笔记的全文索引字段。
    ///
    /// 调用方应使用独占后台 catalog 连接，避免在渲染路径执行索引写入。
    pub fn index_note_batch(&self, entries: &[SearchIndexEntry]) -> Result<(), CatalogError> {
        ensure_unique_note_ids(entries)?;
        if entries.is_empty() {
            return Ok(());
        }

        let transaction = self
            .connection()
            .unchecked_transaction()
            .map_err(|source| CatalogError::sql("search index transaction start", source))?;
        for entry in entries {
            replace_search_index_entry(&transaction, entry)?;
        }
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("search index transaction commit", source))
    }

    /// 从全文索引中移除指定笔记；缺失的索引行按幂等成功处理。
    pub fn remove_search_index_entries(&self, note_ids: &[NoteId]) -> Result<(), CatalogError> {
        if note_ids.is_empty() {
            return Ok(());
        }

        let transaction = self.connection().unchecked_transaction().map_err(|source| {
            CatalogError::sql("search index removal transaction start", source)
        })?;
        for note_id in note_ids {
            delete_search_index_entry(&transaction, *note_id)?;
        }
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("search index removal transaction commit", source))
    }

    /// 以给定的完整快照原子重建全文索引。
    pub fn rebuild_search_index(&self, entries: &[SearchIndexEntry]) -> Result<(), CatalogError> {
        ensure_unique_note_ids(entries)?;

        let transaction = self.connection().unchecked_transaction().map_err(|source| {
            CatalogError::sql("search index rebuild transaction start", source)
        })?;
        transaction
            .execute(DELETE_ALL_SEARCH_INDEX_ENTRIES, [])
            .map_err(|source| CatalogError::sql("search index rebuild clear", source))?;
        for entry in entries {
            insert_search_index_entry(&transaction, entry)?;
        }
        transaction
            .commit()
            .map_err(|source| CatalogError::sql("search index rebuild transaction commit", source))
    }

    /// 搜索活动笔记；空查询不会访问 FTS，回收站笔记始终被排除。
    pub fn search_active_notes(
        &self,
        query: &str,
        maximum_results: usize,
    ) -> Result<Vec<SearchMatch>, CatalogError> {
        let normalized_query = normalize_search_text(query);
        if normalized_query.is_empty() || maximum_results == 0 {
            return Ok(Vec::new());
        }

        let candidates =
            if normalized_query.graphemes(true).count() >= MINIMUM_TRIGRAM_QUERY_GRAPHEMES {
                self.full_text_search_candidates(&normalized_query)?
            } else {
                self.short_query_search_candidates(&normalized_query)?
            };

        let mut ranked_matches = candidates
            .into_iter()
            .filter_map(|candidate| rank_search_candidate(candidate, &normalized_query))
            .collect::<Vec<_>>();
        ranked_matches.sort_by(compare_ranked_search_candidates);

        Ok(ranked_matches
            .into_iter()
            .take(maximum_results)
            .map(|ranked_match| SearchMatch { note_id: ranked_match.candidate.note_id })
            .collect())
    }

    fn full_text_search_candidates(
        &self,
        normalized_query: &str,
    ) -> Result<Vec<SearchCandidate>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(FULL_TEXT_CANDIDATES_SQL)
            .map_err(|source| CatalogError::sql("full-text search query preparation", source))?;
        let fts_query = fts_phrase_query(normalized_query);
        statement
            .query_map(params![ACTIVE_NOTE_LIFECYCLE, fts_query], search_candidate_from_row)
            .map_err(|source| CatalogError::sql("full-text search query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("full-text search row read", source))?
            .into_iter()
            .map(SearchCandidate::try_from)
            .collect()
    }

    fn short_query_search_candidates(
        &self,
        normalized_query: &str,
    ) -> Result<Vec<SearchCandidate>, CatalogError> {
        let mut candidates = self.structured_short_query_candidates(normalized_query)?;
        let mut known_note_ids =
            candidates.iter().map(|candidate| candidate.note_id).collect::<HashSet<_>>();
        for candidate in self.short_query_body_candidates()? {
            if !normalized_contains(&candidate.body, normalized_query)
                || !known_note_ids.insert(candidate.note_id)
            {
                continue;
            }
            candidates.push(candidate);
        }
        Ok(candidates)
    }

    fn structured_short_query_candidates(
        &self,
        normalized_query: &str,
    ) -> Result<Vec<SearchCandidate>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(SHORT_QUERY_STRUCTURED_CANDIDATES_SQL)
            .map_err(|source| CatalogError::sql("short search query preparation", source))?;
        let tag_prefix_pattern = format!("{}%", escape_like_pattern(normalized_query));
        statement
            .query_map(
                params![ACTIVE_NOTE_LIFECYCLE, normalized_query, tag_prefix_pattern],
                search_candidate_from_row,
            )
            .map_err(|source| CatalogError::sql("short search query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("short search row read", source))?
            .into_iter()
            .map(SearchCandidate::try_from)
            .collect()
    }

    fn short_query_body_candidates(&self) -> Result<Vec<SearchCandidate>, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(SHORT_QUERY_BODY_CANDIDATES_SQL)
            .map_err(|source| CatalogError::sql("short body fallback preparation", source))?;
        statement
            .query_map(
                params![ACTIVE_NOTE_LIFECYCLE, SHORT_QUERY_BODY_CANDIDATE_LIMIT],
                search_candidate_from_row,
            )
            .map_err(|source| CatalogError::sql("short body fallback query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("short body fallback row read", source))?
            .into_iter()
            .map(SearchCandidate::try_from)
            .collect()
    }
}

#[derive(Debug)]
struct StoredSearchCandidate {
    note_id: String,
    title: String,
    relative_path: String,
    modified_nanoseconds: i64,
    body: String,
    tags: String,
}

#[derive(Debug)]
struct SearchCandidate {
    note_id: NoteId,
    title: String,
    relative_path: String,
    modified_nanoseconds: i64,
    body: String,
    tags: String,
}

#[derive(Debug)]
struct RankedSearchCandidate {
    candidate: SearchCandidate,
    score: i64,
}

impl TryFrom<StoredSearchCandidate> for SearchCandidate {
    type Error = CatalogError;

    fn try_from(stored_candidate: StoredSearchCandidate) -> Result<Self, Self::Error> {
        let note_id =
            Uuid::parse_str(&stored_candidate.note_id).map(NoteId::from).map_err(|_| {
                CatalogError::InvalidStoredValue {
                    column: "search_index.note_id",
                    value: stored_candidate.note_id,
                }
            })?;
        Ok(Self {
            note_id,
            title: stored_candidate.title,
            relative_path: stored_candidate.relative_path,
            modified_nanoseconds: stored_candidate.modified_nanoseconds,
            body: stored_candidate.body,
            tags: stored_candidate.tags,
        })
    }
}

fn search_candidate_from_row(row: &Row<'_>) -> rusqlite::Result<StoredSearchCandidate> {
    Ok(StoredSearchCandidate {
        note_id: row.get(0)?,
        title: row.get(1)?,
        relative_path: row.get(2)?,
        modified_nanoseconds: row.get(3)?,
        body: row.get(4)?,
        tags: row.get(5)?,
    })
}

fn rank_search_candidate(
    candidate: SearchCandidate,
    normalized_query: &str,
) -> Option<RankedSearchCandidate> {
    let title_matches = normalized_contains(&candidate.title, normalized_query);
    let path_matches = normalized_contains(&candidate.relative_path, normalized_query);
    let body_matches = normalized_contains(&candidate.body, normalized_query);
    let tag_match_weight = search_tag_match_weight(&candidate.tags, normalized_query);
    if !title_matches && !path_matches && !body_matches && tag_match_weight == 0 {
        return None;
    }

    let score = if title_matches { TITLE_MATCH_WEIGHT } else { 0 }
        + if path_matches { PATH_MATCH_WEIGHT } else { 0 }
        + tag_match_weight
        + if body_matches { BODY_MATCH_WEIGHT } else { 0 };
    Some(RankedSearchCandidate { candidate, score })
}

fn search_tag_match_weight(tags: &str, normalized_query: &str) -> i64 {
    let mut match_weight = 0;
    for tag in tags.split(TAG_SEPARATOR) {
        let normalized_tag = normalize_search_text(tag);
        if normalized_tag == normalized_query {
            match_weight = match_weight.max(TAG_EXACT_MATCH_WEIGHT);
            continue;
        }
        if normalized_tag.starts_with(normalized_query) {
            match_weight = match_weight.max(TAG_PREFIX_MATCH_WEIGHT);
        }
    }
    match_weight
}

fn compare_ranked_search_candidates(
    left: &RankedSearchCandidate,
    right: &RankedSearchCandidate,
) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| {
            right.candidate.modified_nanoseconds.cmp(&left.candidate.modified_nanoseconds)
        })
        .then_with(|| left.candidate.relative_path.cmp(&right.candidate.relative_path))
        .then_with(|| left.candidate.note_id.as_uuid().cmp(&right.candidate.note_id.as_uuid()))
}

fn normalize_search_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn normalized_contains(value: &str, normalized_query: &str) -> bool {
    normalize_search_text(value).contains(normalized_query)
}

fn fts_phrase_query(normalized_query: &str) -> String {
    format!("\"{}\"", normalized_query.replace('"', "\"\""))
}

fn escape_like_pattern(value: &str) -> String {
    value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn ensure_unique_note_ids(entries: &[SearchIndexEntry]) -> Result<(), CatalogError> {
    let mut note_ids = HashSet::with_capacity(entries.len());
    for entry in entries {
        if note_ids.insert(entry.note_id) {
            continue;
        }

        return Err(CatalogError::InvalidStoredValue {
            column: "search_index.note_id",
            value: entry.note_id.to_string(),
        });
    }
    Ok(())
}

fn replace_search_index_entry(
    transaction: &Transaction<'_>,
    entry: &SearchIndexEntry,
) -> Result<(), CatalogError> {
    delete_search_index_entry(transaction, entry.note_id)?;
    insert_search_index_entry(transaction, entry)
}

fn delete_search_index_entry(
    transaction: &Transaction<'_>,
    note_id: NoteId,
) -> Result<(), CatalogError> {
    transaction
        .execute("DELETE FROM note_search WHERE note_id = ?1", [note_id.to_string()])
        .map_err(|source| CatalogError::sql("search index entry removal", source))?;
    Ok(())
}

fn insert_search_index_entry(
    transaction: &Transaction<'_>,
    entry: &SearchIndexEntry,
) -> Result<(), CatalogError> {
    transaction
        .execute(
            "INSERT INTO note_search (note_id, title, relative_path, body, tags)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                entry.note_id.to_string(),
                entry.title,
                entry.relative_path.to_string_lossy(),
                entry.body,
                entry.tags.join(TAG_SEPARATOR),
            ],
        )
        .map_err(|source| CatalogError::sql("search index entry insert", source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::SearchIndexEntry;
    use crate::{Catalog, CatalogNote, DocumentKind, NoteId, TagId};

    struct SearchCatalogFixture {
        _directory: tempfile::TempDir,
        catalog: Catalog,
    }

    fn search_index_entry(note_id: NoteId, title: &str) -> SearchIndexEntry {
        SearchIndexEntry {
            note_id,
            title: title.to_owned(),
            relative_path: "notes/research.md".into(),
            body: "完整正文".to_owned(),
            tags: vec!["research".to_owned(), "写作".to_owned()],
        }
    }

    fn temporary_catalog() -> SearchCatalogFixture {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        SearchCatalogFixture { _directory: directory, catalog }
    }

    fn index_searchable_note(
        catalog: &Catalog,
        note_id: NoteId,
        title: &str,
        relative_path: &str,
        body: &str,
        tags: &[&str],
        modified_seconds: u64,
    ) {
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: relative_path.into(),
                kind: DocumentKind::Markdown,
                title: title.to_owned(),
                excerpt: body.to_owned(),
                modified_at: UNIX_EPOCH + Duration::from_secs(modified_seconds),
                file_size: u64::try_from(body.len()).expect("test body length should fit u64"),
                content_hash: modified_seconds.to_le_bytes().to_vec(),
                starred: false,
            })
            .expect("searchable note should persist");
        for tag_name in tags {
            let tag_id = TagId::generate();
            catalog
                .connection()
                .execute(
                    "INSERT INTO tags (tag_id, normalized_name, display_name) VALUES (?1, ?2, ?3)",
                    [tag_id.to_string(), tag_name.to_lowercase(), (*tag_name).to_owned()],
                )
                .expect("test tag should persist");
            catalog
                .connection()
                .execute(
                    "INSERT INTO note_tags (note_id, tag_id) VALUES (?1, ?2)",
                    [note_id.to_string(), tag_id.to_string()],
                )
                .expect("test note tag should persist");
        }
        catalog
            .index_note_batch(&[SearchIndexEntry {
                note_id,
                title: title.to_owned(),
                relative_path: relative_path.into(),
                body: body.to_owned(),
                tags: tags.iter().map(|tag_name| (*tag_name).to_owned()).collect(),
            }])
            .expect("searchable note should index");
    }

    #[test]
    fn batch_index_replaces_all_searchable_fields() {
        let fixture = temporary_catalog();
        let catalog = &fixture.catalog;
        let note_id = NoteId::generate();
        catalog
            .index_note_batch(&[search_index_entry(note_id, "旧标题")])
            .expect("initial index write should succeed");
        let mut replacement = search_index_entry(note_id, "新标题");
        replacement.relative_path = "archive/research.md".into();
        replacement.body = "替换后的完整正文".to_owned();
        replacement.tags = vec!["archive".to_owned()];

        catalog.index_note_batch(&[replacement]).expect("replacement index write should succeed");

        let indexed_fields: (String, String, String, String) = catalog
            .connection()
            .query_row(
                "SELECT title, relative_path, body, tags FROM note_search WHERE note_id = ?1",
                [note_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("indexed entry should be readable");
        assert_eq!(
            indexed_fields,
            (
                "新标题".to_owned(),
                "archive/research.md".to_owned(),
                "替换后的完整正文".to_owned(),
                "archive".to_owned(),
            )
        );
    }

    #[test]
    fn removal_is_idempotent_and_rebuild_replaces_the_complete_snapshot() {
        let fixture = temporary_catalog();
        let catalog = &fixture.catalog;
        let removed_note_id = NoteId::generate();
        let retained_note_id = NoteId::generate();
        catalog
            .index_note_batch(&[
                search_index_entry(removed_note_id, "待移除"),
                search_index_entry(retained_note_id, "初始保留"),
            ])
            .expect("initial index batch should succeed");

        catalog
            .remove_search_index_entries(&[removed_note_id, removed_note_id])
            .expect("repeated removal should succeed");
        catalog
            .rebuild_search_index(&[search_index_entry(retained_note_id, "重建保留")])
            .expect("index rebuild should succeed");

        let indexed_note_ids: Vec<String> = catalog
            .connection()
            .prepare("SELECT note_id FROM note_search ORDER BY note_id")
            .expect("index query should prepare")
            .query_map([], |row| row.get(0))
            .expect("index query should execute")
            .collect::<Result<_, _>>()
            .expect("index rows should read");
        assert_eq!(indexed_note_ids, vec![retained_note_id.to_string()]);
    }

    #[test]
    fn batch_rejects_duplicate_note_identity_before_writing() {
        let fixture = temporary_catalog();
        let catalog = &fixture.catalog;
        let note_id = NoteId::generate();

        assert!(
            catalog
                .index_note_batch(&[
                    search_index_entry(note_id, "第一条"),
                    search_index_entry(note_id, "第二条"),
                ])
                .is_err()
        );
        let indexed_count: i64 = catalog
            .connection()
            .query_row("SELECT COUNT(*) FROM note_search", [], |row| row.get(0))
            .expect("index count should be readable");
        assert_eq!(indexed_count, 0);
    }

    #[test]
    fn search_matches_chinese_latin_and_tag_prefixes_with_fixed_field_priority() {
        let fixture = temporary_catalog();
        let catalog = &fixture.catalog;
        let title_note_id = NoteId::generate();
        let body_note_id = NoteId::generate();
        let tag_note_id = NoteId::generate();
        index_searchable_note(
            catalog,
            title_note_id,
            "中文搜索设计",
            "notes/search.md",
            "正文不包含目标词",
            &["设计"],
            1,
        );
        index_searchable_note(
            catalog,
            body_note_id,
            "Implementation",
            "notes/implementation.md",
            "The search language supports Latin text.",
            &["engineering"],
            2,
        );
        index_searchable_note(
            catalog,
            tag_note_id,
            "标签匹配",
            "notes/tag.md",
            "正文不包含目标词",
            &["drafting"],
            3,
        );

        assert_eq!(
            catalog.search_active_notes("搜索", 10).expect("Chinese title search should succeed"),
            vec![super::SearchMatch { note_id: title_note_id }]
        );
        assert_eq!(
            catalog.search_active_notes("language", 10).expect("Latin body search should succeed"),
            vec![super::SearchMatch { note_id: body_note_id }]
        );
        assert_eq!(
            catalog.search_active_notes("dra", 10).expect("tag prefix search should succeed"),
            vec![super::SearchMatch { note_id: tag_note_id }]
        );
    }

    #[test]
    fn search_handles_combining_characters_and_literal_sql_wildcards() {
        let fixture = temporary_catalog();
        let catalog = &fixture.catalog;
        let combining_note_id = NoteId::generate();
        let percent_note_id = NoteId::generate();
        index_searchable_note(
            catalog,
            combining_note_id,
            "Cafe\u{301} notes",
            "notes/cafe.md",
            "组合字符测试",
            &[],
            1,
        );
        index_searchable_note(
            catalog,
            percent_note_id,
            "100%_safe",
            "notes/literal.md",
            "百分号和下划线是标题的一部分",
            &[],
            2,
        );

        assert_eq!(
            catalog
                .search_active_notes("Cafe\u{301}", 10)
                .expect("combining character search should succeed"),
            vec![super::SearchMatch { note_id: combining_note_id }]
        );
        assert_eq!(
            catalog.search_active_notes("%", 10).expect("literal percent search should succeed"),
            vec![super::SearchMatch { note_id: percent_note_id }]
        );
        assert!(
            catalog
                .search_active_notes("", 10)
                .expect("empty search should not execute FTS")
                .is_empty()
        );
    }

    #[test]
    fn search_excludes_trash_and_uses_stable_path_tie_breaking() {
        let fixture = temporary_catalog();
        let catalog = &fixture.catalog;
        let first_note_id = NoteId::generate();
        let second_note_id = NoteId::generate();
        let trashed_note_id = NoteId::generate();
        index_searchable_note(catalog, first_note_id, "计划", "a/plan.md", "正文", &[], 10);
        index_searchable_note(catalog, second_note_id, "计划", "b/plan.md", "正文", &[], 10);
        index_searchable_note(catalog, trashed_note_id, "计划", "trash/plan.md", "正文", &[], 20);
        catalog
            .connection()
            .execute(
                "UPDATE notes SET lifecycle = 1 WHERE note_id = ?1",
                [trashed_note_id.to_string()],
            )
            .expect("trash fixture should update lifecycle");

        assert_eq!(
            catalog.search_active_notes("计划", 10).expect("active note search should succeed"),
            vec![
                super::SearchMatch { note_id: first_note_id },
                super::SearchMatch { note_id: second_note_id },
            ]
        );
    }
}
