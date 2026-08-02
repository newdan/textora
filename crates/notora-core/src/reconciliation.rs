use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::{Catalog, CatalogError, CatalogNote, DocumentKind, NoteId};

/// 由扫描器或 watcher 规范化后的文件系统笔记记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredNote {
    pub relative_path: PathBuf,
    pub kind: DocumentKind,
    pub title: String,
    pub excerpt: String,
    pub modified_at: SystemTime,
    pub file_size: u64,
    pub content_hash: Vec<u8>,
}

impl DiscoveredNote {
    fn into_catalog_note(self, note_id: NoteId, starred: bool) -> CatalogNote {
        CatalogNote {
            note_id,
            relative_path: self.relative_path,
            kind: self.kind,
            title: self.title,
            excerpt: self.excerpt,
            modified_at: self.modified_at,
            file_size: self.file_size,
            content_hash: self.content_hash,
            starred,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationChange {
    Updated(CatalogNote),
    Moved { from: PathBuf, note: CatalogNote },
    Added(CatalogNote),
    Missing(CatalogNote),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconciliationPlan {
    pub changes: Vec<ReconciliationChange>,
}

#[derive(Debug)]
pub enum ReconciliationError {
    Catalog(CatalogError),
    DuplicateDiscoveredPath { path: PathBuf },
}

impl std::fmt::Display for ReconciliationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(source) => write!(formatter, "catalog reconciliation failed: {source}"),
            Self::DuplicateDiscoveredPath { path } => {
                write!(formatter, "scanner produced duplicate relative path: {}", path.display())
            }
        }
    }
}

impl std::error::Error for ReconciliationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(source) => Some(source),
            Self::DuplicateDiscoveredPath { .. } => None,
        }
    }
}

/// 将当前文件系统记录与 catalog 活动笔记进行保守比对。
///
/// 本函数不直接删除缺失行。调用方必须先经过 watcher 的合并窗口，再决定是否将
/// `Missing` 持久化或清理。
pub fn reconcile_catalog(
    catalog: &Catalog,
    discovered_notes: impl IntoIterator<Item = DiscoveredNote>,
) -> Result<ReconciliationPlan, ReconciliationError> {
    let existing_notes = catalog.active_notes().map_err(ReconciliationError::Catalog)?;
    reconcile_notes(existing_notes, discovered_notes)
}

pub(crate) fn reconcile_notes(
    existing_notes: Vec<CatalogNote>,
    discovered_notes: impl IntoIterator<Item = DiscoveredNote>,
) -> Result<ReconciliationPlan, ReconciliationError> {
    let discovered_notes = collect_discovered_notes(discovered_notes)?;
    let mut existing_by_path = existing_notes
        .into_iter()
        .map(|note| (note.relative_path.clone(), note))
        .collect::<HashMap<_, _>>();
    let mut unmatched_discovered = Vec::new();
    let mut changes = Vec::new();

    for discovered_note in discovered_notes {
        let Some(existing_note) = existing_by_path.remove(&discovered_note.relative_path) else {
            unmatched_discovered.push(discovered_note);
            continue;
        };
        changes.push(ReconciliationChange::Updated(
            discovered_note.into_catalog_note(existing_note.note_id, existing_note.starred),
        ));
    }

    let mut existing_by_hash = HashMap::<Vec<u8>, Vec<CatalogNote>>::new();
    for existing_note in existing_by_path.into_values() {
        existing_by_hash.entry(existing_note.content_hash.clone()).or_default().push(existing_note);
    }

    for discovered_note in unmatched_discovered {
        let has_unique_hash_match = existing_by_hash
            .get(&discovered_note.content_hash)
            .is_some_and(|candidates| candidates.len() == 1);
        let unique_match = has_unique_hash_match
            .then(|| existing_by_hash.remove(&discovered_note.content_hash))
            .flatten()
            .and_then(|mut candidates| candidates.pop());
        match unique_match {
            Some(existing_note) => changes.push(ReconciliationChange::Moved {
                from: existing_note.relative_path.clone(),
                note: discovered_note
                    .into_catalog_note(existing_note.note_id, existing_note.starred),
            }),
            None => changes.push(ReconciliationChange::Added(
                discovered_note.into_catalog_note(NoteId::generate(), false),
            )),
        }
    }

    for missing_notes in existing_by_hash.into_values() {
        changes.extend(missing_notes.into_iter().map(ReconciliationChange::Missing));
    }
    Ok(ReconciliationPlan { changes })
}

fn collect_discovered_notes(
    discovered_notes: impl IntoIterator<Item = DiscoveredNote>,
) -> Result<Vec<DiscoveredNote>, ReconciliationError> {
    let mut paths = HashSet::new();
    let mut collected_notes = Vec::new();
    for discovered_note in discovered_notes {
        if !paths.insert(discovered_note.relative_path.clone()) {
            return Err(ReconciliationError::DuplicateDiscoveredPath {
                path: discovered_note.relative_path,
            });
        }
        collected_notes.push(discovered_note);
    }
    Ok(collected_notes)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{DiscoveredNote, ReconciliationChange, reconcile_notes};
    use crate::{CatalogNote, DocumentKind, NoteId};

    fn discovered(relative_path: &str, hash_byte: u8) -> DiscoveredNote {
        DiscoveredNote {
            relative_path: relative_path.into(),
            kind: DocumentKind::Markdown,
            title: relative_path.to_owned(),
            excerpt: String::new(),
            modified_at: UNIX_EPOCH + Duration::from_secs(1),
            file_size: 1,
            content_hash: vec![hash_byte],
        }
    }

    fn existing(note_id: NoteId, relative_path: &str, hash_byte: u8) -> CatalogNote {
        CatalogNote {
            note_id,
            relative_path: relative_path.into(),
            kind: DocumentKind::Markdown,
            title: relative_path.to_owned(),
            excerpt: String::new(),
            modified_at: UNIX_EPOCH + Duration::from_secs(1),
            file_size: 1,
            content_hash: vec![hash_byte],
            starred: true,
        }
    }

    #[test]
    fn unique_content_hash_preserves_identity_across_moves() {
        let note_id = NoteId::generate();
        let plan = reconcile_notes(vec![existing(note_id, "old.md", 1)], [discovered("new.md", 1)])
            .expect("unique hash move should reconcile");

        assert!(matches!(
            plan.changes.as_slice(),
            [ReconciliationChange::Moved { from, note }]
                if from == std::path::Path::new("old.md")
                    && note.note_id == note_id
                    && note.starred
        ));
    }

    #[test]
    fn ambiguous_hash_does_not_guess_a_move() {
        let first_id = NoteId::generate();
        let second_id = NoteId::generate();
        let plan = reconcile_notes(
            vec![existing(first_id, "first.md", 1), existing(second_id, "second.md", 1)],
            [discovered("new.md", 1)],
        )
        .expect("ambiguous hash reconciliation should complete");

        assert!(matches!(plan.changes.first(), Some(ReconciliationChange::Added(_))));
        assert_eq!(
            plan.changes
                .iter()
                .filter(|change| matches!(change, ReconciliationChange::Missing(_)))
                .count(),
            2
        );
    }
}
