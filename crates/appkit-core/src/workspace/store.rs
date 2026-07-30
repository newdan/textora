use std::io;
use std::path::{Path, PathBuf};

use crate::workspace::types::PersistedWorkspace;

pub struct WorkspaceStore {
    workspace_file: PathBuf,
    pinned_paths_file: PathBuf,
    snapshots_dir: PathBuf,
}

impl WorkspaceStore {
    pub fn new(
        workspace_file: PathBuf,
        pinned_paths_file: PathBuf,
        snapshots_dir: PathBuf,
    ) -> Self {
        Self { workspace_file, pinned_paths_file, snapshots_dir }
    }

    fn workspace_toml_path(&self) -> &Path {
        &self.workspace_file
    }

    fn pinned_file(&self) -> &Path {
        &self.pinned_paths_file
    }

    pub fn load_workspace(&self) -> io::Result<Option<PersistedWorkspace>> {
        let path = self.workspace_toml_path();
        let toml_str = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        toml::from_str(&toml_str)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
            .map(Some)
    }

    pub fn save_workspace(&self, snapshot: &PersistedWorkspace) -> io::Result<()> {
        let toml_str = toml::to_string_pretty(snapshot)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        crate::persistence::atomic_write(self.workspace_toml_path(), toml_str.as_bytes())
    }

    /// Remove orphaned snapshot files that are no longer referenced by any tab.
    ///
    /// Only files under the injected `snapshots_dir` are examined or deleted.
    pub fn cleanup_snapshot_orphans(&self) {
        let path = self.workspace_toml_path();
        let toml_str = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return,
        };
        let snap: PersistedWorkspace = match toml::from_str(&toml_str) {
            Ok(s) => s,
            Err(_) => return,
        };
        let active: std::collections::HashSet<String> =
            snap.entries.iter().filter_map(|t| t.snapshot_filename.clone()).collect();
        crate::snapshot::cleanup_orphans(&self.snapshots_dir, &active);
    }

    pub fn load_pinned_paths(&self) -> io::Result<Vec<PathBuf>> {
        let path = self.pinned_file();
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut paths = Vec::new();
        for line in contents.lines() {
            let line = line.trim();
            if !line.is_empty() {
                paths.push(PathBuf::from(line));
            }
        }
        Ok(paths)
    }

    pub fn save_pinned_paths(&self, paths: &[PathBuf]) -> io::Result<()> {
        let mut lines = Vec::new();
        for p in paths {
            lines.push(p.to_string_lossy().to_string());
        }
        let data = lines.join("\n");
        crate::persistence::atomic_write(self.pinned_file(), data.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::types::PersistedTab;

    fn minimal_workspace(snapshot_filename: Option<&str>) -> PersistedWorkspace {
        PersistedWorkspace {
            version: 1,
            active_index: 0,
            sidebar_pinned: false,
            sidebar_width: None,
            entries: vec![PersistedTab {
                file_path: None,
                suggested_file_name: None,
                cursor_offset: 0,
                selection_anchor: None,
                dirty: true,
                scroll_anchor_line: None,
                scroll_anchor_offset: None,
                snapshot_filename: snapshot_filename.map(str::to_owned),
                original_file_size: None,
                original_mtime_secs: None,
                original_disk_revision: None,
                unsaved_lines: None,
                active_plugin: None,
                preview_anchor_text: None,
                preview_anchor_offset: None,
                clean_untitled_content: None,
            }],
        }
    }

    #[test]
    fn cleanup_orphans_only_touches_injected_snapshot_directory() {
        let base = tempfile::tempdir().expect("temporary workspace directory should be created");
        let workspace_file = base.path().join("workspace.toml");
        let pinned_file = base.path().join("pinned_paths.json");
        let injected_snapshots = base.path().join("injected_snapshots");
        let other_snapshots = base.path().join("other_snapshots");

        std::fs::create_dir(&injected_snapshots)
            .expect("injected snapshots directory should be created");
        std::fs::create_dir(&other_snapshots)
            .expect("unrelated snapshots directory should be created");

        let workspace = minimal_workspace(Some("keep.dirty"));
        let workspace_toml =
            toml::to_string_pretty(&workspace).expect("workspace fixture should serialize to TOML");
        std::fs::write(&workspace_file, workspace_toml)
            .expect("workspace fixture should be written");

        std::fs::write(injected_snapshots.join("keep.dirty"), b"keep")
            .expect("active snapshot fixture should be written");
        std::fs::write(injected_snapshots.join("orphan.dirty"), b"orphan")
            .expect("orphan snapshot fixture should be written");
        std::fs::write(other_snapshots.join("other_orphan.dirty"), b"other")
            .expect("unrelated snapshot fixture should be written");

        let store =
            WorkspaceStore::new(workspace_file.clone(), pinned_file, injected_snapshots.clone());
        store.cleanup_snapshot_orphans();

        assert!(injected_snapshots.join("keep.dirty").exists(), "active snapshot kept");
        assert!(
            !injected_snapshots.join("orphan.dirty").exists(),
            "orphan in injected dir removed"
        );
        assert!(
            other_snapshots.join("other_orphan.dirty").exists(),
            "unrelated snapshot directory is untouched"
        );

        // Sanity check that an empty active set still only touches the injected dir.
        let workspace = minimal_workspace(None);
        let workspace_toml = toml::to_string_pretty(&workspace)
            .expect("workspace fixture should serialize to TOML on second cleanup pass");
        std::fs::write(&workspace_file, workspace_toml)
            .expect("workspace fixture should be rewritten for second cleanup pass");
        std::fs::write(injected_snapshots.join("keep.dirty"), b"keep")
            .expect("active snapshot fixture should be rewritten for second cleanup pass");
        store.cleanup_snapshot_orphans();
        assert!(!injected_snapshots.join("keep.dirty").exists(), "unreferenced snapshot removed");
        assert!(other_snapshots.join("other_orphan.dirty").exists(), "other dir still untouched");
    }
}
