//! File Open History — cross-session history of recently closed files.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 100;
pub const MENU_LIMIT: usize = 20;
const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileHistoryEntry {
    pub file_path: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub last_closed_at: u64,
    pub last_cursor_line: usize,
    pub last_cursor_col: usize,
    /// Saved scroll anchor: document line + pixel offset.
    /// Content-relative, survives window resize.
    pub scroll_anchor_line: usize,
    pub scroll_anchor_offset: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHistory {
    pub version: u32,
    pub excluded_dirs: Vec<PathBuf>,
    pub entries: Vec<FileHistoryEntry>,
}

impl Default for FileHistory {
    fn default() -> Self {
        Self { version: CURRENT_VERSION, entries: Vec::new(), excluded_dirs: Vec::new() }
    }
}

impl FileHistory {
    pub fn load(path: &Path) -> io::Result<Self> {
        Self::load_from(path)
    }

    pub fn load_from(path: &Path) -> io::Result<Self> {
        let toml_str = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let mut h = toml::from_str::<Self>(&toml_str).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse file history at {}: {}", path.display(), e),
            )
        })?;
        h.entries.truncate(MAX_ENTRIES);
        Ok(h)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        Self::save_to(path, self)
    }

    pub fn save_to(path: &Path, history: &Self) -> io::Result<()> {
        let toml_str = toml::to_string_pretty(history)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        crate::persistence::atomic_write(path, toml_str.as_bytes())
    }

    pub fn record(&mut self, entry: FileHistoryEntry) {
        self.entries.retain(|e| e.file_path != entry.file_path);
        self.entries.push(entry);
        self.entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_closed_at));
        self.entries.truncate(MAX_ENTRIES);
    }

    pub fn record_batch(&mut self, entries: Vec<FileHistoryEntry>) {
        let now = current_timestamp_ms();
        for mut entry in entries {
            entry.last_closed_at = now;
            self.record(entry);
        }
    }

    pub fn get_valid_entries(&self, n: usize) -> Vec<&FileHistoryEntry> {
        self.entries
            .iter()
            .filter(|e| std::fs::metadata(&e.file_path).is_ok() && !self.is_excluded(&e.file_path))
            .take(n)
            .collect()
    }

    pub fn get_by_workspace(&self, workspace_root: &Path, n: usize) -> Vec<&FileHistoryEntry> {
        self.entries
            .iter()
            .filter(|e| {
                e.workspace_root.as_deref() == Some(workspace_root)
                    && std::fs::metadata(&e.file_path).is_ok()
                    && !self.is_excluded(&e.file_path)
            })
            .take(n)
            .collect()
    }

    #[allow(dead_code)]
    pub fn remove_entry(&mut self, file_path: &Path) {
        self.entries.retain(|e| e.file_path != file_path);
    }

    #[allow(dead_code)]
    pub fn add_excluded_dir(&mut self, dir: PathBuf) {
        let dir = canonicalize_dir(dir);
        if !self.excluded_dirs.contains(&dir) {
            self.excluded_dirs.push(dir);
        }
    }

    #[allow(dead_code)]
    pub fn remove_excluded_dir(&mut self, dir: &Path) {
        let dir = canonicalize_dir(dir.to_owned());
        self.excluded_dirs.retain(|d| d != &dir);
    }

    /// Check whether `file_path` is inside any excluded directory.
    /// Tries multiple path comparison strategies to handle symlinks.
    pub fn is_excluded(&self, file_path: &Path) -> bool {
        self.excluded_dirs.iter().any(|excluded| {
            // Direct prefix match
            file_path.starts_with(excluded)
                // Try canonicalizing the file_path
                || file_path.canonicalize().is_ok_and(|p| p.starts_with(excluded))
                // Try canonicalizing the excluded dir
                || excluded.canonicalize().is_ok_and(|e| file_path.starts_with(&e))
                // Try canonicalizing both
                || (|| {
                    let fp = file_path.canonicalize().ok()?;
                    let ed = excluded.canonicalize().ok()?;
                    Some(fp.starts_with(&ed))
                })().unwrap_or(false)
        })
    }
}

// ── Helpers ──

fn current_timestamp_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Normalize a directory path to absolute form (no symlink resolution).
#[allow(dead_code)]
fn canonicalize_dir(dir: PathBuf) -> PathBuf {
    if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir().map(|cwd| cwd.join(&dir)).unwrap_or(dir)
    }
}

/// Compute the workspace root from a list of open file paths.
/// Returns the common ancestor directory, or None if there is no
/// meaningful common ancestor (e.g. files on different volumes or
/// only root in common).
pub fn compute_workspace_root(paths: &[&Path]) -> Option<PathBuf> {
    match paths.len() {
        0 => None,
        1 => {
            // For a single file, return its parent directory (if it has one).
            paths[0].parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf())
        }
        _ => {
            let components: Vec<Vec<std::path::Component>> =
                paths.iter().map(|p| p.components().collect()).collect();
            let min_len = components.iter().map(|c| c.len()).min().unwrap_or(0);
            let mut common = 0;
            for i in 0..min_len {
                let c = components[0][i];
                if components.iter().all(|comps| comps[i] == c) {
                    common = i + 1;
                } else {
                    break;
                }
            }
            // If only the root "/" is common, treat as no common ancestor.
            if common <= 1 {
                return None;
            }
            let root: PathBuf = components[0][..common].iter().collect();
            if root.as_os_str().is_empty() { None } else { Some(root) }
        }
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn history_toml_path(config_dir: &Path) -> PathBuf {
        config_dir.join("history.toml")
    }

    #[test]
    fn file_history_save_to_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        // Passing a directory instead of a file should return an error
        let result = FileHistory::save_to(dir.path(), &FileHistory::default());
        assert!(result.is_err());
    }

    #[test]
    fn file_history_load_from_propagates_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.toml");
        std::fs::write(&path, b"invalid = toml = format").unwrap();

        let result = FileHistory::load_from(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    fn temp_config_dir() -> (tempfile::TempDir, PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().to_path_buf();
        (td, p)
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"").unwrap();
        p
    }

    // ── Serialization round-trip ──

    #[test]
    fn test_empty_history_roundtrip() {
        let (_td, cd) = temp_config_dir();
        let h = FileHistory::default();
        let path = history_toml_path(&cd);
        h.save(&path).unwrap();
        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert_eq!(loaded.entries.len(), 0);
        assert_eq!(loaded.version, CURRENT_VERSION);
        assert!(loaded.excluded_dirs.is_empty());
    }

    #[test]
    fn test_record_and_save_load() {
        let (_td, cd) = temp_config_dir();
        let f = touch(cd.as_path(), "some_file.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: f.clone(),
            workspace_root: Some(cd.clone()),
            last_closed_at: 1000,
            last_cursor_line: 42,
            last_cursor_col: 7,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        let path = history_toml_path(&cd);
        h.save(&path).unwrap();
        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].file_path, f);
        assert_eq!(loaded.entries[0].workspace_root, Some(cd.clone()));
        assert_eq!(loaded.entries[0].last_closed_at, 1000);
        assert_eq!(loaded.entries[0].last_cursor_line, 42);
        assert_eq!(loaded.entries[0].last_cursor_col, 7);
    }

    #[test]
    fn history_file_path_roundtrip_preserves_toml_format() {
        let (_td, cd) = temp_config_dir();
        let f = touch(cd.as_path(), "round.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: f.clone(),
            workspace_root: Some(cd.clone()),
            last_closed_at: 1234,
            last_cursor_line: 5,
            last_cursor_col: 10,
            scroll_anchor_line: 1,
            scroll_anchor_offset: 12.5,
        });
        let path = cd.join("history.toml");
        h.save(&path).unwrap();
        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].file_path, f);
        assert_eq!(loaded.entries[0].workspace_root, Some(cd));
        assert_eq!(loaded.entries[0].last_closed_at, 1234);
        assert_eq!(loaded.entries[0].last_cursor_line, 5);
        assert_eq!(loaded.entries[0].last_cursor_col, 10);
        assert_eq!(loaded.entries[0].scroll_anchor_line, 1);
        assert_eq!(loaded.entries[0].scroll_anchor_offset, 12.5);
    }

    #[test]
    fn test_record_dedup_same_path() {
        let (_td, cd) = temp_config_dir();
        let f = touch(cd.as_path(), "dup.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: f.clone(),
            workspace_root: Some(cd.clone()),
            last_closed_at: 1000,
            last_cursor_line: 1,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: f.clone(),
            workspace_root: Some(cd.clone()),
            last_closed_at: 2000,
            last_cursor_line: 5,
            last_cursor_col: 2,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0].last_closed_at, 2000);
        assert_eq!(h.entries[0].last_cursor_line, 5);
    }

    #[test]
    fn test_record_desc_order() {
        let (_td, cd) = temp_config_dir();
        let f1 = touch(cd.as_path(), "a.rs");
        let f2 = touch(cd.as_path(), "b.rs");
        let f3 = touch(cd.as_path(), "c.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: f1.clone(),
            workspace_root: None,
            last_closed_at: 3000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: f2.clone(),
            workspace_root: None,
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: f3.clone(),
            workspace_root: None,
            last_closed_at: 5000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        assert_eq!(h.entries[0].last_closed_at, 5000);
        assert_eq!(h.entries[1].last_closed_at, 3000);
        assert_eq!(h.entries[2].last_closed_at, 1000);
    }

    #[test]
    fn test_truncate_to_max() {
        let (_td, cd) = temp_config_dir();
        let mut h = FileHistory::default();
        for i in 0..(MAX_ENTRIES + 50) {
            let f = touch(cd.as_path(), &format!("f{}.rs", i));
            h.record(FileHistoryEntry {
                file_path: f,
                workspace_root: None,
                last_closed_at: i as u64,
                last_cursor_line: 0,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            });
        }
        assert_eq!(h.entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn test_corrupt_file_returns_empty() {
        let (_td, cd) = temp_config_dir();
        let path = history_toml_path(&cd);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"this is not valid toml!!@#").unwrap();
        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert_eq!(loaded.entries.len(), 0);
    }

    #[test]
    fn test_missing_file_returns_empty() {
        let (_td, cd) = temp_config_dir();
        let path = history_toml_path(&cd);
        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert_eq!(loaded.entries.len(), 0);
    }

    // ── get_valid_entries ──

    #[test]
    fn test_valid_entries_skips_nonexistent() {
        let (_td, cd) = temp_config_dir();
        let real_file = touch(cd.as_path(), "real.rs");
        let ghost = cd.join("ghost.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: real_file.clone(),
            workspace_root: None,
            last_closed_at: 2000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: ghost,
            workspace_root: None,
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        let valid = h.get_valid_entries(10);
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].file_path, real_file);
    }

    #[test]
    fn test_valid_entries_skips_excluded() {
        let (_td, cd) = temp_config_dir();
        let sub = cd.join("vendored");
        std::fs::create_dir_all(&sub).unwrap();
        let excluded_file = touch(&sub, "lib.rs");
        let normal_file = touch(cd.as_path(), "normal.rs");

        let mut h = FileHistory::default();
        h.add_excluded_dir(sub.clone());
        h.record(FileHistoryEntry {
            file_path: excluded_file,
            workspace_root: None,
            last_closed_at: 2000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: normal_file.clone(),
            workspace_root: None,
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });

        let valid = h.get_valid_entries(10);
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].file_path, normal_file);
    }

    #[test]
    fn test_valid_entries_respects_n() {
        let (_td, cd) = temp_config_dir();
        let mut h = FileHistory::default();
        for i in 0..10 {
            let f = touch(cd.as_path(), &format!("f{}.rs", i));
            h.record(FileHistoryEntry {
                file_path: f,
                workspace_root: None,
                last_closed_at: i as u64,
                last_cursor_line: 0,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            });
        }
        assert_eq!(h.get_valid_entries(3).len(), 3);
        assert_eq!(h.get_valid_entries(10).len(), 10);
    }

    // ── get_by_workspace ──

    #[test]
    fn test_filter_by_workspace() {
        let (_td, cd) = temp_config_dir();
        let ws_a = cd.join("proj_a");
        let ws_b = cd.join("proj_b");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        let fa = touch(&ws_a, "a.rs");
        let fb = touch(&ws_b, "b.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: fa.clone(),
            workspace_root: Some(ws_a.clone()),
            last_closed_at: 2000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: fb,
            workspace_root: Some(ws_b.clone()),
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        let a_entries = h.get_by_workspace(&ws_a, 10);
        assert_eq!(a_entries.len(), 1);
        assert_eq!(a_entries[0].file_path, fa);
    }

    // ── remove_entry ──

    #[test]
    fn test_remove_entry() {
        let (_td, cd) = temp_config_dir();
        let f1 = touch(cd.as_path(), "keep.rs");
        let f2 = touch(cd.as_path(), "rm.rs");
        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: f1.clone(),
            workspace_root: None,
            last_closed_at: 2000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: f2.clone(),
            workspace_root: None,
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.remove_entry(&f2);
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0].file_path, f1);
        h.remove_entry(&PathBuf::from("/nonexistent"));
        assert_eq!(h.entries.len(), 1);
    }

    // ── excluded_dirs ──

    #[test]
    fn test_excluded_dir_roundtrip() {
        let (_td, cd) = temp_config_dir();
        let excl = cd.join("vendor");
        std::fs::create_dir_all(&excl).unwrap();
        let mut h = FileHistory::default();
        h.add_excluded_dir(excl.clone());
        assert!(h.excluded_dirs.contains(&excl));
        h.remove_excluded_dir(&excl);
        assert!(!h.is_excluded(&excl.join("foo.rs")));
    }

    #[test]
    fn test_excluded_dir_dedup() {
        let (_td, cd) = temp_config_dir();
        let excl = cd.join("vendor");
        std::fs::create_dir_all(&excl).unwrap();
        let mut h = FileHistory::default();
        h.add_excluded_dir(excl.clone());
        h.add_excluded_dir(excl.clone());
        assert_eq!(h.excluded_dirs.len(), 1, "should deduplicate");
    }

    // ── record_batch ──

    #[test]
    fn test_record_batch_same_timestamp() {
        let (_td, cd) = temp_config_dir();
        let f1 = touch(cd.as_path(), "x.rs");
        let f2 = touch(cd.as_path(), "y.rs");
        let mut h = FileHistory::default();
        h.record_batch(vec![
            FileHistoryEntry {
                file_path: f1.clone(),
                workspace_root: None,
                last_closed_at: 0,
                last_cursor_line: 0,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            },
            FileHistoryEntry {
                file_path: f2.clone(),
                workspace_root: None,
                last_closed_at: 0,
                last_cursor_line: 0,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            },
        ]);
        assert_eq!(h.entries.len(), 2);
        let ts = h.entries[0].last_closed_at;
        assert!(ts > 0);
        assert_eq!(h.entries[1].last_closed_at, ts);
    }

    // ── compute_workspace_root ──

    #[test]
    fn test_workspace_root_common_ancestor() {
        let root = compute_workspace_root(&[
            Path::new("/home/user/proj/src/a.rs"),
            Path::new("/home/user/proj/src/b.rs"),
            Path::new("/home/user/proj/Cargo.toml"),
        ]);
        assert_eq!(root, Some(PathBuf::from("/home/user/proj")));
    }

    #[test]
    fn test_workspace_root_single_file() {
        let root = compute_workspace_root(&[Path::new("/tmp/foo.rs")]);
        assert_eq!(root, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn test_workspace_root_empty() {
        assert_eq!(compute_workspace_root(&[]), None);
    }

    #[test]
    fn test_workspace_root_no_common() {
        let root = compute_workspace_root(&[Path::new("/a/foo.rs"), Path::new("/b/bar.rs")]);
        assert_eq!(root, None);
    }

    // ── Additional edge cases ──

    #[test]
    fn test_save_load_with_excluded_dirs() {
        let (_td, cd) = temp_config_dir();
        let excl = cd.join("vendor");
        std::fs::create_dir_all(&excl).unwrap();

        let mut h = FileHistory::default();
        h.add_excluded_dir(excl.clone());
        h.record(FileHistoryEntry {
            file_path: touch(cd.as_path(), "f.rs"),
            workspace_root: None,
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        let path = history_toml_path(&cd);
        h.save(&path).unwrap();

        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert_eq!(loaded.excluded_dirs.len(), 1);
        assert!(loaded.is_excluded(&excl.join("anything.rs")));
        assert!(!loaded.is_excluded(&cd.join("other.rs")));
    }

    #[test]
    fn test_get_by_workspace_skips_nonexistent() {
        let (_td, cd) = temp_config_dir();
        let ws = cd.join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let real = touch(&ws, "real.rs");
        let ghost = ws.join("ghost.rs");

        let mut h = FileHistory::default();
        h.record(FileHistoryEntry {
            file_path: real.clone(),
            workspace_root: Some(ws.clone()),
            last_closed_at: 2000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        h.record(FileHistoryEntry {
            file_path: ghost,
            workspace_root: Some(ws.clone()),
            last_closed_at: 1000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });

        let entries = h.get_by_workspace(&ws, 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_path, real);
    }

    #[test]
    fn test_record_batch_dedup_same_path() {
        let (_td, cd) = temp_config_dir();
        let f = touch(cd.as_path(), "dup.rs");

        let mut h = FileHistory::default();
        h.record_batch(vec![
            FileHistoryEntry {
                file_path: f.clone(),
                workspace_root: None,
                last_closed_at: 0,
                last_cursor_line: 1,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            },
            FileHistoryEntry {
                file_path: f.clone(),
                workspace_root: None,
                last_closed_at: 0,
                last_cursor_line: 2,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            },
        ]);
        assert_eq!(h.entries.len(), 1);
    }

    #[test]
    fn test_compute_workspace_root_different_volumes() {
        let root = compute_workspace_root(&[
            Path::new("/VolumeA/src/lib.rs"),
            Path::new("/VolumeB/src/main.rs"),
        ]);
        assert_eq!(root, None);
    }

    #[test]
    fn test_load_truncates_oversized() {
        let (_td, cd) = temp_config_dir();
        let mut h = FileHistory::default();
        for i in 0..(MAX_ENTRIES + 30) {
            let f = touch(cd.as_path(), &format!("f{}.rs", i));
            h.record(FileHistoryEntry {
                file_path: f,
                workspace_root: None,
                last_closed_at: i as u64,
                last_cursor_line: 0,
                last_cursor_col: 0,
                scroll_anchor_line: 0,
                scroll_anchor_offset: 0.0,
            });
        }
        let path = history_toml_path(&cd);
        h.save(&path).unwrap();

        let loaded = FileHistory::load(&path).unwrap_or_default();
        assert!(loaded.entries.len() <= MAX_ENTRIES);
    }

    #[test]
    fn test_empty_entries_save_creates_empty_file() {
        let (_td, cd) = temp_config_dir();
        let h = FileHistory::default();
        let path = history_toml_path(&cd);
        h.save(&path).unwrap();

        let path = history_toml_path(&cd);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version"));
        assert!(contents.contains("entries"));
    }
}
