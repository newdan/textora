use std::path::PathBuf;

use rusqlite::{Row, ToSql, params_from_iter};
use uuid::Uuid;

use crate::{DocumentKind, NavigationScope, NoteId};

use super::{Catalog, CatalogError};

const ACTIVE_NOTE_LIFECYCLE: i64 = 0;
const TRASHED_NOTE_LIFECYCLE: i64 = 1;
const CARD_QUERY_EXTRA_ROW_COUNT: usize = 1;

/// 一张中栏卡片所需的预计算 catalog 字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogCard {
    pub note_id: NoteId,
    pub relative_path: PathBuf,
    pub kind: DocumentKind,
    pub title: String,
    pub excerpt: String,
    pub modified_nanoseconds: i64,
    pub starred: bool,
    pub tags: Vec<String>,
}

/// 由 catalog 固定排序键组成的稳定续页位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogCardCursor {
    pub modified_nanoseconds: i64,
    pub relative_path: PathBuf,
    pub note_id: NoteId,
}

/// 单次卡片查询的分页结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogCardPage {
    pub cards: Vec<CatalogCard>,
    pub next_cursor: Option<CatalogCardCursor>,
}

impl Catalog {
    /// 查询工作区 catalog 卡片。ExternalFiles 由产品 session 提供，绝不进入此数据源。
    pub fn query_catalog_cards(
        &self,
        scope: &NavigationScope,
        cursor: Option<&CatalogCardCursor>,
        page_size: usize,
    ) -> Result<CatalogCardPage, CatalogError> {
        if page_size == 0 || *scope == NavigationScope::ExternalFiles {
            return Ok(CatalogCardPage { cards: Vec::new(), next_cursor: None });
        }

        if let NavigationScope::Search { query } = scope {
            return self.query_search_cards(query, cursor, page_size);
        }

        let query = CatalogCardQuery::from_scope(scope)?;
        let mut parameters = query.parameters;
        let mut sql = format!(
            "SELECT n.note_id, n.relative_path, n.kind, n.title, n.excerpt, n.modified_ns, n.starred\n{}\nWHERE {}",
            query.source, query.predicate
        );
        append_cursor_predicate(&mut sql, &mut parameters, cursor);
        sql.push_str(" ORDER BY n.modified_ns DESC, n.relative_path ASC, n.note_id ASC LIMIT ?");
        let requested_row_count =
            page_size.checked_add(CARD_QUERY_EXTRA_ROW_COUNT).ok_or_else(|| {
                CatalogError::InvalidStoredValue {
                    column: "card_page_size",
                    value: page_size.to_string(),
                }
            })?;
        parameters.push(Box::new(i64::try_from(requested_row_count).map_err(|_| {
            CatalogError::InvalidStoredValue {
                column: "card_page_size",
                value: page_size.to_string(),
            }
        })?));

        let mut statement = self
            .connection()
            .prepare(&sql)
            .map_err(|source| CatalogError::sql("card page query preparation", source))?;
        let mut cards = statement
            .query_map(
                params_from_iter(
                    parameters.iter().map(|parameter| parameter.as_ref() as &dyn ToSql),
                ),
                catalog_card_from_row,
            )
            .map_err(|source| CatalogError::sql("card page query", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| CatalogError::sql("card page row read", source))?
            .into_iter()
            .map(CatalogCard::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let has_next_page = cards.len() > page_size;
        if has_next_page {
            let _ = cards.pop();
        }
        self.populate_card_tags(&mut cards)?;
        let next_cursor = has_next_page.then(|| {
            cards
                .last()
                .map(CatalogCardCursor::from)
                .expect("an extra row guarantees the current card page is non-empty")
        });
        Ok(CatalogCardPage { cards, next_cursor })
    }

    fn query_search_cards(
        &self,
        search_query: &str,
        cursor: Option<&CatalogCardCursor>,
        page_size: usize,
    ) -> Result<CatalogCardPage, CatalogError> {
        let matches = self.search_active_notes(search_query, usize::MAX)?;
        let start_index = match cursor {
            Some(cursor) => matches
                .iter()
                .position(|search_match| search_match.note_id == cursor.note_id)
                .map(|index| index + 1)
                .unwrap_or(matches.len()),
            None => 0,
        };
        let requested_row_count =
            page_size.checked_add(CARD_QUERY_EXTRA_ROW_COUNT).ok_or_else(|| {
                CatalogError::InvalidStoredValue {
                    column: "card_page_size",
                    value: page_size.to_string(),
                }
            })?;
        let page_note_ids = matches
            .iter()
            .skip(start_index)
            .take(requested_row_count)
            .map(|search_match| search_match.note_id)
            .collect::<Vec<_>>();
        let has_next_page = page_note_ids.len() > page_size;
        let card_note_ids = page_note_ids.into_iter().take(page_size).collect::<Vec<_>>();
        let mut cards = card_note_ids
            .into_iter()
            .map(|note_id| self.card_for_active_note(note_id))
            .collect::<Result<Vec<_>, _>>()?;
        self.populate_card_tags(&mut cards)?;
        let next_cursor = has_next_page.then(|| {
            cards
                .last()
                .map(CatalogCardCursor::from)
                .expect("an extra search match guarantees the current card page is non-empty")
        });
        Ok(CatalogCardPage { cards, next_cursor })
    }

    fn card_for_active_note(&self, note_id: NoteId) -> Result<CatalogCard, CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT note_id, relative_path, kind, title, excerpt, modified_ns, starred
                 FROM notes WHERE note_id = ?1 AND lifecycle = ?2",
            )
            .map_err(|source| CatalogError::sql("search card lookup preparation", source))?;
        let stored_card = statement
            .query_row(
                [note_id.to_string(), ACTIVE_NOTE_LIFECYCLE.to_string()],
                catalog_card_from_row,
            )
            .map_err(|source| CatalogError::sql("search card lookup", source))?;
        CatalogCard::try_from(stored_card)
    }

    fn populate_card_tags(&self, cards: &mut [CatalogCard]) -> Result<(), CatalogError> {
        let mut statement = self
            .connection()
            .prepare(
                "SELECT t.display_name FROM tags AS t
                 JOIN note_tags AS nt ON nt.tag_id = t.tag_id
                 WHERE nt.note_id = ?1 ORDER BY t.normalized_name ASC",
            )
            .map_err(|source| CatalogError::sql("card tag query preparation", source))?;
        for card in cards {
            card.tags = statement
                .query_map([card.note_id.to_string()], |row| row.get(0))
                .map_err(|source| CatalogError::sql("card tag query", source))?
                .collect::<Result<Vec<String>, _>>()
                .map_err(|source| CatalogError::sql("card tag row read", source))?;
        }
        Ok(())
    }
}

struct CatalogCardQuery {
    source: &'static str,
    predicate: &'static str,
    parameters: Vec<Box<dyn ToSql>>,
}

impl CatalogCardQuery {
    fn from_scope(scope: &NavigationScope) -> Result<Self, CatalogError> {
        let active_parameters = || vec![Box::new(ACTIVE_NOTE_LIFECYCLE) as Box<dyn ToSql>];
        match scope {
            NavigationScope::WorkspaceRoot => Ok(Self {
                source: "FROM notes AS n",
                predicate: "n.lifecycle = ?",
                parameters: active_parameters(),
            }),
            NavigationScope::Directory { relative_path } => {
                let prefix = directory_prefix_pattern(relative_path)?;
                Ok(Self {
                    source: "FROM notes AS n",
                    predicate: "n.lifecycle = ? AND n.relative_path LIKE ? ESCAPE '\\'",
                    parameters: vec![Box::new(ACTIVE_NOTE_LIFECYCLE), Box::new(prefix)],
                })
            }
            NavigationScope::Starred => Ok(Self {
                source: "FROM notes AS n",
                predicate: "n.lifecycle = ? AND n.starred = 1",
                parameters: active_parameters(),
            }),
            NavigationScope::Trash => Ok(Self {
                source: "FROM notes AS n",
                predicate: "n.lifecycle = ?",
                parameters: vec![Box::new(TRASHED_NOTE_LIFECYCLE)],
            }),
            NavigationScope::Tag { tag_id } => Ok(Self {
                source: "FROM notes AS n",
                predicate: "n.lifecycle = ? AND EXISTS (SELECT 1 FROM note_tags AS nt WHERE nt.note_id = n.note_id AND nt.tag_id = ?)",
                parameters: vec![Box::new(ACTIVE_NOTE_LIFECYCLE), Box::new(tag_id.to_string())],
            }),
            NavigationScope::Search { .. } => unreachable!("search cards use ranked search"),
            NavigationScope::ExternalFiles => {
                unreachable!("external cards return before catalog query")
            }
        }
    }
}

fn append_cursor_predicate(
    sql: &mut String,
    parameters: &mut Vec<Box<dyn ToSql>>,
    cursor: Option<&CatalogCardCursor>,
) {
    let Some(cursor) = cursor else {
        return;
    };
    sql.push_str(
        " AND (n.modified_ns < ? OR (n.modified_ns = ? AND (n.relative_path > ? OR (n.relative_path = ? AND n.note_id > ?))))",
    );
    parameters.push(Box::new(cursor.modified_nanoseconds));
    parameters.push(Box::new(cursor.modified_nanoseconds));
    parameters.push(Box::new(cursor.relative_path.to_string_lossy().to_string()));
    parameters.push(Box::new(cursor.relative_path.to_string_lossy().to_string()));
    parameters.push(Box::new(cursor.note_id.to_string()));
}

fn directory_prefix_pattern(relative_path: &std::path::Path) -> Result<String, CatalogError> {
    let directory = relative_path.to_string_lossy();
    if directory.is_empty() {
        return Err(CatalogError::InvalidStoredValue {
            column: "directory_relative_path",
            value: directory.into_owned(),
        });
    }
    Ok(format!("{}/%", escape_like_pattern(&directory)))
}

fn escape_like_pattern(value: &str) -> String {
    value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

struct StoredCatalogCard {
    note_id: String,
    relative_path: String,
    kind: i64,
    title: String,
    excerpt: String,
    modified_nanoseconds: i64,
    starred: i64,
}

impl TryFrom<StoredCatalogCard> for CatalogCard {
    type Error = CatalogError;

    fn try_from(stored_card: StoredCatalogCard) -> Result<Self, Self::Error> {
        let note_id = Uuid::parse_str(&stored_card.note_id).map(NoteId::from).map_err(|_| {
            CatalogError::InvalidStoredValue { column: "note_id", value: stored_card.note_id }
        })?;
        let kind = match stored_card.kind {
            1 => DocumentKind::Text,
            2 => DocumentKind::Markdown,
            3 => DocumentKind::Mindmap,
            value => {
                return Err(CatalogError::InvalidStoredValue {
                    column: "kind",
                    value: value.to_string(),
                });
            }
        };
        let starred = match stored_card.starred {
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
            relative_path: stored_card.relative_path.into(),
            kind,
            title: stored_card.title,
            excerpt: stored_card.excerpt,
            modified_nanoseconds: stored_card.modified_nanoseconds,
            starred,
            tags: Vec::new(),
        })
    }
}

impl From<&CatalogCard> for CatalogCardCursor {
    fn from(card: &CatalogCard) -> Self {
        Self {
            modified_nanoseconds: card.modified_nanoseconds,
            relative_path: card.relative_path.clone(),
            note_id: card.note_id,
        }
    }
}

fn catalog_card_from_row(row: &Row<'_>) -> rusqlite::Result<StoredCatalogCard> {
    Ok(StoredCatalogCard {
        note_id: row.get(0)?,
        relative_path: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        excerpt: row.get(4)?,
        modified_nanoseconds: row.get(5)?,
        starred: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::catalog::SearchIndexEntry;
    use crate::{Catalog, CatalogNote, DocumentKind, NavigationScope, NoteId};

    fn insert_note(catalog: &Catalog, note_id: NoteId, path: &str, modified_seconds: u64) {
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: path.into(),
                kind: DocumentKind::Markdown,
                title: path.to_owned(),
                excerpt: "excerpt".to_owned(),
                modified_at: UNIX_EPOCH + Duration::from_secs(modified_seconds),
                file_size: 7,
                content_hash: modified_seconds.to_le_bytes().to_vec(),
                starred: false,
            })
            .expect("card fixture note should persist");
    }

    #[test]
    fn cursor_paginates_a_stable_catalog_order_without_duplicates() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let first_note_id = NoteId::generate();
        let second_note_id = NoteId::generate();
        let third_note_id = NoteId::generate();
        insert_note(&catalog, first_note_id, "a.md", 3);
        insert_note(&catalog, second_note_id, "b.md", 3);
        insert_note(&catalog, third_note_id, "c.md", 2);

        let first_page = catalog
            .query_catalog_cards(&NavigationScope::WorkspaceRoot, None, 2)
            .expect("first card page should load");
        assert_eq!(
            first_page.cards.iter().map(|card| card.note_id).collect::<Vec<_>>(),
            vec![first_note_id, second_note_id]
        );
        let cursor = first_page.next_cursor.expect("full first page should yield a cursor");
        let second_page = catalog
            .query_catalog_cards(&NavigationScope::WorkspaceRoot, Some(&cursor), 2)
            .expect("second card page should load");
        assert_eq!(
            second_page.cards.iter().map(|card| card.note_id).collect::<Vec<_>>(),
            vec![third_note_id]
        );
        assert_eq!(second_page.next_cursor, None);
    }

    #[test]
    fn search_cards_keep_ranked_multilingual_results_and_page_without_an_empty_tail() {
        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let first_note_id = NoteId::generate();
        let second_note_id = NoteId::generate();
        insert_note(&catalog, first_note_id, "ideas/first.md", 3);
        insert_note(&catalog, second_note_id, "ideas/second.md", 2);
        catalog
            .index_note_batch(&[
                SearchIndexEntry {
                    note_id: first_note_id,
                    title: "中文计划".to_owned(),
                    relative_path: "ideas/first.md".into(),
                    body: "第一条中文笔记".to_owned(),
                    tags: vec!["计划".to_owned()],
                },
                SearchIndexEntry {
                    note_id: second_note_id,
                    title: "其他内容".to_owned(),
                    relative_path: "ideas/second.md".into(),
                    body: "第二条中文笔记".to_owned(),
                    tags: Vec::new(),
                },
            ])
            .expect("search fixture should index");

        let first_page = catalog
            .query_catalog_cards(&NavigationScope::Search { query: "中文".to_owned() }, None, 1)
            .expect("short Chinese search should load its first page");
        assert_eq!(first_page.cards[0].note_id, first_note_id);
        let cursor = first_page.next_cursor.expect("a remaining match should produce a cursor");
        let second_page = catalog
            .query_catalog_cards(
                &NavigationScope::Search { query: "中文".to_owned() },
                Some(&cursor),
                1,
            )
            .expect("short Chinese search should load its second page");
        assert_eq!(second_page.cards[0].note_id, second_note_id);
        assert_eq!(second_page.next_cursor, None);
    }

    #[test]
    fn search_pagination_reaches_matches_beyond_the_previous_candidate_cap() {
        const MATCH_COUNT: usize = 600;
        const PAGE_SIZE: usize = 100;

        let directory = tempfile::tempdir().expect("catalog test directory should be created");
        let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let mut search_entries = Vec::with_capacity(MATCH_COUNT);
        for index in 0..MATCH_COUNT {
            let note_id = NoteId::generate();
            let relative_path = format!("bulk/note-{index:04}.md");
            insert_note(&catalog, note_id, &relative_path, index as u64);
            search_entries.push(SearchIndexEntry {
                note_id,
                title: format!("Needle {index}"),
                relative_path: relative_path.into(),
                body: "pagination needle marker".to_owned(),
                tags: Vec::new(),
            });
        }
        catalog.index_note_batch(&search_entries).expect("bulk search fixture should index");

        let scope = NavigationScope::Search { query: "needle".to_owned() };
        let mut cursor = None;
        let mut returned_note_ids = std::collections::HashSet::new();
        loop {
            let page = catalog
                .query_catalog_cards(&scope, cursor.as_ref(), PAGE_SIZE)
                .expect("search page should load");
            returned_note_ids.extend(page.cards.into_iter().map(|card| card.note_id));
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }

        assert_eq!(returned_note_ids.len(), MATCH_COUNT);
    }
}
