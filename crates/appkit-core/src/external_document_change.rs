use std::path::PathBuf;

use core::disk_revision::DiskRevision;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExternalDocumentChange {
    Unchanged,
    ReloadClean { revision: DiskRevision },
    PreserveDirtyConflict { disk_revision: DiskRevision },
    DetachDeleted,
    RebindRename { new_path: PathBuf, revision: DiskRevision },
    AmbiguousRename,
}

pub fn classify_external_change(
    baseline: Option<&DiskRevision>,
    observed: Option<&DiskRevision>,
    dirty: bool,
    explicit_rename: Option<&DiskRevision>,
    rename_candidates: &[DiskRevision],
) -> ExternalDocumentChange {
    let Some(baseline) = baseline else {
        return ExternalDocumentChange::Unchanged;
    };
    if let Some(observed) = observed {
        if observed == baseline {
            return ExternalDocumentChange::Unchanged;
        }
        if dirty {
            return ExternalDocumentChange::PreserveDirtyConflict {
                disk_revision: observed.clone(),
            };
        }
        return ExternalDocumentChange::ReloadClean { revision: observed.clone() };
    }
    if let Some(rename_target) = explicit_rename {
        return ExternalDocumentChange::RebindRename {
            new_path: rename_target.path.clone(),
            revision: rename_target.clone(),
        };
    }
    let matching_hash_candidates = rename_candidates
        .iter()
        .filter(|candidate| candidate.content_hash == baseline.content_hash)
        .collect::<Vec<_>>();
    match matching_hash_candidates.as_slice() {
        [candidate] if same_renamed_file(baseline, candidate) => {
            ExternalDocumentChange::RebindRename {
                new_path: candidate.path.clone(),
                revision: (*candidate).clone(),
            }
        }
        [candidate] if !same_renamed_file(baseline, candidate) => {
            ExternalDocumentChange::DetachDeleted
        }
        [] => ExternalDocumentChange::DetachDeleted,
        _ => ExternalDocumentChange::AmbiguousRename,
    }
}

fn same_renamed_file(baseline: &DiskRevision, candidate: &DiskRevision) -> bool {
    if baseline.content_hash != candidate.content_hash {
        return false;
    }
    match (&baseline.file_identity, &candidate.file_identity) {
        (Some(baseline_identity), Some(candidate_identity)) => {
            baseline_identity == candidate_identity
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::disk_revision::read_disk_revision;
    use std::fs;

    fn revision(path: &std::path::Path) -> DiskRevision {
        read_disk_revision(path)
            .expect("revision should be readable")
            .expect("test file should exist")
    }

    #[test]
    fn classifies_unchanged_and_clean_external_changes() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("file should be written");
        let baseline = revision(&path);
        assert_eq!(
            classify_external_change(Some(&baseline), Some(&baseline), false, None, &[]),
            ExternalDocumentChange::Unchanged
        );
        fs::write(&path, "remote").expect("external change should be written");
        let observed = revision(&path);
        assert!(matches!(
            classify_external_change(Some(&baseline), Some(&observed), false, None, &[]),
            ExternalDocumentChange::ReloadClean { .. }
        ));
    }

    #[test]
    fn classifies_dirty_external_change_as_conflict_preservation() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("file should be written");
        let baseline = revision(&path);
        fs::write(&path, "remote").expect("external change should be written");
        let observed = revision(&path);
        assert_eq!(
            classify_external_change(Some(&baseline), Some(&observed), true, None, &[]),
            ExternalDocumentChange::PreserveDirtyConflict { disk_revision: observed }
        );
    }

    #[test]
    fn classifies_deleted_and_unique_or_ambiguous_renames() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("old.md");
        let renamed = directory.path().join("new.md");
        fs::write(&original, "same").expect("original should be written");
        let baseline = revision(&original);
        fs::rename(&original, &renamed).expect("file should be renamed");
        let renamed_revision = revision(&renamed);
        assert_eq!(
            classify_external_change(
                Some(&baseline),
                None,
                false,
                None,
                std::slice::from_ref(&renamed_revision),
            ),
            ExternalDocumentChange::RebindRename {
                new_path: renamed.clone(),
                revision: renamed_revision.clone(),
            }
        );
        assert_eq!(
            classify_external_change(Some(&baseline), None, false, None, &[]),
            ExternalDocumentChange::DetachDeleted
        );
        let copy = directory.path().join("copy.md");
        fs::copy(&renamed, &copy).expect("copy should be created");
        let copy_revision = revision(&copy);
        assert_eq!(
            classify_external_change(
                Some(&baseline),
                None,
                false,
                None,
                &[renamed_revision, copy_revision],
            ),
            ExternalDocumentChange::AmbiguousRename
        );
    }

    #[test]
    fn explicit_rename_wins_without_guessing_from_candidates() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("old.md");
        let renamed = directory.path().join("new.md");
        fs::write(&original, "same").expect("original should be written");
        let baseline = revision(&original);
        fs::write(&renamed, "different").expect("renamed path should be written");
        let renamed_revision = revision(&renamed);
        assert!(matches!(
            classify_external_change(Some(&baseline), None, false, Some(&renamed_revision), &[],),
            ExternalDocumentChange::RebindRename { .. }
        ));
    }
}
