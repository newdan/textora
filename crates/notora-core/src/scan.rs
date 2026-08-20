use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::catalog::SearchIndexEntry;
use crate::file_monitor::{WorkspaceFileBatch, WorkspaceFileChange};
use crate::reconciliation::reconcile_notes_with_renames;
use crate::{
    Catalog, CatalogError, CatalogNote, DiscoveredNote, DocumentKind, NoteEncryption,
    ReconciliationChange, ReconciliationError, WORKSPACE_METADATA_DIRECTORY_NAME, Workspace,
    parse_note_text_summary,
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
    let existing_notes = catalog.active_notes().map_err(ScanError::Catalog)?;
    let existing_encryption_by_path =
        existing_encryption_by_path(catalog, &existing_notes).map_err(ScanError::Catalog)?;
    let existing_notes_by_path = existing_notes
        .into_iter()
        .map(|note| (note.relative_path.clone(), note))
        .collect::<HashMap<_, _>>();
    let mut discovered_files = Vec::new();
    scan_directory(
        workspace,
        workspace.root(),
        &existing_notes_by_path,
        &existing_encryption_by_path,
        &mut discovered_files,
        &mut completion,
    );
    apply_scan_results(
        catalog,
        &mut completion,
        existing_notes_by_path.into_values().collect(),
        discovered_files,
        &[],
    )?;
    Ok(completion)
}

/// 只扫描 watcher batch 涉及的文件或目录，并保留未受影响的 catalog 行与索引。
pub fn scan_workspace_paths(
    workspace: &Workspace,
    catalog: &Catalog,
    relative_paths: &[PathBuf],
) -> Result<ScanCompletion, ScanError> {
    scan_workspace_paths_with_renames(workspace, catalog, relative_paths, &[])
}

/// 扫描 watcher 批次，并优先使用文件系统提供的明确重命名关系保持 NoteId。
pub fn scan_workspace_file_batch(
    workspace: &Workspace,
    catalog: &Catalog,
    batch: &WorkspaceFileBatch,
) -> Result<ScanCompletion, ScanError> {
    let rename_hints = batch
        .changes
        .iter()
        .filter_map(|change| match change {
            WorkspaceFileChange::Renamed { from, to, .. } => Some((from.clone(), to.clone())),
            WorkspaceFileChange::Created(_)
            | WorkspaceFileChange::Modified(_)
            | WorkspaceFileChange::Removed(_) => None,
        })
        .collect::<Vec<_>>();
    scan_workspace_paths_with_renames(workspace, catalog, &batch.relative_paths, &rename_hints)
}

fn scan_workspace_paths_with_renames(
    workspace: &Workspace,
    catalog: &Catalog,
    relative_paths: &[PathBuf],
    rename_hints: &[(PathBuf, PathBuf)],
) -> Result<ScanCompletion, ScanError> {
    let existing_notes = catalog.active_notes().map_err(ScanError::Catalog)?;
    let existing_encryption_by_path =
        existing_encryption_by_path(catalog, &existing_notes).map_err(ScanError::Catalog)?;
    let existing_notes_by_path = existing_notes
        .iter()
        .cloned()
        .map(|note| (note.relative_path.clone(), note))
        .collect::<HashMap<_, _>>();
    let mut completion = ScanCompletion::default();
    let mut affected_notes_by_path = HashMap::new();
    let mut discovered_files = Vec::new();
    for relative_path in minimal_batch_paths(relative_paths) {
        for note in &existing_notes {
            if note.relative_path == relative_path || note.relative_path.starts_with(&relative_path)
            {
                affected_notes_by_path.insert(note.relative_path.clone(), note.clone());
            }
        }
        let path = match workspace.resolve_relative_path(&relative_path) {
            Ok(path) => path,
            Err(error) => {
                completion.failures.push(ScanFailure { relative_path, message: error.to_string() });
                continue;
            }
        };
        if path.is_dir() {
            scan_directory(
                workspace,
                &path,
                &existing_notes_by_path,
                &existing_encryption_by_path,
                &mut discovered_files,
                &mut completion,
            );
        } else if path.is_file() {
            scan_file(
                workspace,
                &path,
                &existing_notes_by_path,
                &existing_encryption_by_path,
                &mut discovered_files,
                &mut completion,
            );
        }
    }
    let discovered_files = discovered_files
        .into_iter()
        .map(|file| (file.note.relative_path.clone(), file))
        .collect::<HashMap<_, _>>()
        .into_values()
        .collect();
    apply_scan_results(
        catalog,
        &mut completion,
        affected_notes_by_path.into_values().collect(),
        discovered_files,
        rename_hints,
    )?;
    Ok(completion)
}

fn minimal_batch_paths(relative_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = relative_paths.to_vec();
    paths.sort_by(|left, right| {
        left.components().count().cmp(&right.components().count()).then_with(|| left.cmp(right))
    });
    let mut minimal_paths = Vec::<PathBuf>::new();
    for path in paths {
        if minimal_paths.iter().any(|parent| path == *parent || path.starts_with(parent)) {
            continue;
        }
        minimal_paths.push(path);
    }
    minimal_paths
}

fn apply_scan_results(
    catalog: &Catalog,
    completion: &mut ScanCompletion,
    existing_notes: Vec<CatalogNote>,
    discovered_files: Vec<DiscoveredFile>,
    rename_hints: &[(PathBuf, PathBuf)],
) -> Result<(), ScanError> {
    let mut tag_names_by_note_id = catalog.active_note_tag_names().map_err(ScanError::Catalog)?;
    let plan = reconcile_notes_with_renames(
        existing_notes,
        discovered_files.iter().map(|file| file.note.clone()),
        rename_hints,
    )
    .map_err(ScanError::Reconciliation)?;
    let changed_bodies_by_path = discovered_files
        .into_iter()
        .filter_map(|file| file.body.map(|body| (file.note.relative_path, body)))
        .collect::<HashMap<_, _>>();
    let mut missing_note_ids = Vec::new();
    let mut present_note_ids = Vec::new();
    let mut notes_to_upsert = Vec::new();
    let mut search_entries = Vec::new();
    for change in plan.changes {
        match change {
            ReconciliationChange::Updated(reconciled) => {
                let note = reconciled.note;
                present_note_ids.push(note.note_id);
                let Some(body) = changed_bodies_by_path.get(&note.relative_path) else {
                    continue;
                };
                search_entries.push(search_entry(
                    &note,
                    body.clone(),
                    tag_names_by_note_id.remove(&note.note_id).unwrap_or_default(),
                ));
                notes_to_upsert.push((note, reconciled.encryption));
            }
            ReconciliationChange::Added(reconciled) => {
                let note = reconciled.note;
                present_note_ids.push(note.note_id);
                let Some(body) = changed_bodies_by_path.get(&note.relative_path) else {
                    continue;
                };
                search_entries.push(search_entry(
                    &note,
                    body.clone(),
                    tag_names_by_note_id.remove(&note.note_id).unwrap_or_default(),
                ));
                notes_to_upsert.push((note, reconciled.encryption));
            }
            ReconciliationChange::Moved { from, reconciled } => {
                let note = reconciled.note;
                present_note_ids.push(note.note_id);
                let Some(body) = changed_bodies_by_path.get(&note.relative_path) else {
                    continue;
                };
                let external_title = (from.file_name() != note.relative_path.file_name())
                    .then_some(note.title.as_str());
                catalog
                    .apply_external_note_relocation(
                        note.note_id,
                        &note.relative_path,
                        external_title,
                    )
                    .map_err(ScanError::Catalog)?;
                search_entries.push(search_entry(
                    &note,
                    body.clone(),
                    tag_names_by_note_id.remove(&note.note_id).unwrap_or_default(),
                ));
                notes_to_upsert.push((note, reconciled.encryption));
            }
            ReconciliationChange::Missing(note) => missing_note_ids.push(note.note_id),
        }
    }
    let (confirmed_present_note_ids, confirmed_missing_note_ids) = if completion.failures.is_empty()
    {
        (present_note_ids.as_slice(), missing_note_ids.as_slice())
    } else {
        (&[][..], &[][..])
    };
    catalog
        .apply_scan_reconciliation(
            &notes_to_upsert,
            confirmed_present_note_ids,
            confirmed_missing_note_ids,
            &search_entries,
        )
        .map_err(ScanError::Catalog)?;
    completion.indexed_files = search_entries.len();
    Ok(())
}

struct DiscoveredFile {
    note: DiscoveredNote,
    body: Option<String>,
}

fn search_entry(note: &CatalogNote, body: String, tags: Vec<String>) -> SearchIndexEntry {
    SearchIndexEntry {
        note_id: note.note_id,
        title: note.title.clone(),
        relative_path: note.relative_path.clone(),
        body,
        tags,
    }
}

fn scan_directory(
    workspace: &Workspace,
    directory: &Path,
    existing_notes_by_path: &HashMap<PathBuf, CatalogNote>,
    existing_encryption_by_path: &HashMap<PathBuf, NoteEncryption>,
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
            scan_directory(
                workspace,
                &path,
                existing_notes_by_path,
                existing_encryption_by_path,
                discovered_files,
                completion,
            );
            continue;
        }
        if file_type.is_file() {
            scan_file(
                workspace,
                &path,
                existing_notes_by_path,
                existing_encryption_by_path,
                discovered_files,
                completion,
            );
        }
    }
}

fn scan_file(
    workspace: &Workspace,
    path: &Path,
    existing_notes_by_path: &HashMap<PathBuf, CatalogNote>,
    existing_encryption_by_path: &HashMap<PathBuf, NoteEncryption>,
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
    let serialized = match fs::read(path) {
        Ok(serialized) => serialized,
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
    let known_encryption = existing_encryption_by_path.get(&relative_path).copied();
    let encryption = match classify_note_encryption(kind, known_encryption, &serialized) {
        Ok(encryption) => encryption,
        Err(error) => {
            completion.failures.push(ScanFailure { relative_path, message: error.to_string() });
            return;
        }
    };
    let content_hash = blake3::hash(&serialized).as_bytes().to_vec();
    let (excerpt, searchable_body) = match encryption {
        NoteEncryption::Encrypted => (String::new(), String::new()),
        NoteEncryption::Unencrypted => {
            let contents = match String::from_utf8(serialized) {
                Ok(contents) => contents,
                Err(_) => {
                    completion.failures.push(ScanFailure {
                        relative_path,
                        message: "supported note file is not valid UTF-8".to_owned(),
                    });
                    return;
                }
            };
            let summary = parse_note_text_summary(kind, file_stem, &contents);
            (summary.excerpt, contents)
        }
    };
    let is_unchanged = existing_notes_by_path.get(&relative_path).is_some_and(|existing_note| {
        existing_note.kind == kind
            && existing_note.content_hash == content_hash
            && known_encryption == Some(encryption)
    });
    let note = DiscoveredNote {
        relative_path: relative_path.clone(),
        kind,
        title: file_stem.to_owned(),
        excerpt,
        modified_at,
        file_size: metadata.len(),
        content_hash,
        encryption,
    };
    let body = (!is_unchanged).then_some(searchable_body);
    discovered_files.push(DiscoveredFile { note, body });
}

fn classify_note_encryption(
    kind: DocumentKind,
    known_encryption: Option<NoteEncryption>,
    serialized: &[u8],
) -> Result<NoteEncryption, textora_encryption::EncryptionError> {
    if kind != DocumentKind::Markdown {
        return Ok(NoteEncryption::Unencrypted);
    }
    if known_encryption == Some(NoteEncryption::Encrypted) {
        textora_encryption::inspect_encrypted_markdown(serialized)?;
        return Ok(NoteEncryption::Encrypted);
    }
    match textora_encryption::inspect_encrypted_markdown(serialized) {
        Ok(_) => Ok(NoteEncryption::Encrypted),
        Err(textora_encryption::EncryptionError::NotEncryptedDocument) => {
            Ok(NoteEncryption::Unencrypted)
        }
        Err(error) => Err(error),
    }
}

fn existing_encryption_by_path(
    catalog: &Catalog,
    existing_notes: &[CatalogNote],
) -> Result<HashMap<PathBuf, NoteEncryption>, CatalogError> {
    let mut encryption_by_path = HashMap::with_capacity(existing_notes.len());
    for note in existing_notes {
        let metadata = catalog.note_editor_metadata(note.note_id)?.ok_or_else(|| {
            CatalogError::InvalidStoredValue {
                column: "note_editor_metadata",
                value: note.note_id.to_string(),
            }
        })?;
        encryption_by_path.insert(note.relative_path.clone(), metadata.encryption);
    }
    Ok(encryption_by_path)
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

    use super::{scan_workspace, scan_workspace_paths};
    use crate::{Catalog, DocumentKind, NoteEncryption, NoteFileNameBinding, Workspace};

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
        assert_eq!(second_completion.indexed_files, 0);
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
    fn scan_rebuilds_encrypted_metadata_without_indexing_plaintext() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let secret_marker = "private-search-marker";
        fs::write(directory.path().join("Visible Title.md"), encrypted_markdown(secret_marker))
            .expect("encrypted fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        let completion = scan_workspace(&workspace, &catalog).expect("scan should complete");
        let note = catalog.active_notes().expect("scanned note should load").remove(0);
        let metadata = catalog
            .note_editor_metadata(note.note_id)
            .expect("encrypted metadata should query")
            .expect("encrypted metadata should exist");

        assert!(completion.failures.is_empty());
        assert_eq!(note.title, "Visible Title");
        assert_eq!(note.excerpt, "");
        assert_eq!(metadata.encryption, NoteEncryption::Encrypted);
        assert_eq!(
            metadata.file_name_binding,
            NoteFileNameBinding::TitleBound { disambiguator: 1 }
        );
        assert_eq!(
            catalog
                .search_active_notes("Visible Title", 10)
                .expect("encrypted title should be searchable")
                .len(),
            1
        );
        assert!(
            catalog
                .search_active_notes(secret_marker, 10)
                .expect("encrypted plaintext search should complete")
                .is_empty()
        );
    }

    #[test]
    fn damaged_known_encrypted_file_is_not_downgraded_to_plain_markdown() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("Encrypted.md");
        fs::write(&note_path, encrypted_markdown("authenticated body"))
            .expect("encrypted fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let note_id = catalog.active_notes().expect("encrypted note should load")[0].note_id;
        let mut tampered = fs::read(&note_path).expect("encrypted fixture should be readable");
        let wrapped_key_start = tampered
            .windows(b"wrapped-key=".len())
            .position(|window| window == b"wrapped-key=")
            .map(|index| index + b"wrapped-key=".len())
            .expect("encrypted fixture should contain a wrapped key");
        tampered[wrapped_key_start] = b'=';
        fs::write(&note_path, tampered).expect("tampered fixture should be written");

        let completion = scan_workspace(&workspace, &catalog).expect("damaged scan should finish");
        let metadata = catalog
            .note_editor_metadata(note_id)
            .expect("existing metadata should query")
            .expect("existing encrypted note should be preserved");

        assert_eq!(completion.failures.len(), 1);
        assert_eq!(completion.indexed_files, 0);
        assert_eq!(metadata.encryption, NoteEncryption::Encrypted);
        assert!(catalog.active_note(note_id).expect("note should query").is_some());
    }

    #[test]
    fn replacing_plain_markdown_with_encrypted_content_clears_the_old_fts_body() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("Replaced.md");
        let old_marker = "old-plaintext-marker";
        let encrypted_marker = "new-encrypted-marker";
        fs::write(&note_path, format!("# Before\n\n{old_marker}"))
            .expect("plain fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("plain scan should complete");
        assert_eq!(
            catalog
                .search_active_notes(old_marker, 10)
                .expect("plain body should be searchable")
                .len(),
            1
        );

        fs::write(&note_path, encrypted_markdown(encrypted_marker))
            .expect("encrypted replacement should be written");
        scan_workspace(&workspace, &catalog).expect("encrypted replacement scan should complete");
        let note_id = catalog.active_notes().expect("replacement note should load")[0].note_id;

        assert_eq!(
            catalog
                .note_editor_metadata(note_id)
                .expect("replacement metadata should query")
                .expect("replacement metadata should exist")
                .encryption,
            NoteEncryption::Encrypted
        );
        for marker in [old_marker, encrypted_marker] {
            assert!(
                catalog
                    .search_active_notes(marker, 10)
                    .expect("encrypted body search should complete")
                    .is_empty()
            );
        }
    }

    #[test]
    fn rescanning_changed_markdown_preserves_the_existing_notora_title() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("note.md");
        fs::write(&note_path, "# Initial H1\n\nBody").expect("fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let note_id = catalog.active_notes().expect("note should load")[0].note_id;
        catalog
            .update_note_title(note_id, "Independent Notora Title")
            .expect("metadata title should update");

        fs::write(&note_path, "# Changed H1\n\nChanged body")
            .expect("changed fixture should be written");
        scan_workspace(&workspace, &catalog).expect("changed scan should complete");

        assert_eq!(
            catalog
                .active_note(note_id)
                .expect("note lookup should succeed")
                .expect("note should remain active")
                .title,
            "Independent Notora Title"
        );
        let indexed_title: String = catalog
            .connection()
            .query_row(
                "SELECT title FROM note_search WHERE note_id = ?1",
                [note_id.to_string()],
                |row| row.get(0),
            )
            .expect("search title should remain indexed");
        assert_eq!(indexed_title, "Independent Notora Title");
    }

    #[test]
    fn missing_note_requires_two_complete_scans_before_catalog_removal() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("temporarily-missing.md");
        fs::write(&note_path, "# Keep me\n\nrecoverable #temporary")
            .expect("markdown fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let note_id = catalog.active_notes().expect("initial notes should load").remove(0).note_id;
        let formal_tag = catalog.create_tag("正式标签").expect("formal tag should create");
        catalog.attach_tag(note_id, formal_tag.tag_id).expect("formal tag should attach");
        assert_eq!(
            catalog.navigation_tree().expect("initial tag navigation should load").tags.len(),
            1
        );

        fs::remove_file(&note_path).expect("fixture note should become temporarily missing");
        scan_workspace(&workspace, &catalog).expect("first missing scan should complete");
        assert!(catalog.active_note(note_id).expect("note lookup should succeed").is_some());

        fs::write(&note_path, "# Keep me\n\nrecoverable #temporary")
            .expect("temporarily missing note should return");
        scan_workspace(&workspace, &catalog).expect("reappearance scan should complete");
        fs::remove_file(&note_path).expect("fixture note should become missing again");
        scan_workspace(&workspace, &catalog).expect("new first missing scan should complete");
        assert!(catalog.active_note(note_id).expect("note lookup should succeed").is_some());

        scan_workspace(&workspace, &catalog).expect("confirming missing scan should complete");
        assert!(catalog.active_note(note_id).expect("note lookup should succeed").is_none());
        assert!(
            catalog.navigation_tree().expect("orphaned tag navigation should load").tags.is_empty()
        );
    }

    #[test]
    fn targeted_scan_reads_and_indexes_only_the_changed_batch_paths() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let changed_path = directory.path().join("changed.md");
        let untouched_path = directory.path().join("untouched.md");
        fs::write(&changed_path, "# Before\n\nold body")
            .expect("changed fixture should be written");
        fs::write(&untouched_path, "# Untouched\n\nstable body")
            .expect("untouched fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");

        fs::write(&changed_path, "# After\n\nnew body").expect("changed fixture should be updated");
        fs::write(&untouched_path, [0xff, 0xfe])
            .expect("untouched fixture should become unreadable as UTF-8");
        let completion =
            scan_workspace_paths(&workspace, &catalog, &[std::path::PathBuf::from("changed.md")])
                .expect("targeted scan should ignore paths outside its batch");

        assert_eq!(completion.discovered_files, 1);
        assert_eq!(completion.indexed_files, 1);
        assert!(completion.failures.is_empty());
        assert_eq!(
            catalog
                .search_active_notes("new body", 10)
                .expect("changed body should be searchable")
                .len(),
            1
        );
        assert!(catalog.active_notes().expect("catalog notes should remain").len() == 2);
    }

    #[test]
    fn finder_rename_preserves_note_id_and_uses_the_new_stem_as_literal_title() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let old_path = directory.path().join("old.md");
        let new_path = directory.path().join("New (2).md");
        fs::write(&old_path, "same body").expect("rename fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let original = catalog.active_notes().expect("original note should load").remove(0);

        fs::rename(&old_path, &new_path).expect("Finder rename fixture should move");
        scan_workspace_paths(&workspace, &catalog, &["old.md".into(), "New (2).md".into()])
            .expect("rename scan should complete");

        let renamed = catalog.active_notes().expect("renamed note should load").remove(0);
        assert_eq!(renamed.note_id, original.note_id);
        assert_eq!(renamed.relative_path, std::path::PathBuf::from("New (2).md"));
        assert_eq!(renamed.title, "New (2)");
        assert_eq!(
            catalog
                .note_file_name_metadata(renamed.note_id)
                .expect("renamed metadata should query")
                .expect("renamed metadata should exist")
                .title_revision,
            1
        );
    }

    #[test]
    fn changed_document_hashtags_do_not_replace_formal_navigation_tags() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("tagged.md");
        fs::write(&note_path, "# 标题\n\n正文 #计划 #共享")
            .expect("tagged fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let formal_tag = catalog.create_tag("正式/标签").expect("formal tag should create");
        catalog
            .attach_tag(note_id_for(&catalog), formal_tag.tag_id)
            .expect("formal tag should attach");
        assert_eq!(
            catalog
                .navigation_tree()
                .expect("initial tag navigation should load")
                .tags
                .into_iter()
                .map(|tag| tag.display_name)
                .collect::<Vec<_>>(),
            vec!["正式/标签".to_owned()]
        );
        assert_eq!(
            catalog
                .search_active_notes("正式/标签", 10)
                .expect("formal tag should participate in search")
                .len(),
            1
        );

        fs::write(&note_path, "# 标题\n\n正文 #归档").expect("tagged fixture should be updated");
        scan_workspace_paths(&workspace, &catalog, &[std::path::PathBuf::from("tagged.md")])
            .expect("targeted tag scan should complete");

        assert_eq!(
            catalog
                .navigation_tree()
                .expect("replacement tag navigation should load")
                .tags
                .into_iter()
                .map(|tag| tag.display_name)
                .collect::<Vec<_>>(),
            vec!["正式/标签".to_owned()]
        );
        assert!(
            catalog
                .search_active_notes("正式/标签", 10)
                .expect("formal tag search should remain")
                .len()
                == 1
        );
        assert_eq!(
            catalog
                .search_active_notes("归档", 10)
                .expect("body hashtag should remain searchable")
                .len(),
            1
        );
    }

    fn note_id_for(catalog: &Catalog) -> crate::NoteId {
        catalog
            .active_notes()
            .expect("active notes should load")
            .into_iter()
            .next()
            .expect("fixture note should exist")
            .note_id
    }

    #[test]
    fn formal_tags_survive_a_full_rescan_after_body_hashtag_changes() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let note_path = directory.path().join("tagged.md");
        fs::write(&note_path, "# 标题\n\n正文 #正文标签")
            .expect("tagged fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        scan_workspace(&workspace, &catalog).expect("initial scan should complete");
        let note_id = note_id_for(&catalog);
        let formal_tag = catalog.create_tag("产品/Notora").expect("formal tag should create");
        catalog.attach_tag(note_id, formal_tag.tag_id).expect("formal tag should attach");

        fs::write(&note_path, "# 标题\n\n正文 #另一个正文标签")
            .expect("tagged fixture should be updated");
        scan_workspace(&workspace, &catalog).expect("rescan should complete");

        assert_eq!(
            catalog
                .tags_for_note(note_id)
                .expect("formal tags should query")
                .into_iter()
                .map(|tag| tag.display_name)
                .collect::<Vec<_>>(),
            vec!["产品/Notora".to_owned()]
        );
        assert!(
            catalog
                .search_active_notes("另一个正文标签", 10)
                .expect("body hashtag should remain searchable")
                .iter()
                .any(|search_match| search_match.note_id == note_id)
        );
        assert!(
            catalog
                .search_active_notes("产品/Notora", 10)
                .expect("formal tag should remain searchable")
                .iter()
                .any(|search_match| search_match.note_id == note_id)
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
        scan_workspace(&workspace, &catalog).expect("first deletion scan should complete");
        scan_workspace(&workspace, &catalog).expect("confirming deletion scan should complete");

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

    fn encrypted_markdown(plaintext: &str) -> Vec<u8> {
        textora_encryption::create_encrypted_markdown(&encryption_password(), plaintext.as_bytes())
            .expect("encrypted fixture should be created")
            .into_parts()
            .0
    }

    fn encryption_password() -> textora_encryption::EncryptionPassword {
        textora_encryption::EncryptionPassword::new("test-password".to_owned())
            .expect("test password should satisfy policy")
    }
}
