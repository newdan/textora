//! 工作区目录树扫描；目录生命周期只以文件系统为真源。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{WORKSPACE_METADATA_DIRECTORY_NAME, Workspace};

const MACOS_FINDER_METADATA_FILE_NAME: &str = ".DS_Store";
const MACOS_RESOURCE_FORK_PREFIX: &str = "._";

#[derive(Debug)]
pub struct WorkspaceDirectoryScanError {
    pub path: PathBuf,
    pub operation: &'static str,
    pub source: std::io::Error,
}

impl std::fmt::Display for WorkspaceDirectoryScanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "无法{}工作区目录 {}：{}",
            self.operation,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for WorkspaceDirectoryScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// 扫描所有真实目录，包括空目录；返回稳定排序的工作区相对路径。
///
/// 调用方必须在后台 worker 中调用。symlink 与保留 metadata 不会进入结果，也不会被递归。
pub fn scan_workspace_directories(
    workspace: &Workspace,
) -> Result<Vec<PathBuf>, WorkspaceDirectoryScanError> {
    let mut directories = BTreeSet::new();
    scan_directory(workspace.root(), workspace.root(), &mut directories)?;
    Ok(directories.into_iter().collect())
}

fn scan_directory(
    workspace_root: &Path,
    directory: &Path,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<(), WorkspaceDirectoryScanError> {
    let entries =
        fs::read_dir(directory).map_err(|source| scan_error(directory, "读取", source))?;
    let mut entries = entries
        .map(|entry| entry.map_err(|source| scan_error(directory, "遍历", source)))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if should_ignore(&path) {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| scan_error(&path, "检查", source))?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let relative_path = path.strip_prefix(workspace_root).map_err(|_| {
            scan_error(
                &path,
                "解析",
                std::io::Error::new(std::io::ErrorKind::InvalidData, "目录不在工作区根目录内"),
            )
        })?;
        directories.insert(relative_path.to_path_buf());
        scan_directory(workspace_root, &path, directories)?;
    }
    Ok(())
}

fn should_ignore(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_none_or(|file_name| {
        file_name == WORKSPACE_METADATA_DIRECTORY_NAME
            || file_name == MACOS_FINDER_METADATA_FILE_NAME
            || file_name.starts_with(MACOS_RESOURCE_FORK_PREFIX)
    })
}

fn scan_error(
    path: &Path,
    operation: &'static str,
    source: std::io::Error,
) -> WorkspaceDirectoryScanError {
    WorkspaceDirectoryScanError { path: path.to_path_buf(), operation, source }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::scan_workspace_directories;
    use crate::Workspace;

    #[test]
    fn scan_includes_empty_and_nested_directories_in_stable_order() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        fs::create_dir(temporary_directory.path().join("z-empty"))
            .expect("empty directory should be created");
        fs::create_dir(temporary_directory.path().join("docs"))
            .expect("parent directory should be created");
        fs::create_dir(temporary_directory.path().join("docs/plans"))
            .expect("nested empty directory should be created");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        assert_eq!(
            scan_workspace_directories(&workspace).expect("directory scan should succeed"),
            vec![
                std::path::PathBuf::from("docs"),
                std::path::PathBuf::from("docs/plans"),
                std::path::PathBuf::from("z-empty"),
            ]
        );
    }

    #[test]
    fn scan_ignores_workspace_metadata_and_macos_artifacts() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        fs::create_dir(temporary_directory.path().join(".DS_Store"))
            .expect("finder artifact directory should be created");
        fs::create_dir(temporary_directory.path().join("._resource"))
            .expect("resource fork directory should be created");
        fs::create_dir(temporary_directory.path().join("notes"))
            .expect("ordinary directory should be created");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        assert_eq!(
            scan_workspace_directories(&workspace).expect("directory scan should succeed"),
            vec![std::path::PathBuf::from("notes")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        let outside_directory = tempfile::tempdir().expect("outside directory should exist");
        fs::create_dir(outside_directory.path().join("escaped"))
            .expect("outside child should be created");
        symlink(outside_directory.path(), temporary_directory.path().join("linked"))
            .expect("directory symlink should be created");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        assert!(
            scan_workspace_directories(&workspace)
                .expect("directory scan should succeed")
                .is_empty()
        );
    }
}
