use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::{Catalog, CatalogError, NoteId, TrashEntry, Workspace, WorkspaceError};

const TRASH_DIRECTORY_NAME: &str = "trash";
const DELETION_STAGING_DIRECTORY_NAME: &str = "delete-staging";

/// 回收站文件操作失败；所有目标均由 `NoteId` 和 catalog entry 精确解析。
#[derive(Debug)]
pub enum TrashError {
    Catalog(CatalogError),
    Workspace(WorkspaceError),
    MissingActiveNote { note_id: NoteId },
    MissingTrashEntry { note_id: NoteId },
    RestoreConflict { path: PathBuf },
    InvalidTrashPath { path: PathBuf },
    Io { operation: &'static str, path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for TrashError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(source) => write!(formatter, "trash catalog operation failed: {source}"),
            Self::Workspace(source) => {
                write!(formatter, "trash workspace validation failed: {source}")
            }
            Self::MissingActiveNote { note_id } => {
                write!(formatter, "active note {note_id} does not exist")
            }
            Self::MissingTrashEntry { note_id } => {
                write!(formatter, "trashed note {note_id} does not exist")
            }
            Self::RestoreConflict { path } => {
                write!(formatter, "restore would overwrite {}", path.display())
            }
            Self::InvalidTrashPath { path } => {
                write!(formatter, "invalid trash path {}", path.display())
            }
            Self::Io { operation, path, source } => {
                write!(formatter, "trash {operation} failed for {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for TrashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(source) => Some(source),
            Self::Workspace(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::MissingActiveNote { .. }
            | Self::MissingTrashEntry { .. }
            | Self::RestoreConflict { .. }
            | Self::InvalidTrashPath { .. } => None,
        }
    }
}

/// 原子移动一篇活动笔记并记录 Trash metadata。catalog 提交失败时会尝试移回源文件。
pub fn move_to_trash(
    workspace: &Workspace,
    catalog: &Catalog,
    note_id: NoteId,
) -> Result<TrashEntry, TrashError> {
    let note = catalog
        .active_note(note_id)
        .map_err(TrashError::Catalog)?
        .ok_or(TrashError::MissingActiveNote { note_id })?;
    let source_path =
        workspace.resolve_relative_path(&note.relative_path).map_err(TrashError::Workspace)?;
    let file_name = source_path
        .file_name()
        .filter(|file_name| !file_name.is_empty())
        .ok_or_else(|| TrashError::InvalidTrashPath { path: source_path.clone() })?;
    let trash_relative_path =
        Path::new(".notora").join(TRASH_DIRECTORY_NAME).join(note_id.to_string()).join(file_name);
    let trash_path = prepare_controlled_trash_destination(workspace, &trash_relative_path)?;
    fs::rename(&source_path, &trash_path)
        .map_err(|source| io_error("move to trash", &source_path, source))?;
    match catalog.record_note_trashed(note_id, &trash_relative_path, SystemTime::now()) {
        Ok(entry) => Ok(entry),
        Err(error) => {
            let _ = fs::rename(&trash_path, &source_path);
            Err(TrashError::Catalog(error))
        }
    }
}

/// 恢复一篇精确 Trash entry；原路径已有文件时返回冲突，绝不覆盖。
pub fn restore_from_trash(
    workspace: &Workspace,
    catalog: &Catalog,
    note_id: NoteId,
) -> Result<TrashEntry, TrashError> {
    restore_from_trash_to_relative_path(workspace, catalog, note_id, None)
}

/// 原路径冲突时，以确定且可见的新文件名恢复同一份 Trash entry，绝不覆盖已有文件。
pub fn restore_from_trash_with_renamed_path(
    workspace: &Workspace,
    catalog: &Catalog,
    note_id: NoteId,
) -> Result<TrashEntry, TrashError> {
    let entry = catalog
        .trash_entry(note_id)
        .map_err(TrashError::Catalog)?
        .ok_or(TrashError::MissingTrashEntry { note_id })?;
    let restored_relative_path =
        renamed_restore_relative_path(&entry.original_relative_path, note_id)?;
    restore_from_trash_to_relative_path(workspace, catalog, note_id, Some(restored_relative_path))
}

fn restore_from_trash_to_relative_path(
    workspace: &Workspace,
    catalog: &Catalog,
    note_id: NoteId,
    restored_relative_path: Option<PathBuf>,
) -> Result<TrashEntry, TrashError> {
    let entry = catalog
        .trash_entry(note_id)
        .map_err(TrashError::Catalog)?
        .ok_or(TrashError::MissingTrashEntry { note_id })?;
    let trash_path = resolve_controlled_trash_path(workspace, &entry.trash_relative_path)?;
    let restored_relative_path =
        restored_relative_path.unwrap_or_else(|| entry.original_relative_path.clone());
    let restore_path =
        workspace.resolve_relative_path(&restored_relative_path).map_err(TrashError::Workspace)?;
    if restore_path.exists() {
        return Err(TrashError::RestoreConflict { path: restore_path });
    }
    let parent = restore_path
        .parent()
        .ok_or_else(|| TrashError::InvalidTrashPath { path: restore_path.clone() })?;
    fs::create_dir_all(parent)
        .map_err(|source| io_error("create restore directory", parent, source))?;
    fs::rename(&trash_path, &restore_path)
        .map_err(|source| io_error("restore", &trash_path, source))?;
    match catalog.restore_trashed_note_to_path(note_id, Some(&restored_relative_path)) {
        Ok(restored) => Ok(restored),
        Err(error) => {
            let _ = fs::rename(&restore_path, &trash_path);
            Err(TrashError::Catalog(error))
        }
    }
}

fn renamed_restore_relative_path(
    original_relative_path: &Path,
    note_id: NoteId,
) -> Result<PathBuf, TrashError> {
    let file_name =
        original_relative_path.file_name().filter(|file_name| !file_name.is_empty()).ok_or_else(
            || TrashError::InvalidTrashPath { path: original_relative_path.to_path_buf() },
        )?;
    let stem = Path::new(file_name).file_stem().unwrap_or(file_name).to_string_lossy();
    let suffix = note_id.to_string();
    let renamed_file_name = match Path::new(file_name).extension() {
        Some(extension) => format!("{stem} (restored {suffix}).{}", extension.to_string_lossy()),
        None => format!("{stem} (restored {suffix})"),
    };
    Ok(original_relative_path.parent().unwrap_or_else(|| Path::new("")).join(renamed_file_name))
}

/// 永久删除一篇精确 Trash entry。文件会先移入受控 staging，再删除 catalog metadata。
pub fn permanently_delete_trashed_note(
    workspace: &Workspace,
    catalog: &Catalog,
    note_id: NoteId,
) -> Result<(), TrashError> {
    let entry = catalog
        .trash_entry(note_id)
        .map_err(TrashError::Catalog)?
        .ok_or(TrashError::MissingTrashEntry { note_id })?;
    let trash_path = resolve_controlled_trash_path(workspace, &entry.trash_relative_path)?;
    let file_name = trash_path
        .file_name()
        .ok_or_else(|| TrashError::InvalidTrashPath { path: trash_path.clone() })?;
    let staging_relative_path = Path::new(".notora")
        .join(DELETION_STAGING_DIRECTORY_NAME)
        .join(note_id.to_string())
        .join(file_name);
    let staging_path = prepare_controlled_trash_destination(workspace, &staging_relative_path)?;
    fs::rename(&trash_path, &staging_path)
        .map_err(|source| io_error("stage permanent deletion", &trash_path, source))?;
    if let Err(error) = catalog.permanently_delete_trashed_note(note_id) {
        let _ = fs::rename(&staging_path, &trash_path);
        return Err(TrashError::Catalog(error));
    }
    fs::remove_file(&staging_path)
        .map_err(|source| io_error("permanently delete", &staging_path, source))?;
    Ok(())
}

/// 清空在调用开始时已解析出的 Trash entry 列表；绝不递归删除工作区目录。
pub fn empty_trash(workspace: &Workspace, catalog: &Catalog) -> Result<(), TrashError> {
    let entries = catalog.trash_entries().map_err(TrashError::Catalog)?;
    for entry in entries {
        permanently_delete_trashed_note(workspace, catalog, entry.note_id)?;
    }
    Ok(())
}

fn prepare_controlled_trash_destination(
    workspace: &Workspace,
    relative_path: &Path,
) -> Result<PathBuf, TrashError> {
    validate_controlled_trash_relative_path(relative_path)?;
    let absolute_path = workspace.root().join(relative_path);
    let parent = absolute_path
        .parent()
        .ok_or_else(|| TrashError::InvalidTrashPath { path: absolute_path.clone() })?;
    fs::create_dir_all(parent)
        .map_err(|source| io_error("create trash directory", parent, source))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|source| io_error("canonicalize trash directory", parent, source))?;
    if !canonical_parent.starts_with(workspace.metadata_directory()) {
        return Err(TrashError::InvalidTrashPath { path: absolute_path });
    }
    Ok(absolute_path)
}

fn resolve_controlled_trash_path(
    workspace: &Workspace,
    relative_path: &Path,
) -> Result<PathBuf, TrashError> {
    validate_controlled_trash_relative_path(relative_path)?;
    let absolute_path = workspace.root().join(relative_path);
    let canonical_path = fs::canonicalize(&absolute_path)
        .map_err(|source| io_error("canonicalize trash file", &absolute_path, source))?;
    if !canonical_path.starts_with(workspace.metadata_directory()) {
        return Err(TrashError::InvalidTrashPath { path: absolute_path });
    }
    Ok(canonical_path)
}

fn validate_controlled_trash_relative_path(relative_path: &Path) -> Result<(), TrashError> {
    let components = relative_path.components().collect::<Vec<_>>();
    let valid = matches!(components.as_slice(), [Component::Normal(metadata), Component::Normal(area), Component::Normal(_), Component::Normal(_)]
        if *metadata == OsStr::new(".notora")
            && (*area == OsStr::new(TRASH_DIRECTORY_NAME) || *area == OsStr::new(DELETION_STAGING_DIRECTORY_NAME)));
    if valid {
        return Ok(());
    }
    Err(TrashError::InvalidTrashPath { path: relative_path.to_path_buf() })
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> TrashError {
    TrashError::Io { operation, path: path.to_path_buf(), source }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};

    use crate::{Catalog, CatalogNote, DocumentKind, NoteId, Workspace};

    #[test]
    fn moving_restoring_and_permanently_deleting_a_note_preserves_metadata_until_final_delete() {
        let directory = tempfile::tempdir().expect("workspace fixture directory should exist");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should open");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        let note_path = workspace.root().join("plans/roadmap.md");
        fs::create_dir_all(note_path.parent().expect("fixture note should have a parent"))
            .expect("fixture parent should exist");
        fs::write(&note_path, "# Roadmap").expect("fixture note should write");
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: "plans/roadmap.md".into(),
                kind: DocumentKind::Markdown,
                title: "Roadmap".to_owned(),
                excerpt: "fixture".to_owned(),
                modified_at: UNIX_EPOCH + Duration::from_secs(1),
                file_size: 9,
                content_hash: vec![1, 2, 3],
                starred: false,
            })
            .expect("fixture catalog note should persist");
        let tag = catalog.create_tag("Plan").expect("fixture tag should create");
        assert!(catalog.attach_tag(note_id, tag.tag_id).expect("fixture tag should attach"));

        let trashed =
            super::move_to_trash(&workspace, &catalog, note_id).expect("note should move to trash");
        assert!(!note_path.exists());
        assert!(workspace.root().join(&trashed.trash_relative_path).is_file());
        assert_eq!(
            catalog.tags_for_note(note_id).expect("metadata should remain"),
            vec![tag.clone()]
        );

        super::restore_from_trash(&workspace, &catalog, note_id).expect("note should restore");
        assert!(note_path.is_file());
        assert_eq!(catalog.tags_for_note(note_id).expect("metadata should remain"), vec![tag]);

        let _ =
            super::move_to_trash(&workspace, &catalog, note_id).expect("note should trash again");
        super::permanently_delete_trashed_note(&workspace, &catalog, note_id)
            .expect("trashed note should delete exactly once");
        assert!(catalog.trash_entry(note_id).expect("trash entry should query").is_none());
        assert!(catalog.tags_for_note(note_id).expect("deleted note has no metadata").is_empty());
    }

    #[test]
    fn restore_refuses_to_overwrite_a_new_file_at_the_original_path() {
        let directory = tempfile::tempdir().expect("workspace fixture directory should exist");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should open");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let note_id = NoteId::generate();
        let note_path = workspace.root().join("note.md");
        fs::write(&note_path, "# Original").expect("fixture note should write");
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: "note.md".into(),
                kind: DocumentKind::Markdown,
                title: "Note".to_owned(),
                excerpt: "fixture".to_owned(),
                modified_at: UNIX_EPOCH + Duration::from_secs(1),
                file_size: 10,
                content_hash: vec![1, 2, 3],
                starred: false,
            })
            .expect("fixture catalog note should persist");
        super::move_to_trash(&workspace, &catalog, note_id).expect("note should move to trash");
        fs::write(&note_path, "# Replacement").expect("replacement should write");

        assert!(matches!(
            super::restore_from_trash(&workspace, &catalog, note_id),
            Err(super::TrashError::RestoreConflict { .. })
        ));

        super::restore_from_trash_with_renamed_path(&workspace, &catalog, note_id)
            .expect("renamed restore should keep both files");
        let restored_note = catalog
            .active_note(note_id)
            .expect("restored note should query")
            .expect("restored note should remain active");
        assert_ne!(restored_note.relative_path, std::path::PathBuf::from("note.md"));
        assert!(workspace.root().join(restored_note.relative_path).is_file());
    }
}
