use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::{Catalog, CatalogError, CatalogNote, DocumentKind, NoteEncryption, NoteId};

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
    pub encryption: NoteEncryption,
}

impl DiscoveredNote {
    fn into_reconciled_note(self, note_id: NoteId, starred: bool, title: String) -> ReconciledNote {
        ReconciledNote {
            note: CatalogNote {
                note_id,
                relative_path: self.relative_path,
                kind: self.kind,
                title,
                excerpt: self.excerpt,
                modified_at: self.modified_at,
                file_size: self.file_size,
                content_hash: self.content_hash,
                starred,
            },
            encryption: self.encryption,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledNote {
    pub note: CatalogNote,
    pub encryption: NoteEncryption,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationChange {
    Updated(ReconciledNote),
    Moved { from: PathBuf, reconciled: ReconciledNote },
    Added(ReconciledNote),
    Missing(CatalogNote),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconciliationPlan {
    pub changes: Vec<ReconciliationChange>,
}

#[derive(Debug)]
pub enum ReconciliationError {
    Catalog(CatalogError),
    DuplicateDiscoveredPath {
        path: PathBuf,
    },
    RenameTargetAlreadyTracked {
        from: PathBuf,
        to: PathBuf,
    },
    RenameChangesDocumentKind {
        from: PathBuf,
        to: PathBuf,
        from_kind: DocumentKind,
        to_kind: DocumentKind,
    },
}

impl std::fmt::Display for ReconciliationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(source) => write!(formatter, "catalog reconciliation failed: {source}"),
            Self::DuplicateDiscoveredPath { path } => {
                write!(formatter, "scanner produced duplicate relative path: {}", path.display())
            }
            Self::RenameTargetAlreadyTracked { from, to } => write!(
                formatter,
                "external rename from {} would overwrite the tracked note at {}",
                from.display(),
                to.display()
            ),
            Self::RenameChangesDocumentKind { from, to, from_kind, to_kind } => write!(
                formatter,
                "external rename from {} ({from_kind:?}) to {} ({to_kind:?}) changes document kind",
                from.display(),
                to.display()
            ),
        }
    }
}

impl std::error::Error for ReconciliationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(source) => Some(source),
            Self::DuplicateDiscoveredPath { .. }
            | Self::RenameTargetAlreadyTracked { .. }
            | Self::RenameChangesDocumentKind { .. } => None,
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
    reconcile_notes_with_renames(existing_notes, discovered_notes, &[])
}

pub(crate) fn reconcile_notes_with_renames(
    existing_notes: Vec<CatalogNote>,
    discovered_notes: impl IntoIterator<Item = DiscoveredNote>,
    rename_hints: &[(PathBuf, PathBuf)],
) -> Result<ReconciliationPlan, ReconciliationError> {
    let discovered_notes = collect_discovered_notes(discovered_notes)?;
    validate_rename_hints(&existing_notes, &discovered_notes, rename_hints)?;
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
        changes.push(ReconciliationChange::Updated(discovered_note.into_reconciled_note(
            existing_note.note_id,
            existing_note.starred,
            existing_note.title,
        )));
    }

    let renamed_from_by_to =
        rename_hints.iter().map(|(from, to)| (to, from)).collect::<HashMap<_, _>>();
    let mut still_unmatched_discovered = Vec::new();
    for discovered_note in unmatched_discovered {
        let Some(from) = renamed_from_by_to.get(&discovered_note.relative_path) else {
            still_unmatched_discovered.push(discovered_note);
            continue;
        };
        let Some(existing_note) = existing_by_path.remove(*from) else {
            still_unmatched_discovered.push(discovered_note);
            continue;
        };
        let basename_changed =
            existing_note.relative_path.file_name() != discovered_note.relative_path.file_name();
        let title =
            if basename_changed { discovered_note.title.clone() } else { existing_note.title };
        changes.push(ReconciliationChange::Moved {
            from: existing_note.relative_path.clone(),
            reconciled: discovered_note.into_reconciled_note(
                existing_note.note_id,
                existing_note.starred,
                title,
            ),
        });
    }

    let mut existing_by_hash = HashMap::<Vec<u8>, Vec<CatalogNote>>::new();
    for existing_note in existing_by_path.into_values() {
        existing_by_hash.entry(existing_note.content_hash.clone()).or_default().push(existing_note);
    }

    for discovered_note in still_unmatched_discovered {
        let has_unique_hash_match = existing_by_hash
            .get(&discovered_note.content_hash)
            .is_some_and(|candidates| candidates.len() == 1);
        let unique_match = has_unique_hash_match
            .then(|| existing_by_hash.remove(&discovered_note.content_hash))
            .flatten()
            .and_then(|mut candidates| candidates.pop());
        match unique_match {
            Some(existing_note) => {
                let basename_changed = existing_note.relative_path.file_name()
                    != discovered_note.relative_path.file_name();
                let title = if basename_changed {
                    discovered_note.title.clone()
                } else {
                    existing_note.title
                };
                changes.push(ReconciliationChange::Moved {
                    from: existing_note.relative_path.clone(),
                    reconciled: discovered_note.into_reconciled_note(
                        existing_note.note_id,
                        existing_note.starred,
                        title,
                    ),
                });
            }
            None => {
                let title = discovered_note.title.clone();
                changes.push(ReconciliationChange::Added(discovered_note.into_reconciled_note(
                    NoteId::generate(),
                    false,
                    title,
                )))
            }
        }
    }

    for missing_notes in existing_by_hash.into_values() {
        changes.extend(missing_notes.into_iter().map(ReconciliationChange::Missing));
    }
    Ok(ReconciliationPlan { changes })
}

fn validate_rename_hints(
    existing_notes: &[CatalogNote],
    discovered_notes: &[DiscoveredNote],
    rename_hints: &[(PathBuf, PathBuf)],
) -> Result<(), ReconciliationError> {
    let existing_by_path =
        existing_notes.iter().map(|note| (&note.relative_path, note)).collect::<HashMap<_, _>>();
    let discovered_by_path =
        discovered_notes.iter().map(|note| (&note.relative_path, note)).collect::<HashMap<_, _>>();
    for (from, to) in rename_hints {
        let Some(source_note) = existing_by_path.get(from) else {
            continue;
        };
        if from != to && existing_by_path.contains_key(to) {
            return Err(ReconciliationError::RenameTargetAlreadyTracked {
                from: from.clone(),
                to: to.clone(),
            });
        }
        let Some(target_note) = discovered_by_path.get(to) else {
            continue;
        };
        if source_note.kind != target_note.kind {
            return Err(ReconciliationError::RenameChangesDocumentKind {
                from: from.clone(),
                to: to.clone(),
                from_kind: source_note.kind,
                to_kind: target_note.kind,
            });
        }
    }
    Ok(())
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

    use super::{
        DiscoveredNote, ReconciliationChange, reconcile_notes, reconcile_notes_with_renames,
    };
    use crate::{CatalogNote, DocumentKind, NoteEncryption, NoteId};

    fn discovered(relative_path: &str, hash_byte: u8) -> DiscoveredNote {
        DiscoveredNote {
            relative_path: relative_path.into(),
            kind: DocumentKind::Markdown,
            title: relative_path.to_owned(),
            excerpt: String::new(),
            modified_at: UNIX_EPOCH + Duration::from_secs(1),
            file_size: 1,
            content_hash: vec![hash_byte],
            encryption: NoteEncryption::Unencrypted,
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
            [ReconciliationChange::Moved { from, reconciled }]
                if from == std::path::Path::new("old.md")
                    && reconciled.note.note_id == note_id
                    && reconciled.note.starred
                    && reconciled.note.title == "new.md"
        ));
    }

    #[test]
    fn directory_only_move_preserves_the_existing_notora_title() {
        let note_id = NoteId::generate();
        let mut old = existing(note_id, "old/note.md", 1);
        old.title = "Independent title".to_owned();
        let plan = reconcile_notes(vec![old], [discovered("new/note.md", 1)])
            .expect("directory move should reconcile");

        assert!(matches!(
            plan.changes.as_slice(),
            [ReconciliationChange::Moved { reconciled, .. }]
                if reconciled.note.title == "Independent title"
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

    #[test]
    fn explicit_rename_preserves_identity_when_content_hash_is_ambiguous() {
        let renamed_id = NoteId::generate();
        let unchanged_id = NoteId::generate();
        let plan = reconcile_notes_with_renames(
            vec![existing(renamed_id, "first.md", 1), existing(unchanged_id, "second.md", 1)],
            [discovered("renamed.md", 1), discovered("second.md", 1)],
            &[("first.md".into(), "renamed.md".into())],
        )
        .expect("explicit rename should reconcile");

        assert!(plan.changes.iter().any(|change| matches!(
            change,
            ReconciliationChange::Moved { from, reconciled }
                if from == std::path::Path::new("first.md")
                    && reconciled.note.relative_path == std::path::Path::new("renamed.md")
                    && reconciled.note.note_id == renamed_id
        )));
        assert!(plan.changes.iter().any(|change| matches!(
            change,
            ReconciliationChange::Updated(reconciled)
                if reconciled.note.note_id == unchanged_id
        )));
    }

    #[test]
    fn explicit_rename_cannot_overwrite_another_tracked_note_identity() {
        let result = reconcile_notes_with_renames(
            vec![
                existing(NoteId::generate(), "source.md", 1),
                existing(NoteId::generate(), "target.md", 2),
            ],
            [discovered("target.md", 1)],
            &[("source.md".into(), "target.md".into())],
        );

        assert!(matches!(
            result,
            Err(super::ReconciliationError::RenameTargetAlreadyTracked { from, to })
                if from == std::path::Path::new("source.md")
                    && to == std::path::Path::new("target.md")
        ));
    }

    #[test]
    fn explicit_rename_cannot_change_the_document_kind_of_a_note_identity() {
        let result = reconcile_notes_with_renames(
            vec![existing(NoteId::generate(), "source.md", 1)],
            [DiscoveredNote { kind: DocumentKind::Text, ..discovered("source.txt", 1) }],
            &[("source.md".into(), "source.txt".into())],
        );

        assert!(matches!(
            result,
            Err(super::ReconciliationError::RenameChangesDocumentKind {
                from_kind: DocumentKind::Markdown,
                to_kind: DocumentKind::Text,
                ..
            })
        ));
    }
}
