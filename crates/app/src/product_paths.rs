use std::path::{Path, PathBuf};

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
        let config_dir = home_dir.join(".edit+");
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
}

#[cfg(test)]
mod tests {
    use super::ProductPaths;
    use std::path::Path;

    #[test]
    fn textora_paths_preserve_existing_layout() {
        let paths = ProductPaths::textora(Path::new("/home/user"));
        let root = Path::new("/home/user/.edit+");
        assert_eq!(paths.config_dir, root);
        assert_eq!(paths.settings_file, root.join("settings.toml"));
        assert_eq!(paths.workspace_file, root.join("workspace.toml"));
        assert_eq!(paths.pinned_paths_file, root.join("pinned_paths.json"));
        assert_eq!(paths.snapshots_dir, root.join("snapshots"));
        assert_eq!(paths.history_file, root.join("history.toml"));
    }
}
