use std::path::PathBuf;

/// notora 专属的产品路径，不复用其他产品的配置目录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotoraPaths {
    pub config_directory: PathBuf,
    pub settings_file: PathBuf,
    pub session_file: PathBuf,
    pub snapshots_directory: PathBuf,
    pub catalog_backups_directory: PathBuf,
}

#[derive(Debug)]
pub enum NotoraPathsError {
    MissingPlatformConfigDirectory,
    CreateDirectory { path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for NotoraPathsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPlatformConfigDirectory => {
                formatter.write_str("platform configuration directory is unavailable")
            }
            Self::CreateDirectory { path, source } => {
                write!(formatter, "could not create notora directory {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for NotoraPathsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MissingPlatformConfigDirectory => None,
            Self::CreateDirectory { source, .. } => Some(source),
        }
    }
}

impl NotoraPaths {
    pub fn from_platform_directory() -> Result<Self, NotoraPathsError> {
        let platform_config_directory =
            dirs::config_dir().ok_or(NotoraPathsError::MissingPlatformConfigDirectory)?;
        Self::from_config_directory(platform_config_directory.join("notora"))
    }

    pub fn from_config_directory(
        config_directory: impl Into<PathBuf>,
    ) -> Result<Self, NotoraPathsError> {
        let config_directory = config_directory.into();
        let paths = Self {
            settings_file: config_directory.join("settings.toml"),
            session_file: config_directory.join("session.toml"),
            snapshots_directory: config_directory.join("snapshots"),
            catalog_backups_directory: config_directory.join("catalog-backups"),
            config_directory,
        };
        paths.create_directories()?;
        Ok(paths)
    }

    fn create_directories(&self) -> Result<(), NotoraPathsError> {
        for directory in
            [&self.config_directory, &self.snapshots_directory, &self.catalog_backups_directory]
        {
            std::fs::create_dir_all(directory).map_err(|source| {
                NotoraPathsError::CreateDirectory { path: directory.clone(), source }
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{NotoraPaths, NotoraPathsError};

    #[test]
    fn custom_root_keeps_all_product_paths_isolated() {
        let temporary_directory =
            tempfile::tempdir().expect("test should create a temporary directory");
        let config_directory = temporary_directory.path().join("custom-notora");
        let paths = NotoraPaths::from_config_directory(&config_directory)
            .expect("custom config directory should be created");

        assert_eq!(paths.config_directory, config_directory);
        assert_eq!(paths.settings_file, config_directory.join("settings.toml"));
        assert_eq!(paths.session_file, config_directory.join("session.toml"));
        assert_eq!(paths.snapshots_directory, config_directory.join("snapshots"));
        assert_eq!(paths.catalog_backups_directory, config_directory.join("catalog-backups"));
        assert!(paths.snapshots_directory.is_dir());
        assert!(paths.catalog_backups_directory.is_dir());
    }

    #[test]
    fn directory_creation_failure_includes_the_target_path() {
        let temporary_directory =
            tempfile::tempdir().expect("test should create a temporary directory");
        let occupied_path = temporary_directory.path().join("occupied-config-path");
        std::fs::write(&occupied_path, "not a directory")
            .expect("test should create an occupied path");

        let error = NotoraPaths::from_config_directory(&occupied_path)
            .expect_err("a regular file cannot be used as the config directory");
        assert!(matches!(
            error,
            NotoraPathsError::CreateDirectory { path, .. } if path == occupied_path
        ));
    }
}
