use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::WorkspaceId;

pub const WORKSPACE_METADATA_DIRECTORY_NAME: &str = ".notora";
pub const WORKSPACE_MANIFEST_FILE_NAME: &str = "workspace.toml";
pub const WORKSPACE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const NOTE_RELOCATION_TEMPORARY_FILE_PREFIX: &str = ".notora-relocation-";

static NOTE_RELOCATION_TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 以不覆盖目标的语义移动普通笔记文件。
pub fn move_file_no_replace(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    if requires_case_only_two_hop(source, target) {
        return move_file_case_only(source, target);
    }
    platform_move_file_no_replace(source, target)
}

fn requires_case_only_two_hop(source: &Path, target: &Path) -> bool {
    source != target
        && source.parent() == target.parent()
        && source.file_name().zip(target.file_name()).is_some_and(|(source_name, target_name)| {
            crate::file_name_collision_key(&source_name.to_string_lossy())
                == crate::file_name_collision_key(&target_name.to_string_lossy())
        })
}

fn move_file_case_only(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    let parent = source.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source has no parent directory")
    })?;
    loop {
        let sequence = NOTE_RELOCATION_TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            "{NOTE_RELOCATION_TEMPORARY_FILE_PREFIX}{}-{sequence}.tmp",
            std::process::id()
        ));
        match platform_move_file_no_replace(source, &temporary_path) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
            Ok(()) => {
                if let Err(target_error) = platform_move_file_no_replace(&temporary_path, target) {
                    return match platform_move_file_no_replace(&temporary_path, source) {
                        Ok(()) => Err(target_error),
                        Err(rollback_error) => Err(rollback_error),
                    };
                }
                return Ok(());
            }
        }
    }
}

#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "redox"))]
fn platform_move_file_no_replace(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        target,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)
}

#[cfg(target_os = "windows")]
fn platform_move_file_no_replace(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    fs::rename(source, target)
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "redox",
    target_os = "windows"
)))]
fn platform_move_file_no_replace(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    fs::hard_link(source, target)?;
    fs::remove_file(source)
}

static MANIFEST_TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 磁盘上保存的最小工作区身份信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceManifest {
    pub schema_version: u32,
    pub workspace_id: WorkspaceId,
}

impl WorkspaceManifest {
    fn create() -> Self {
        Self {
            schema_version: WORKSPACE_MANIFEST_SCHEMA_VERSION,
            workspace_id: WorkspaceId::generate(),
        }
    }
}

/// 产品层可持久化的工作区定位信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDescriptor {
    pub root: PathBuf,
    pub workspace_id: WorkspaceId,
}

/// 已验证的工作区根和 metadata 目录。
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    metadata_directory: PathBuf,
    manifest: WorkspaceManifest,
}

impl Workspace {
    /// 打开现有工作区，或为普通目录创建最小 metadata。
    pub fn open_or_initialize(root: &Path) -> Result<Self, WorkspaceError> {
        let root = canonicalize_directory(root)?;
        let metadata_directory = root.join(WORKSPACE_METADATA_DIRECTORY_NAME);
        fs::create_dir_all(&metadata_directory)
            .map_err(|source| WorkspaceError::io(&metadata_directory, source))?;

        let manifest_path = metadata_directory.join(WORKSPACE_MANIFEST_FILE_NAME);
        let manifest = if manifest_path.exists() {
            read_manifest(&manifest_path)?
        } else {
            let manifest = WorkspaceManifest::create();
            write_manifest_atomically(&manifest_path, &manifest)?;
            manifest
        };

        validate_schema_version(manifest.schema_version)?;

        Ok(Self { root, metadata_directory, manifest })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn metadata_directory(&self) -> &Path {
        &self.metadata_directory
    }

    pub fn manifest(&self) -> &WorkspaceManifest {
        &self.manifest
    }

    pub fn descriptor(&self) -> WorkspaceDescriptor {
        WorkspaceDescriptor { root: self.root.clone(), workspace_id: self.manifest.workspace_id }
    }

    /// 解析工作区内可读或可写的相对路径，并拒绝 symlink 逃逸和保留目录。
    pub fn resolve_relative_path(&self, relative_path: &Path) -> Result<PathBuf, WorkspaceError> {
        validate_relative_path(relative_path)?;

        let lexical_path = self.root.join(relative_path);
        let existing_ancestor = find_existing_ancestor(&lexical_path)?;
        let canonical_ancestor = fs::canonicalize(&existing_ancestor)
            .map_err(|source| WorkspaceError::io(&existing_ancestor, source))?;
        if !canonical_ancestor.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideWorkspace { path: lexical_path });
        }

        let unresolved_suffix = lexical_path
            .strip_prefix(&existing_ancestor)
            .map_err(|_| WorkspaceError::OutsideWorkspace { path: lexical_path.clone() })?;
        let resolved_path = if unresolved_suffix.as_os_str().is_empty() {
            canonical_ancestor
        } else {
            canonical_ancestor.join(unresolved_suffix)
        };
        if resolved_path.starts_with(&self.metadata_directory) {
            return Err(WorkspaceError::ReservedMetadataPath { path: resolved_path });
        }

        Ok(resolved_path)
    }
}

#[derive(Debug)]
pub enum WorkspaceError {
    NotDirectory { path: PathBuf },
    InvalidRelativePath { path: PathBuf },
    OutsideWorkspace { path: PathBuf },
    ReservedMetadataPath { path: PathBuf },
    UnsupportedSchema { found: u32 },
    Io { path: PathBuf, source: std::io::Error },
    ManifestParse { path: PathBuf, source: toml::de::Error },
    ManifestSerialize { source: toml::ser::Error },
}

impl WorkspaceError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io { path: path.to_path_buf(), source }
    }
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotDirectory { path } => {
                write!(formatter, "workspace root is not a directory: {}", path.display())
            }
            Self::InvalidRelativePath { path } => {
                write!(formatter, "invalid workspace-relative path: {}", path.display())
            }
            Self::OutsideWorkspace { path } => {
                write!(formatter, "path escapes workspace root: {}", path.display())
            }
            Self::ReservedMetadataPath { path } => write!(
                formatter,
                "path is inside the reserved metadata directory: {}",
                path.display()
            ),
            Self::UnsupportedSchema { found } => {
                write!(formatter, "unsupported workspace manifest schema version: {found}")
            }
            Self::Io { path, source } => {
                write!(formatter, "workspace I/O failed for {}: {source}", path.display())
            }
            Self::ManifestParse { path, source } => {
                write!(formatter, "workspace manifest is invalid at {}: {source}", path.display())
            }
            Self::ManifestSerialize { source } => {
                write!(formatter, "workspace manifest serialization failed: {source}")
            }
        }
    }
}

impl std::error::Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::ManifestParse { source, .. } => Some(source),
            Self::ManifestSerialize { source } => Some(source),
            Self::NotDirectory { .. }
            | Self::InvalidRelativePath { .. }
            | Self::OutsideWorkspace { .. }
            | Self::ReservedMetadataPath { .. }
            | Self::UnsupportedSchema { .. } => None,
        }
    }
}

fn canonicalize_directory(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let metadata = fs::metadata(path).map_err(|source| WorkspaceError::io(path, source))?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::NotDirectory { path: path.to_path_buf() });
    }

    fs::canonicalize(path).map_err(|source| WorkspaceError::io(path, source))
}

fn read_manifest(path: &Path) -> Result<WorkspaceManifest, WorkspaceError> {
    let contents = fs::read_to_string(path).map_err(|source| WorkspaceError::io(path, source))?;
    toml::from_str(&contents)
        .map_err(|source| WorkspaceError::ManifestParse { path: path.to_path_buf(), source })
}

fn validate_schema_version(schema_version: u32) -> Result<(), WorkspaceError> {
    if schema_version == WORKSPACE_MANIFEST_SCHEMA_VERSION {
        return Ok(());
    }

    Err(WorkspaceError::UnsupportedSchema { found: schema_version })
}

fn validate_relative_path(path: &Path) -> Result<(), WorkspaceError> {
    if path.as_os_str().is_empty() {
        return Err(WorkspaceError::InvalidRelativePath { path: path.to_path_buf() });
    }

    let mut components = path.components();
    let Some(Component::Normal(first_component)) = components.next() else {
        return Err(WorkspaceError::InvalidRelativePath { path: path.to_path_buf() });
    };
    if first_component == OsStr::new(WORKSPACE_METADATA_DIRECTORY_NAME) {
        return Err(WorkspaceError::ReservedMetadataPath { path: path.to_path_buf() });
    }

    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(WorkspaceError::InvalidRelativePath { path: path.to_path_buf() });
    }

    Ok(())
}

fn find_existing_ancestor(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut ancestor = path;
    loop {
        if ancestor.exists() {
            return Ok(ancestor.to_path_buf());
        }

        ancestor = ancestor
            .parent()
            .ok_or_else(|| WorkspaceError::OutsideWorkspace { path: path.to_path_buf() })?;
    }
}

fn write_manifest_atomically(
    path: &Path,
    manifest: &WorkspaceManifest,
) -> Result<(), WorkspaceError> {
    let contents = toml::to_string_pretty(manifest)
        .map_err(|source| WorkspaceError::ManifestSerialize { source })?;
    let parent = path
        .parent()
        .ok_or_else(|| WorkspaceError::InvalidRelativePath { path: path.to_path_buf() })?;
    let sequence = MANIFEST_TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_path = parent.join(format!(
        ".{WORKSPACE_MANIFEST_FILE_NAME}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let mut temporary_path_guard = TemporaryManifestPath::new(&temporary_path);
    {
        let mut temporary_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|source| WorkspaceError::io(&temporary_path, source))?;
        temporary_file
            .write_all(contents.as_bytes())
            .map_err(|source| WorkspaceError::io(&temporary_path, source))?;
        temporary_file.flush().map_err(|source| WorkspaceError::io(&temporary_path, source))?;
        temporary_file.sync_all().map_err(|source| WorkspaceError::io(&temporary_path, source))?;
    }
    fs::rename(&temporary_path, path).map_err(|source| WorkspaceError::io(path, source))?;
    temporary_path_guard.keep();

    #[cfg(unix)]
    sync_directory(parent);

    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

struct TemporaryManifestPath {
    path: PathBuf,
    should_remove: bool,
}

impl TemporaryManifestPath {
    fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf(), should_remove: true }
    }

    fn keep(&mut self) {
        self.should_remove = false;
    }
}

impl Drop for TemporaryManifestPath {
    fn drop(&mut self) {
        if self.should_remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{
        WORKSPACE_MANIFEST_FILE_NAME, WORKSPACE_METADATA_DIRECTORY_NAME, Workspace, WorkspaceError,
        move_file_no_replace,
    };

    #[test]
    fn no_replace_move_never_overwrites_an_existing_target() {
        let directory = tempfile::tempdir().expect("move test directory should be created");
        let source = directory.path().join("source.md");
        let target = directory.path().join("target.md");
        fs::write(&source, "source").expect("source fixture should be written");
        fs::write(&target, "target").expect("target fixture should be written");

        assert!(move_file_no_replace(&source, &target).is_err());
        assert_eq!(fs::read_to_string(&source).expect("source should remain"), "source");
        assert_eq!(fs::read_to_string(&target).expect("target should remain"), "target");
    }

    #[test]
    fn no_replace_move_supports_case_only_file_name_changes() {
        let directory = tempfile::tempdir().expect("move test directory should be created");
        let source = directory.path().join("Plan.md");
        let target = directory.path().join("plan.md");
        fs::write(&source, "content").expect("source fixture should be written");

        move_file_no_replace(&source, &target).expect("case-only move should succeed");

        assert_eq!(fs::read_to_string(&target).expect("target should be readable"), "content");
        let file_names = fs::read_dir(directory.path())
            .expect("test directory should be readable")
            .map(|entry| {
                entry
                    .expect("directory entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(file_names, vec!["plan.md"]);
    }

    #[test]
    fn initialization_persists_a_stable_workspace_identity() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let first_workspace = Workspace::open_or_initialize(directory.path())
            .expect("empty directory should initialize as a workspace");
        let second_workspace = Workspace::open_or_initialize(directory.path())
            .expect("initialized workspace should reopen");

        assert_eq!(first_workspace.manifest(), second_workspace.manifest());
        assert!(
            directory
                .path()
                .join(WORKSPACE_METADATA_DIRECTORY_NAME)
                .join(WORKSPACE_MANIFEST_FILE_NAME)
                .is_file()
        );
    }

    #[test]
    fn relative_paths_cannot_escape_or_enter_metadata_directory() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let workspace = Workspace::open_or_initialize(directory.path())
            .expect("empty directory should initialize as a workspace");

        assert!(matches!(
            workspace.resolve_relative_path(Path::new("../outside.md")),
            Err(WorkspaceError::InvalidRelativePath { .. })
        ));
        assert!(matches!(
            workspace.resolve_relative_path(Path::new(".notora/catalog.sqlite3")),
            Err(WorkspaceError::ReservedMetadataPath { .. })
        ));
    }

    #[test]
    fn unknown_manifest_schema_is_rejected() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        let metadata_directory = directory.path().join(WORKSPACE_METADATA_DIRECTORY_NAME);
        fs::create_dir(&metadata_directory).expect("metadata directory should be created");
        fs::write(
            metadata_directory.join(WORKSPACE_MANIFEST_FILE_NAME),
            "schema_version = 2\nworkspace_id = \"5ca91d55-b9fc-484f-907c-1bef1c83a814\"\n",
        )
        .expect("future schema fixture should be written");

        assert!(matches!(
            Workspace::open_or_initialize(directory.path()),
            Err(WorkspaceError::UnsupportedSchema { found: 2 })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_descendants_cannot_escape_workspace_root() {
        use std::os::unix::fs::symlink;

        let workspace_directory =
            tempfile::tempdir().expect("workspace directory should be created");
        let outside_directory = tempfile::tempdir().expect("outside directory should be created");
        symlink(outside_directory.path(), workspace_directory.path().join("linked"))
            .expect("test symlink should be created");
        let workspace = Workspace::open_or_initialize(workspace_directory.path())
            .expect("workspace should initialize");

        assert!(matches!(
            workspace.resolve_relative_path(Path::new("linked/escape.md")),
            Err(WorkspaceError::OutsideWorkspace { .. })
        ));
    }

    #[test]
    fn resolving_an_existing_file_returns_a_file_path_without_a_trailing_separator() {
        let directory = tempfile::tempdir().expect("workspace test directory should be created");
        fs::write(directory.path().join("note.md"), "# Note")
            .expect("note fixture should be written");
        let workspace =
            Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");

        let resolved_path = workspace
            .resolve_relative_path(Path::new("note.md"))
            .expect("existing note should resolve");

        assert!(fs::metadata(resolved_path).expect("resolved note metadata should load").is_file());
    }
}
