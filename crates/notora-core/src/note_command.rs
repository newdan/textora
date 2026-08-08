//! 通过可恢复的领域命令操作工作区笔记文件。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::{
    Catalog, CatalogError, CatalogNote, DocumentKind, NoteEncryption, NoteId, Workspace,
    WorkspaceError, parse_note_text_summary,
};

const CATALOG_NOTE_TITLE_PREFIX: &str = "未命名";
const MAXIMUM_AUTOMATIC_NOTE_SUFFIX: u32 = 1_000_000;

/// 已显式确定文档类型、位置与持久化属性的新建请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredCreateNoteRequest {
    pub kind: DocumentKind,
    pub target_directory: Option<PathBuf>,
    pub encryption: NoteEncryption,
}

/// 重命名笔记时传入的单个新文件名；不能包含目录成分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameNoteRequest {
    pub note_id: NoteId,
    pub new_file_name: PathBuf,
}

/// 移动笔记时传入的工作区内目标目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveNoteRequest {
    pub note_id: NoteId,
    pub target_directory: PathBuf,
}

/// 所有会变更工作区文件的类型化命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteCommand {
    CreateConfigured(ConfiguredCreateNoteRequest),
    Rename(RenameNoteRequest),
    Move(MoveNoteRequest),
}

/// 成功执行文件命令后的稳定笔记状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteCommandResult {
    pub note: CatalogNote,
    pub previous_relative_path: Option<PathBuf>,
}

/// 兼容仅含新建命令时的公开返回类型；后续命令共用相同结果结构。
pub type CreateNoteResult = NoteCommandResult;

/// 文件与 catalog 不能形成跨系统原子事务时的明确失败状态。
#[derive(Debug)]
pub enum NoteCommandError {
    Catalog {
        source: CatalogError,
    },
    Workspace(WorkspaceError),
    TargetDirectoryMissing {
        path: PathBuf,
    },
    TargetDirectoryNotDirectory {
        path: PathBuf,
    },
    FileWrite {
        path: PathBuf,
        source: std::io::Error,
    },
    FileMetadata {
        path: PathBuf,
        source: std::io::Error,
    },
    CatalogAfterFileWrite {
        relative_path: PathBuf,
        source: CatalogError,
    },
    NoteNotFound {
        note_id: NoteId,
    },
    InvalidFileName {
        path: PathBuf,
    },
    DocumentKindChange {
        path: PathBuf,
    },
    TargetAlreadyExists {
        path: PathBuf,
    },
    FileMove {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    CatalogAfterFileMove {
        from_relative_path: PathBuf,
        to_relative_path: PathBuf,
        source: Box<CatalogError>,
    },
    AutomaticNameExhausted {
        directory: PathBuf,
    },
    EncryptionUnavailable,
}

impl std::fmt::Display for NoteCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog { source } => {
                write!(formatter, "catalog command lookup failed: {source}")
            }
            Self::Workspace(source) => write!(formatter, "invalid note workspace target: {source}"),
            Self::TargetDirectoryMissing { path } => {
                write!(formatter, "note target directory does not exist: {}", path.display())
            }
            Self::TargetDirectoryNotDirectory { path } => {
                write!(formatter, "note target is not a directory: {}", path.display())
            }
            Self::FileWrite { path, source } => {
                write!(formatter, "could not create note file {}: {source}", path.display())
            }
            Self::FileMetadata { path, source } => {
                write!(
                    formatter,
                    "could not read created note metadata {}: {source}",
                    path.display()
                )
            }
            Self::CatalogAfterFileWrite { relative_path, source } => write!(
                formatter,
                "note file {} was created but catalog indexing failed; reconciliation can recover it: {source}",
                relative_path.display()
            ),
            Self::NoteNotFound { note_id } => {
                write!(formatter, "active note does not exist: {note_id}")
            }
            Self::InvalidFileName { path } => {
                write!(formatter, "note rename requires one plain file name: {}", path.display())
            }
            Self::DocumentKindChange { path } => write!(
                formatter,
                "renaming a note cannot change its document kind: {}",
                path.display()
            ),
            Self::TargetAlreadyExists { path } => {
                write!(formatter, "note destination already exists: {}", path.display())
            }
            Self::FileMove { from, to, source } => write!(
                formatter,
                "could not move note file from {} to {}: {source}",
                from.display(),
                to.display()
            ),
            Self::CatalogAfterFileMove { from_relative_path, to_relative_path, source } => write!(
                formatter,
                "note file moved from {} to {} but catalog update failed; reconciliation can recover it: {source}",
                from_relative_path.display(),
                to_relative_path.display()
            ),
            Self::AutomaticNameExhausted { directory } => {
                write!(formatter, "no automatic note name is available in {}", directory.display())
            }
            Self::EncryptionUnavailable => {
                write!(
                    formatter,
                    "encrypted note creation is unavailable before the encryption engine is installed"
                )
            }
        }
    }
}

impl std::error::Error for NoteCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog { source } => Some(source),
            Self::Workspace(source) => Some(source),
            Self::FileWrite { source, .. }
            | Self::FileMetadata { source, .. }
            | Self::FileMove { source, .. } => Some(source),
            Self::CatalogAfterFileWrite { source, .. } => Some(source),
            Self::CatalogAfterFileMove { source, .. } => Some(source.as_ref()),
            Self::TargetDirectoryMissing { .. }
            | Self::TargetDirectoryNotDirectory { .. }
            | Self::NoteNotFound { .. }
            | Self::InvalidFileName { .. }
            | Self::DocumentKindChange { .. }
            | Self::TargetAlreadyExists { .. }
            | Self::AutomaticNameExhausted { .. }
            | Self::EncryptionUnavailable => None,
        }
    }
}

/// 执行单个领域命令；调用方负责将它放到产品的 I/O effect 边界之后。
pub fn execute_note_command(
    workspace: &Workspace,
    catalog: &Catalog,
    command: NoteCommand,
) -> Result<NoteCommandResult, NoteCommandError> {
    match command {
        NoteCommand::CreateConfigured(request) => {
            create_configured_note(workspace, catalog, request)
        }
        NoteCommand::Rename(request) => rename_note(workspace, catalog, request),
        NoteCommand::Move(request) => move_note(workspace, catalog, request),
    }
}

fn create_configured_note(
    workspace: &Workspace,
    catalog: &Catalog,
    request: ConfiguredCreateNoteRequest,
) -> Result<NoteCommandResult, NoteCommandError> {
    if request.encryption != NoteEncryption::Unencrypted {
        return Err(NoteCommandError::EncryptionUnavailable);
    }
    let target_directory =
        resolve_target_directory(workspace, request.target_directory.as_deref())?;
    let initial_contents = initial_contents(request.kind);
    for suffix in 1..=MAXIMUM_AUTOMATIC_NOTE_SUFFIX {
        let file_name = automatic_file_name(request.kind, suffix);
        let relative_path = target_directory.relative_path.join(&file_name);
        let absolute_path =
            workspace.resolve_relative_path(&relative_path).map_err(NoteCommandError::Workspace)?;
        match create_note_file(&absolute_path, initial_contents) {
            Ok(()) => {
                let note = created_catalog_note(
                    request.kind,
                    relative_path.clone(),
                    &target_directory.title_prefix,
                    suffix,
                    initial_contents,
                    &absolute_path,
                )?;
                catalog
                    .create_active_note(
                        &note,
                        request.encryption,
                        match request.kind {
                            DocumentKind::Markdown | DocumentKind::Mindmap => {
                                crate::TitleInitialization::AwaitingFirstCommit
                            }
                            DocumentKind::Text => crate::TitleInitialization::Independent,
                        },
                    )
                    .map_err(|source| NoteCommandError::CatalogAfterFileWrite {
                        relative_path,
                        source,
                    })?;
                return Ok(NoteCommandResult { note, previous_relative_path: None });
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(NoteCommandError::FileWrite { path: absolute_path, source });
            }
        }
    }
    Err(NoteCommandError::AutomaticNameExhausted { directory: target_directory.absolute_path })
}

fn rename_note(
    workspace: &Workspace,
    catalog: &Catalog,
    request: RenameNoteRequest,
) -> Result<NoteCommandResult, NoteCommandError> {
    let note = active_note(catalog, request.note_id)?;
    let file_name = validate_new_file_name(&request.new_file_name, note.kind)?;
    let parent_directory = note
        .relative_path
        .parent()
        .ok_or_else(|| NoteCommandError::InvalidFileName { path: note.relative_path.clone() })?;
    let target_relative_path = parent_directory.join(file_name);
    relocate_note(workspace, catalog, note, target_relative_path)
}

fn move_note(
    workspace: &Workspace,
    catalog: &Catalog,
    request: MoveNoteRequest,
) -> Result<NoteCommandResult, NoteCommandError> {
    let note = active_note(catalog, request.note_id)?;
    let target_directory = resolve_target_directory(workspace, Some(&request.target_directory))?;
    let file_name = note
        .relative_path
        .file_name()
        .ok_or_else(|| NoteCommandError::InvalidFileName { path: note.relative_path.clone() })?;
    let target_relative_path = target_directory.relative_path.join(file_name);
    relocate_note(workspace, catalog, note, target_relative_path)
}

fn active_note(catalog: &Catalog, note_id: NoteId) -> Result<CatalogNote, NoteCommandError> {
    catalog
        .active_note(note_id)
        .map_err(|source| NoteCommandError::Catalog { source })?
        .ok_or(NoteCommandError::NoteNotFound { note_id })
}

fn validate_new_file_name(
    file_name: &Path,
    original_kind: DocumentKind,
) -> Result<&std::ffi::OsStr, NoteCommandError> {
    let mut components = file_name.components();
    let Some(Component::Normal(component)) = components.next() else {
        return Err(NoteCommandError::InvalidFileName { path: file_name.to_path_buf() });
    };
    if components.next().is_some() {
        return Err(NoteCommandError::InvalidFileName { path: file_name.to_path_buf() });
    }
    if DocumentKind::from_path(file_name) != Some(original_kind) {
        return Err(NoteCommandError::DocumentKindChange { path: file_name.to_path_buf() });
    }
    Ok(component)
}

fn relocate_note(
    workspace: &Workspace,
    catalog: &Catalog,
    mut note: CatalogNote,
    target_relative_path: PathBuf,
) -> Result<NoteCommandResult, NoteCommandError> {
    let source_relative_path = note.relative_path.clone();
    if source_relative_path == target_relative_path {
        return Ok(NoteCommandResult { note, previous_relative_path: None });
    }
    let source_path = workspace
        .resolve_relative_path(&source_relative_path)
        .map_err(NoteCommandError::Workspace)?;
    let target_path = workspace
        .resolve_relative_path(&target_relative_path)
        .map_err(NoteCommandError::Workspace)?;
    ensure_target_is_available(&target_path)?;
    fs::rename(&source_path, &target_path).map_err(|source| NoteCommandError::FileMove {
        from: source_path,
        to: target_path,
        source,
    })?;
    catalog.update_active_note_path(note.note_id, &target_relative_path).map_err(|source| {
        NoteCommandError::CatalogAfterFileMove {
            from_relative_path: source_relative_path.clone(),
            to_relative_path: target_relative_path.clone(),
            source: Box::new(source),
        }
    })?;
    note.relative_path = target_relative_path;
    Ok(NoteCommandResult { note, previous_relative_path: Some(source_relative_path) })
}

fn ensure_target_is_available(target_path: &Path) -> Result<(), NoteCommandError> {
    match fs::symlink_metadata(target_path) {
        Ok(_) => Err(NoteCommandError::TargetAlreadyExists { path: target_path.to_path_buf() }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => {
            Err(NoteCommandError::FileMetadata { path: target_path.to_path_buf(), source })
        }
    }
}

struct TargetDirectory {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    title_prefix: String,
}

fn resolve_target_directory(
    workspace: &Workspace,
    requested_directory: Option<&Path>,
) -> Result<TargetDirectory, NoteCommandError> {
    let relative_path = requested_directory.map_or_else(PathBuf::new, Path::to_path_buf);
    let absolute_path = match requested_directory {
        Some(directory) => {
            workspace.resolve_relative_path(directory).map_err(NoteCommandError::Workspace)?
        }
        None => workspace.root().to_path_buf(),
    };
    let metadata = fs::metadata(&absolute_path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => {
            NoteCommandError::TargetDirectoryMissing { path: absolute_path.clone() }
        }
        _ => NoteCommandError::FileMetadata { path: absolute_path.clone(), source },
    })?;
    if !metadata.is_dir() {
        return Err(NoteCommandError::TargetDirectoryNotDirectory { path: absolute_path });
    }
    Ok(TargetDirectory {
        relative_path,
        absolute_path,
        title_prefix: CATALOG_NOTE_TITLE_PREFIX.to_owned(),
    })
}

fn automatic_file_name(kind: DocumentKind, suffix: u32) -> String {
    format!("{CATALOG_NOTE_TITLE_PREFIX} {suffix}{}", file_extension(kind))
}

fn file_extension(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text => ".txt",
        DocumentKind::Markdown => ".md",
        DocumentKind::Mindmap => ".mmap.md",
    }
}

fn initial_contents(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text | DocumentKind::Markdown => "",
        DocumentKind::Mindmap => "#",
    }
}

fn create_note_file(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

fn created_catalog_note(
    kind: DocumentKind,
    relative_path: PathBuf,
    title_prefix: &str,
    suffix: u32,
    contents: &str,
    absolute_path: &Path,
) -> Result<CatalogNote, NoteCommandError> {
    let metadata = fs::metadata(absolute_path).map_err(|source| {
        NoteCommandError::FileMetadata { path: absolute_path.to_path_buf(), source }
    })?;
    let title = format!("{title_prefix} {suffix}");
    let summary = parse_note_text_summary(kind, &title, contents);
    Ok(CatalogNote {
        note_id: NoteId::generate(),
        relative_path,
        kind,
        title: summary.title,
        excerpt: summary.excerpt,
        modified_at: metadata.modified().map_err(|source| NoteCommandError::FileMetadata {
            path: absolute_path.to_path_buf(),
            source,
        })?,
        file_size: metadata.len(),
        content_hash: blake3::hash(contents.as_bytes()).as_bytes().to_vec(),
        starred: false,
    })
}

#[cfg(test)]
mod create {
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use crate::domain::NoteEncryption;
    use crate::{Catalog, DocumentKind, Workspace};

    use super::{ConfiguredCreateNoteRequest, NoteCommand, execute_note_command};

    #[test]
    fn create_places_each_kind_in_the_requested_workspace_directory() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(directory.path().join("nested")).expect("nested fixture should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        let markdown = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                encryption: NoteEncryption::Unencrypted,
            }),
        )
        .expect("markdown note should be created");
        let mindmap = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Mindmap,
                target_directory: Some("nested".into()),
                encryption: NoteEncryption::Unencrypted,
            }),
        )
        .expect("mindmap note should be created");

        assert_eq!(markdown.note.relative_path, std::path::PathBuf::from("未命名 1.md"));
        assert_eq!(mindmap.note.relative_path, std::path::PathBuf::from("nested/未命名 1.mmap.md"));
        assert_eq!(
            fs::read_to_string(directory.path().join(&mindmap.note.relative_path))
                .expect("mindmap note should be readable"),
            "#"
        );
        assert_eq!(catalog.active_notes().expect("created notes should be indexed").len(), 2);
        for note in [&markdown.note, &mindmap.note] {
            assert_eq!(
                catalog
                    .note_editor_metadata(note.note_id)
                    .expect("created metadata should query")
                    .expect("created metadata should exist")
                    .title_initialization,
                crate::TitleInitialization::AwaitingFirstCommit
            );
        }
    }

    #[test]
    fn concurrent_creation_never_overwrites_an_existing_untitled_file() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let root = workspace.root().to_path_buf();
        let barrier = Arc::new(Barrier::new(2));
        let worker_inputs = (0..2)
            .map(|_| {
                let catalog =
                    Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
                        .expect("catalog should open before concurrent command execution");
                (workspace.clone(), catalog)
            })
            .collect::<Vec<_>>();
        let workers = worker_inputs
            .into_iter()
            .map(|(workspace, catalog)| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    execute_note_command(
                        &workspace,
                        &catalog,
                        NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                            kind: DocumentKind::Markdown,
                            target_directory: None,
                            encryption: NoteEncryption::Unencrypted,
                        }),
                    )
                    .expect("concurrent note creation should choose a free name")
                })
            })
            .collect::<Vec<_>>();
        let created = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker should not panic"))
            .collect::<Vec<_>>();

        assert_ne!(created[0].note.relative_path, created[1].note.relative_path);
        assert!(root.join("未命名 1.md").is_file());
        assert!(root.join("未命名 2.md").is_file());
    }

    #[test]
    fn create_rejects_the_reserved_metadata_directory() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        assert!(
            execute_note_command(
                &workspace,
                &catalog,
                NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                    kind: DocumentKind::Markdown,
                    target_directory: Some(".notora".into()),
                    encryption: NoteEncryption::Unencrypted,
                }),
            )
            .is_err()
        );
    }

    #[test]
    fn configured_creation_rejects_encryption_until_the_real_engine_is_available() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");

        let result = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                encryption: NoteEncryption::Encrypted,
            }),
        );

        assert!(matches!(result, Err(super::NoteCommandError::EncryptionUnavailable)));
        assert!(!directory.path().join("未命名 1.md").exists());
        assert!(catalog.active_notes().expect("catalog should remain readable").is_empty());
    }
}

#[cfg(test)]
mod rename {
    use std::fs;

    use crate::domain::NoteEncryption;
    use crate::{Catalog, DocumentKind, Workspace};

    use super::{
        ConfiguredCreateNoteRequest, NoteCommand, RenameNoteRequest, execute_note_command,
    };

    #[test]
    fn rename_updates_the_catalog_path_without_changing_the_note_id() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                encryption: NoteEncryption::Unencrypted,
            }),
        )
        .expect("note fixture should be created");

        let renamed = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::Rename(RenameNoteRequest {
                note_id: created.note.note_id,
                new_file_name: "roadmap.md".into(),
            }),
        )
        .expect("rename should succeed");

        assert_eq!(renamed.note.note_id, created.note.note_id);
        assert_eq!(renamed.note.relative_path, std::path::PathBuf::from("roadmap.md"));
        assert!(!directory.path().join("未命名 1.md").exists());
        assert!(directory.path().join("roadmap.md").is_file());
        assert_eq!(
            catalog.active_notes().expect("catalog should read renamed note"),
            vec![renamed.note]
        );
    }

    #[test]
    fn rename_rejects_kind_changes_directory_escapes_and_existing_targets() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                encryption: NoteEncryption::Unencrypted,
            }),
        )
        .expect("note fixture should be created");
        fs::write(directory.path().join("occupied.md"), "do not replace")
            .expect("occupied target fixture should be written");

        for new_file_name in ["changed.txt", "../outside.md", "occupied.md"] {
            assert!(
                execute_note_command(
                    &workspace,
                    &catalog,
                    NoteCommand::Rename(RenameNoteRequest {
                        note_id: created.note.note_id,
                        new_file_name: new_file_name.into(),
                    }),
                )
                .is_err()
            );
        }
        assert!(directory.path().join("未命名 1.md").is_file());
        assert_eq!(
            fs::read_to_string(directory.path().join("occupied.md"))
                .expect("occupied target should remain readable"),
            "do not replace"
        );
    }
}

#[cfg(test)]
mod move_note {
    use std::fs;

    use crate::domain::NoteEncryption;
    use crate::{Catalog, DocumentKind, Workspace};

    use super::{ConfiguredCreateNoteRequest, MoveNoteRequest, NoteCommand, execute_note_command};

    #[test]
    fn move_keeps_the_note_id_and_updates_its_relative_path() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(directory.path().join("archive"))
            .expect("archive fixture should be created");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Mindmap,
                target_directory: None,
                encryption: NoteEncryption::Unencrypted,
            }),
        )
        .expect("note fixture should be created");

        let moved = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::Move(MoveNoteRequest {
                note_id: created.note.note_id,
                target_directory: "archive".into(),
            }),
        )
        .expect("move should succeed");

        assert_eq!(moved.note.note_id, created.note.note_id);
        assert_eq!(moved.note.relative_path, std::path::PathBuf::from("archive/未命名 1.mmap.md"));
        assert!(directory.path().join(&moved.note.relative_path).is_file());
    }

    #[test]
    fn move_rejects_reserved_or_missing_destinations_without_moving_the_source() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::create_dir(directory.path().join("occupied"))
            .expect("occupied directory fixture should be created");
        fs::write(directory.path().join("occupied/未命名 1.txt"), "do not replace")
            .expect("occupied note fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
        let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
            .expect("catalog should initialize");
        let created = execute_note_command(
            &workspace,
            &catalog,
            NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
                kind: DocumentKind::Text,
                target_directory: None,
                encryption: NoteEncryption::Unencrypted,
            }),
        )
        .expect("note fixture should be created");

        for target_directory in [".notora", "does-not-exist", "occupied"] {
            assert!(
                execute_note_command(
                    &workspace,
                    &catalog,
                    NoteCommand::Move(MoveNoteRequest {
                        note_id: created.note.note_id,
                        target_directory: target_directory.into(),
                    }),
                )
                .is_err()
            );
        }
        assert!(directory.path().join("未命名 1.txt").is_file());
        assert_eq!(
            fs::read_to_string(directory.path().join("occupied/未命名 1.txt"))
                .expect("occupied target should remain readable"),
            "do not replace"
        );
    }
}
