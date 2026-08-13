//! 工作区目录的类型化后台命令。

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::{WORKSPACE_METADATA_DIRECTORY_NAME, Workspace, WorkspaceError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDirectoryCommand {
    Create { parent_relative_path: PathBuf, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDirectoryCommandResult {
    pub relative_path: PathBuf,
}

#[derive(Debug)]
pub enum WorkspaceDirectoryCommandError {
    EmptyName,
    InvalidName { name: String },
    ReservedName { name: String },
    InvalidParent(WorkspaceError),
    ParentMissing { relative_path: PathBuf },
    ParentNotDirectory { relative_path: PathBuf },
    TargetExists { relative_path: PathBuf },
    CreateDirectory { relative_path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for WorkspaceDirectoryCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("目录名称不能为空"),
            Self::InvalidName { name } => {
                write!(formatter, "目录名称必须是单个有效名称：{name}")
            }
            Self::ReservedName { name } => {
                write!(formatter, "目录名称属于系统保留名称：{name}")
            }
            Self::InvalidParent(source) => write!(formatter, "目录位置无效：{source}"),
            Self::ParentMissing { relative_path } => {
                write!(formatter, "父目录不存在：{}", display_root(relative_path))
            }
            Self::ParentNotDirectory { relative_path } => {
                write!(formatter, "目标位置不是目录：{}", display_root(relative_path))
            }
            Self::TargetExists { relative_path } => {
                write!(formatter, "目标已存在：{}", relative_path.display())
            }
            Self::CreateDirectory { relative_path, source } => {
                write!(formatter, "无法创建目录 {}：{source}", relative_path.display())
            }
        }
    }
}

impl std::error::Error for WorkspaceDirectoryCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidParent(source) => Some(source),
            Self::CreateDirectory { source, .. } => Some(source),
            Self::EmptyName
            | Self::InvalidName { .. }
            | Self::ReservedName { .. }
            | Self::ParentMissing { .. }
            | Self::ParentNotDirectory { .. }
            | Self::TargetExists { .. } => None,
        }
    }
}

/// 执行一个工作区目录命令。调用方必须位于活动工作区的后台 worker。
pub fn execute_workspace_directory_command(
    workspace: &Workspace,
    command: WorkspaceDirectoryCommand,
) -> Result<WorkspaceDirectoryCommandResult, WorkspaceDirectoryCommandError> {
    match command {
        WorkspaceDirectoryCommand::Create { parent_relative_path, name } => {
            create_directory(workspace, &parent_relative_path, &name)
        }
    }
}

fn create_directory(
    workspace: &Workspace,
    parent_relative_path: &Path,
    requested_name: &str,
) -> Result<WorkspaceDirectoryCommandResult, WorkspaceDirectoryCommandError> {
    let directory_name = validate_workspace_directory_name(requested_name)?;
    let parent_path = if parent_relative_path.as_os_str().is_empty() {
        workspace.root().to_path_buf()
    } else {
        workspace
            .resolve_relative_path(parent_relative_path)
            .map_err(WorkspaceDirectoryCommandError::InvalidParent)?
    };
    let parent_metadata = match fs::metadata(&parent_path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(WorkspaceDirectoryCommandError::ParentMissing {
                relative_path: parent_relative_path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(WorkspaceDirectoryCommandError::CreateDirectory {
                relative_path: parent_relative_path.to_path_buf(),
                source,
            });
        }
    };
    if !parent_metadata.is_dir() {
        return Err(WorkspaceDirectoryCommandError::ParentNotDirectory {
            relative_path: parent_relative_path.to_path_buf(),
        });
    }

    let relative_path = parent_relative_path.join(&directory_name);
    let target_path = parent_path.join(&directory_name);
    match fs::symlink_metadata(&target_path) {
        Ok(_) => {
            return Err(WorkspaceDirectoryCommandError::TargetExists { relative_path });
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(WorkspaceDirectoryCommandError::CreateDirectory { relative_path, source });
        }
    }

    fs::create_dir(&target_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            return WorkspaceDirectoryCommandError::TargetExists {
                relative_path: relative_path.clone(),
            };
        }
        WorkspaceDirectoryCommandError::CreateDirectory {
            relative_path: relative_path.clone(),
            source,
        }
    })?;
    Ok(WorkspaceDirectoryCommandResult { relative_path })
}

pub fn validate_workspace_directory_name(
    requested_name: &str,
) -> Result<String, WorkspaceDirectoryCommandError> {
    let name = requested_name.trim();
    if name.is_empty() {
        return Err(WorkspaceDirectoryCommandError::EmptyName);
    }
    let path = Path::new(name);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || name.chars().any(is_forbidden_name_character)
        || name.ends_with('.')
        || requested_name.chars().next_back().is_some_and(char::is_whitespace)
    {
        return Err(WorkspaceDirectoryCommandError::InvalidName { name: name.to_owned() });
    }
    if is_reserved_name(name) {
        return Err(WorkspaceDirectoryCommandError::ReservedName { name: name.to_owned() });
    }
    Ok(name.to_owned())
}

fn is_forbidden_name_character(character: char) -> bool {
    character.is_control()
        || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

fn is_reserved_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case(WORKSPACE_METADATA_DIRECTORY_NAME) {
        return true;
    }
    let device_name = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(device_name.as_str(), "." | ".." | "CON" | "PRN" | "AUX" | "NUL")
        || numbered_device_name(&device_name, "COM")
        || numbered_device_name(&device_name, "LPT")
}

fn numbered_device_name(device_name: &str, prefix: &str) -> bool {
    device_name
        .strip_prefix(prefix)
        .is_some_and(|number| matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}

fn display_root(relative_path: &Path) -> String {
    if relative_path.as_os_str().is_empty() {
        return "工作区根目录".to_owned();
    }
    relative_path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        WorkspaceDirectoryCommand, WorkspaceDirectoryCommandError,
        execute_workspace_directory_command,
    };
    use crate::Workspace;

    fn create(
        workspace: &Workspace,
        parent_relative_path: impl Into<PathBuf>,
        name: &str,
    ) -> Result<super::WorkspaceDirectoryCommandResult, WorkspaceDirectoryCommandError> {
        execute_workspace_directory_command(
            workspace,
            WorkspaceDirectoryCommand::Create {
                parent_relative_path: parent_relative_path.into(),
                name: name.to_owned(),
            },
        )
    }

    #[test]
    fn create_makes_exactly_one_direct_child_and_returns_its_relative_path() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        fs::create_dir(temporary_directory.path().join("docs"))
            .expect("parent directory should exist");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        let result = create(&workspace, "docs", "plans").expect("directory should be created");

        assert_eq!(result.relative_path, PathBuf::from("docs/plans"));
        assert!(temporary_directory.path().join("docs/plans").is_dir());
    }

    #[test]
    fn create_rejects_empty_multi_component_absolute_and_reserved_names() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        for name in ["", "  "] {
            assert!(matches!(
                create(&workspace, "", name),
                Err(WorkspaceDirectoryCommandError::EmptyName)
            ));
        }
        for name in [".", "..", "a/b", "a\\b", "/absolute", "bad.", "bad ", "bad:name"] {
            assert!(matches!(
                create(&workspace, "", name),
                Err(WorkspaceDirectoryCommandError::InvalidName { .. })
                    | Err(WorkspaceDirectoryCommandError::ReservedName { .. })
            ));
        }
        for name in [".notora", ".NOTORA", "CON", "com1", "LPT9.log"] {
            assert!(matches!(
                create(&workspace, "", name),
                Err(WorkspaceDirectoryCommandError::ReservedName { .. })
            ));
        }
        assert_eq!(
            fs::read_dir(temporary_directory.path())
                .expect("workspace should remain readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name() != ".notora")
                .count(),
            0
        );
    }

    #[test]
    fn create_rejects_missing_and_non_directory_parents() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        fs::write(temporary_directory.path().join("file"), "content")
            .expect("file fixture should exist");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        assert!(matches!(
            create(&workspace, "missing", "child"),
            Err(WorkspaceDirectoryCommandError::ParentMissing { .. })
        ));
        assert!(matches!(
            create(&workspace, "file", "child"),
            Err(WorkspaceDirectoryCommandError::ParentNotDirectory { .. })
        ));
    }

    #[test]
    fn create_never_overwrites_file_or_directory_targets() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        fs::write(temporary_directory.path().join("file"), "content")
            .expect("file fixture should exist");
        fs::create_dir(temporary_directory.path().join("directory"))
            .expect("directory fixture should exist");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        for name in ["file", "directory"] {
            assert!(matches!(
                create(&workspace, "", name),
                Err(WorkspaceDirectoryCommandError::TargetExists { .. })
            ));
        }
        assert_eq!(
            fs::read_to_string(temporary_directory.path().join("file"))
                .expect("file target should remain unchanged"),
            "content"
        );
    }

    #[cfg(unix)]
    #[test]
    fn create_rejects_symlink_target_conflicts_and_parent_escape() {
        use std::os::unix::fs::symlink;

        let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
        let outside_directory = tempfile::tempdir().expect("outside directory should exist");
        symlink(
            outside_directory.path().join("missing"),
            temporary_directory.path().join("dangling"),
        )
        .expect("dangling symlink should exist");
        symlink(outside_directory.path(), temporary_directory.path().join("outside"))
            .expect("parent symlink should exist");
        let workspace = Workspace::open_or_initialize(temporary_directory.path())
            .expect("workspace should initialize");

        assert!(matches!(
            create(&workspace, "", "dangling"),
            Err(WorkspaceDirectoryCommandError::TargetExists { .. })
        ));
        assert!(matches!(
            create(&workspace, Path::new("outside"), "escaped"),
            Err(WorkspaceDirectoryCommandError::InvalidParent(_))
        ));
        assert!(!outside_directory.path().join("escaped").exists());
    }
}
