use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) fn home_directory() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductPaths {
    pub config_dir: PathBuf,
    pub theme_dir: PathBuf,
    pub workspace_file: PathBuf,
    pub pinned_paths_file: PathBuf,
    pub snapshots_dir: PathBuf,
    pub history_file: PathBuf,
    pub settings_file: PathBuf,
}

impl ProductPaths {
    pub fn textora(home_dir: &Path) -> Self {
        let config_dir = home_dir.join(".textora");
        Self {
            theme_dir: config_dir.join("themes"),
            workspace_file: config_dir.join("workspace.toml"),
            pinned_paths_file: config_dir.join("pinned_paths.json"),
            snapshots_dir: config_dir.join("snapshots"),
            history_file: config_dir.join("history.toml"),
            settings_file: config_dir.join("settings.toml"),
            config_dir,
        }
    }

    pub(crate) fn migrate_legacy_directory(&self, home_dir: &Path) -> io::Result<()> {
        let previous_directory = home_dir.join(".edit+");
        if self.config_dir.try_exists()? || !previous_directory.try_exists()? {
            return Ok(());
        }
        match fs::rename(previous_directory, &self.config_dir) {
            Err(_) if self.config_dir.is_dir() => Ok(()),
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProductPaths;
    use std::path::Path;

    #[test]
    fn textora_paths_preserve_existing_layout() {
        let paths = ProductPaths::textora(Path::new("/home/user"));
        let root = Path::new("/home/user/.textora");
        assert_eq!(paths.config_dir, root);
        assert_eq!(paths.settings_file, root.join("settings.toml"));
        assert_eq!(paths.workspace_file, root.join("workspace.toml"));
        assert_eq!(paths.pinned_paths_file, root.join("pinned_paths.json"));
        assert_eq!(paths.snapshots_dir, root.join("snapshots"));
        assert_eq!(paths.history_file, root.join("history.toml"));
    }

    #[test]
    fn migrates_legacy_configuration_only_when_the_new_directory_is_absent() {
        let home = tempfile::tempdir().expect("temporary home must be writable");
        let legacy = home.path().join(".edit+");
        std::fs::create_dir(&legacy).expect("legacy config directory must be created");
        std::fs::write(legacy.join("settings.toml"), "font_size = 19")
            .expect("legacy settings must be written");
        let paths = ProductPaths::textora(home.path());

        paths.migrate_legacy_directory(home.path()).expect("legacy config must migrate");
        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(&paths.settings_file).expect("migrated settings must exist"),
            "font_size = 19"
        );

        std::fs::create_dir(&legacy).expect("second legacy directory must be created");
        paths
            .migrate_legacy_directory(home.path())
            .expect("an existing config directory must be accepted");
        assert!(legacy.is_dir());
        assert_eq!(
            std::fs::read_to_string(&paths.settings_file).expect("active settings must exist"),
            "font_size = 19"
        );
    }
}
