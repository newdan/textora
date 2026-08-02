use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::catalog::SearchIndexEntry;
use crate::{
    Catalog, CatalogError, DiscoveredNote, DocumentKind, ReconciliationChange, ReconciliationError,
    WORKSPACE_METADATA_DIRECTORY_NAME, Workspace, parse_note_text_summary, reconcile_catalog,
};

const MACOS_FINDER_METADATA_FILE_NAME: &str = ".DS_Store";
const MACOS_RESOURCE_FORK_PREFIX: &str = "._";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFailure {
    pub relative_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanCompletion {
    pub discovered_files: usize,
    pub indexed_files: usize,
    pub failures: Vec<ScanFailure>,
}

#[derive(Debug)]
pub enum ScanError {
    Catalog(CatalogError),
    Reconciliation(ReconciliationError),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(source) => {
                write!(formatter, "workspace scan catalog access failed: {source}")
            }
            Self::Reconciliation(source) => source.fmt(formatter),
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(source) => Some(source),
            Self::Reconciliation(source) => Some(source),
        }
    }
}

/// 扫描支持的笔记文件并写入 catalog。
///
/// 调用方必须在后台 worker 调用此函数；它会读取文件和执行 SQLite 写入。
pub fn scan_workspace(
    workspace: &Workspace,
    catalog: &Catalog,
) -> Result<ScanCompletion, ScanError> {
    let mut completion = ScanCompletion::default();
    let mut discovered_files = Vec::new();
    scan_directory(workspace, workspace.root(), &mut discovered_files, &mut completion);
    let plan = reconcile_catalog(catalog, discovered_files.iter().map(|file| file.note.clone()))
        .map_err(ScanError::Reconciliation)?;
    let mut note_ids_by_path = HashMap::new();
    let mut missing_note_ids = Vec::new();
    for change in plan.changes {
        match change {
            ReconciliationChange::Updated(note)
            | ReconciliationChange::Added(note)
            | ReconciliationChange::Moved { note, .. } => {
                note_ids_by_path.insert(note.relative_path.clone(), note.note_id);
                catalog.upsert_active_note(&note).map_err(ScanError::Catalog)?;
            }
            ReconciliationChange::Missing(note) => missing_note_ids.push(note.note_id),
        }
    }
    if completion.failures.is_empty() {
        catalog.remove_missing_active_notes(&missing_note_ids).map_err(ScanError::Catalog)?;
    }
    let search_entries = discovered_files
        .into_iter()
        .filter_map(|file| {
            let note_id = note_ids_by_path.get(&file.note.relative_path).copied()?;
            Some(SearchIndexEntry {
                note_id,
                title: file.note.title,
                relative_path: file.note.relative_path,
                body: file.body,
                tags: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    catalog.index_note_batch(&search_entries).map_err(ScanError::Catalog)?;
    completion.indexed_files = search_entries.len();
    Ok(completion)
}

struct DiscoveredFile {
    note: DiscoveredNote,
    body: String,
}

fn scan_directory(
    workspace: &Workspace,
    directory: &Path,
    discovered_files: &mut Vec<DiscoveredFile>,
    completion: &mut ScanCompletion,
) {
    let read_directory = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(source) => {
            push_io_failure(workspace, directory, source, completion);
            return;
        }
    };
    let mut entries = Vec::new();
    for entry in read_directory {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(source) => push_io_failure(workspace, directory, source, completion),
        }
    }
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(source) => {
                push_io_failure(workspace, &path, source, completion);
                continue;
            }
        };
        if file_type.is_symlink() || should_ignore(&path) {
            continue;
        }
        if file_type.is_dir() {
            scan_directory(workspace, &path, discovered_files, completion);
            continue;
        }
        if file_type.is_file() {
            scan_file(workspace, &path, discovered_files, completion);
        }
    }
}

fn scan_file(
    workspace: &Workspace,
    path: &Path,
    discovered_files: &mut Vec<DiscoveredFile>,
    completion: &mut ScanCompletion,
) {
    let Some(kind) = DocumentKind::from_path(path) else {
        return;
    };
    completion.discovered_files += 1;
    let relative_path = match path.strip_prefix(workspace.root()) {
        Ok(relative_path) => relative_path.to_path_buf(),
        Err(_) => {
            completion.failures.push(ScanFailure {
                relative_path: path.to_path_buf(),
                message: "scanner encountered a path outside the workspace root".to_owned(),
            });
            return;
        }
    };
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) => {
            push_io_failure(workspace, path, source, completion);
            return;
        }
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(source) => {
            push_io_failure(workspace, path, source, completion);
            return;
        }
    };
    let modified_at = match metadata.modified() {
        Ok(modified_at) => modified_at,
        Err(source) => {
            push_io_failure(workspace, path, source, completion);
            return;
        }
    };
    let Some(file_stem) = note_file_stem(path, kind) else {
        completion.failures.push(ScanFailure {
            relative_path,
            message: "supported note file has no valid UTF-8 file stem".to_owned(),
        });
        return;
    };
    let summary = parse_note_text_summary(kind, file_stem, &contents);
    let note = DiscoveredNote {
        relative_path: relative_path.clone(),
        kind,
        title: summary.title,
        excerpt: summary.excerpt,
        modified_at,
        file_size: metadata.len(),
        content_hash: blake3::hash(contents.as_bytes()).as_bytes().to_vec(),
    };
    discovered_files.push(DiscoveredFile { note, body: contents });
}

fn should_ignore(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    file_name == WORKSPACE_METADATA_DIRECTORY_NAME
        || file_name == MACOS_FINDER_METADATA_FILE_NAME
        || file_name.starts_with(MACOS_RESOURCE_FORK_PREFIX)
}

fn note_file_stem(path: &Path, kind: DocumentKind) -> Option<&str> {
    let file_name = path.file_name()?.to_str()?;
    match kind {
        DocumentKind::Mindmap => file_name.strip_suffix(".mmap.md"),
        DocumentKind::Markdown => file_name.strip_suffix(".md"),
        DocumentKind::Text => file_name.strip_suffix(".txt"),
    }
}

fn push_io_failure(
    workspace: &Workspace,
    path: &Path,
    source: std::io::Error,
    completion: &mut ScanCompletion,
) {
    let relative_path = path.strip_prefix(workspace.root()).unwrap_or(path).to_path_buf();
    completion.failures.push(ScanFailure { relative_path, message: source.to_string() });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::scan_workspace;
    use crate::{Catalog, DocumentKind, Workspace};

    #[test]
    fn scan_indexes_supported_notes_and_preserves_existing_note_ids() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::write(directory.path().join("note.md"), "# Title\n\nBody")
            .expect("markdown fixture should be written");
        fs::write(directory.path().join("map.mmap.md"), "# Map")
            .expect("mindmap fixture should be written");
        fs::write(directory.path().join("ignored.png"), "binary")
            .expect("unsupported fixture should be written");

        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        fs::write(workspace.metadata_directory().join("hidden.md"), "# Hidden")
            .expect("metadata fixture should be written");

        let first_completion =
            scan_workspace(&workspace, &catalog).expect("first scan should complete");
        let first_notes = catalog.active_notes().expect("first scan notes should load");
        let second_completion =
            scan_workspace(&workspace, &catalog).expect("second scan should complete");
        let second_notes = catalog.active_notes().expect("second scan notes should load");

        assert_eq!(first_completion.discovered_files, 2);
        assert_eq!(first_completion.indexed_files, 2);
        assert!(first_completion.failures.is_empty());
        assert_eq!(second_completion.indexed_files, 2);
        assert_eq!(first_notes, second_notes);
        assert_eq!(first_notes[0].kind, DocumentKind::Mindmap);
        assert_eq!(first_notes[1].kind, DocumentKind::Markdown);
        assert!(
            catalog
                .search_active_notes("Body", 10)
                .expect("scanner should write searchable note bodies")
                .iter()
                .any(|search_match| search_match.note_id == first_notes[1].note_id)
        );
    }

    #[test]
    fn rescan_removes_deleted_notes_from_active_catalog_and_search() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("obsolete.md");
        fs::write(&note_path, "# Obsolete\n\nunique deletion marker")
            .expect("markdown fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");

        fs::remove_file(note_path).expect("fixture note should be deleted");
        scan_workspace(&workspace, &catalog).expect("deletion scan should complete");

        assert!(catalog.active_notes().expect("active notes should load").is_empty());
        assert!(
            catalog
                .search_active_notes("deletion marker", 10)
                .expect("search should complete")
                .is_empty()
        );
    }

    #[test]
    fn rescan_preserves_note_identity_for_a_unique_external_move() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let original_path = directory.path().join("original.md");
        let moved_path = directory.path().join("moved.md");
        fs::write(&original_path, "# Stable identity\n\nmove marker")
            .expect("markdown fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let original_note = catalog.active_notes().expect("initial notes should load").remove(0);

        fs::rename(original_path, moved_path).expect("fixture note should move");
        scan_workspace(&workspace, &catalog).expect("move scan should complete");
        let moved_note = catalog.active_notes().expect("moved notes should load").remove(0);

        assert_eq!(moved_note.note_id, original_note.note_id);
        assert_eq!(moved_note.relative_path, std::path::Path::new("moved.md"));
    }
}
