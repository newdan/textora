//! Diff-based dirty snapshot module.
//!
//! Stores per-tab dirty content as unified diff files instead of inline
//! `unsaved_lines` in workspace.toml. This keeps the workspace file small
//! and makes external modification detection possible via mtime/size in the
//! diff header.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Deterministic filename-safe identifier derived from a file path.
/// Same path always produces the same id (within one binary version).
pub fn path_id(path: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Generate a unique snapshot id for an untitled tab.
pub fn untitled_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("untitled_{:x}_{:x}", ts, n)
}

pub const SNAPSHOT_EXT: &str = "dirty";

pub fn snapshot_filename(id: &str) -> String {
    format!("{}.{}", id, SNAPSHOT_EXT)
}

// ─── Internal hunk types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Hunk {
    old_start: usize, // 0-based line index in original
    old_count: usize,
    new_start: usize, // 0-based line index in result
    new_count: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Delete(String),
    Insert(String),
}

/// Group fine-grained diff changes into hunks with 3 lines of context.
fn group_into_hunks(diff: &TextDiff<'_, '_, '_, str>) -> Vec<Hunk> {
    let mut hunks = Vec::new();

    for group in diff.grouped_ops(3) {
        if group.is_empty() {
            continue;
        }

        let mut old_count = 0usize;
        let mut new_count = 0usize;
        let mut hunk_lines = Vec::new();
        let mut old_start = 0usize;
        let mut new_start = 0usize;
        let mut first = true;

        for op in &group {
            for change in diff.iter_changes(op) {
                if first {
                    old_start = change.old_index().unwrap_or(0);
                    new_start = change.new_index().unwrap_or(0);
                    first = false;
                }
                let s = change.value().to_string();
                match change.tag() {
                    ChangeTag::Equal => {
                        hunk_lines.push(HunkLine::Context(s));
                        old_count += 1;
                        new_count += 1;
                    }
                    ChangeTag::Delete => {
                        hunk_lines.push(HunkLine::Delete(s));
                        old_count += 1;
                    }
                    ChangeTag::Insert => {
                        hunk_lines.push(HunkLine::Insert(s));
                        new_count += 1;
                    }
                }
            }
        }

        hunks.push(Hunk { old_start, old_count, new_start, new_count, lines: hunk_lines });
    }

    hunks
}

// ─── Snapshot header ─────────────────────────────────────────────────────────

const HEADER_MAGIC: &str = "EDITPLUS-DIFF";
const HEADER_VERSION: u32 = 2;
const CURRENT_CONTENT_BEGIN: &str = "EDITPLUS-CURRENT-CONTENT";
const CURRENT_CONTENT_END: &str = "EDITPLUS-END-CURRENT-CONTENT";

/// Serializable representation of a disk revision used by hot-exit snapshots.
///
/// `DiskRevision` intentionally contains `SystemTime` and `blake3::Hash`, which
/// are useful at runtime but are not a stable TOML contract. This type keeps the
/// persisted format explicit and forward-compatible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDiskRevision {
    pub size: u64,
    #[serde(default)]
    pub modified_unix_secs: Option<i64>,
    #[serde(default)]
    pub modified_unix_nanos: u32,
    #[serde(default)]
    pub content_hash_hex: String,
    #[serde(default)]
    pub file_device: Option<u64>,
    #[serde(default)]
    pub file_inode: Option<u64>,
}

impl PersistedDiskRevision {
    pub fn from_disk_revision(revision: &crate::file_safety::DiskRevision) -> Self {
        let (modified_unix_secs, modified_unix_nanos) = revision
            .modified
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| (Some(duration.as_secs() as i64), duration.subsec_nanos()))
            .unwrap_or((None, 0));
        let (file_device, file_inode) = revision
            .file_identity
            .as_ref()
            .map(|identity| (Some(identity.device), Some(identity.inode)))
            .unwrap_or((None, None));

        Self {
            size: revision.size,
            modified_unix_secs,
            modified_unix_nanos,
            content_hash_hex: revision.content_hash.to_hex().to_string(),
            file_device,
            file_inode,
        }
    }

    pub fn to_disk_revision(&self, path: &Path) -> Option<crate::file_safety::DiskRevision> {
        let content_hash = blake3::Hash::from_hex(&self.content_hash_hex).ok()?;
        let modified = self.modified_unix_secs.and_then(|seconds| {
            if seconds < 0 || self.modified_unix_nanos >= 1_000_000_000 {
                return None;
            }
            UNIX_EPOCH.checked_add(Duration::new(seconds as u64, self.modified_unix_nanos))
        });
        let file_identity = self
            .file_device
            .zip(self.file_inode)
            .map(|(device, inode)| core::disk_revision::FileIdentity { device, inode });

        Some(crate::file_safety::DiskRevision {
            path: path.to_owned(),
            size: self.size,
            modified,
            content_hash,
            file_identity,
        })
    }
}

/// Metadata stored in the diff file header.
#[derive(Debug, Clone)]
pub struct SnapshotHeader {
    pub file_size: u64,
    pub mtime_secs: i64,
    pub baseline_revision: Option<PersistedDiskRevision>,
}

impl SnapshotHeader {
    fn to_header_lines(&self) -> Vec<String> {
        vec![
            format!("magic: {}", HEADER_MAGIC),
            format!("version: {}", HEADER_VERSION),
            format!("file_size: {}", self.file_size),
            format!("mtime_secs: {}", self.mtime_secs),
            format!(
                "baseline_revision: {}",
                self.baseline_revision
                    .as_ref()
                    .map(|revision| revision.content_hash_hex.as_str())
                    .unwrap_or("")
            ),
            format!(
                "baseline_size: {}",
                self.baseline_revision.as_ref().map(|revision| revision.size).unwrap_or(0)
            ),
            format!(
                "baseline_modified_secs: {}",
                self.baseline_revision
                    .as_ref()
                    .and_then(|revision| revision.modified_unix_secs)
                    .map(|seconds| seconds.to_string())
                    .unwrap_or_default()
            ),
            format!(
                "baseline_modified_nanos: {}",
                self.baseline_revision
                    .as_ref()
                    .map(|revision| revision.modified_unix_nanos)
                    .unwrap_or(0)
            ),
            format!(
                "baseline_device: {}",
                self.baseline_revision
                    .as_ref()
                    .and_then(|revision| revision.file_device)
                    .map(|device| device.to_string())
                    .unwrap_or_default()
            ),
            format!(
                "baseline_inode: {}",
                self.baseline_revision
                    .as_ref()
                    .and_then(|revision| revision.file_inode)
                    .map(|inode| inode.to_string())
                    .unwrap_or_default()
            ),
        ]
    }

    fn from_header_lines(lines: &[String]) -> Option<Self> {
        let mut magic_ok = false;
        let mut file_size = 0u64;
        let mut mtime_secs = 0i64;
        let mut baseline_hash = None;
        let mut baseline_size = None;
        let mut baseline_modified_secs = None;
        let mut has_baseline_modified_field = false;
        let mut baseline_modified_nanos = 0;
        let mut baseline_device = None;
        let mut baseline_inode = None;

        for line in lines {
            if let Some(rest) = line.strip_prefix("magic: ") {
                if rest.trim() == HEADER_MAGIC {
                    magic_ok = true;
                }
            } else if let Some(rest) = line.strip_prefix("file_size: ") {
                file_size = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("mtime_secs: ") {
                mtime_secs = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("baseline_revision: ")
                && !rest.trim().is_empty()
            {
                baseline_hash = Some(rest.trim().to_owned());
            } else if let Some(rest) = line.strip_prefix("baseline_size: ") {
                baseline_size = rest.trim().parse().ok();
            } else if let Some(rest) = line.strip_prefix("baseline_modified_secs: ") {
                has_baseline_modified_field = true;
                baseline_modified_secs = rest.trim().parse().ok();
            } else if let Some(rest) = line.strip_prefix("baseline_modified_nanos: ") {
                baseline_modified_nanos = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("baseline_device: ") {
                baseline_device = rest.trim().parse().ok();
            } else if let Some(rest) = line.strip_prefix("baseline_inode: ") {
                baseline_inode = rest.trim().parse().ok();
            }
        }

        let baseline_revision = baseline_hash.map(|content_hash_hex| PersistedDiskRevision {
            size: baseline_size.unwrap_or(file_size),
            modified_unix_secs: if has_baseline_modified_field {
                baseline_modified_secs
            } else {
                Some(mtime_secs)
            },
            modified_unix_nanos: baseline_modified_nanos,
            content_hash_hex,
            file_device: baseline_device,
            file_inode: baseline_inode,
        });
        if magic_ok {
            Some(SnapshotHeader { file_size, mtime_secs, baseline_revision })
        } else {
            None
        }
    }
}

// ─── Unified diff serialization ──────────────────────────────────────────────

fn hunk_to_lines(hunk: &Hunk) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start + 1,
        hunk.old_count,
        hunk.new_start + 1,
        hunk.new_count
    ));
    for hl in &hunk.lines {
        match hl {
            HunkLine::Context(s) => lines.push(format!(" {}", s)),
            HunkLine::Delete(s) => lines.push(format!("-{}", s)),
            HunkLine::Insert(s) => lines.push(format!("+{}", s)),
        }
    }
    lines
}

fn parse_hunk_line(line: &str) -> Option<HunkLine> {
    if let Some(rest) = line.strip_prefix(' ') {
        Some(HunkLine::Context(rest.to_string()))
    } else if let Some(rest) = line.strip_prefix('-') {
        Some(HunkLine::Delete(rest.to_string()))
    } else {
        line.strip_prefix('+').map(|rest| HunkLine::Insert(rest.to_string()))
    }
}

fn parse_hunks(lines: &[String]) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = &lines[i];
        if !line.starts_with("@@") {
            i += 1;
            continue;
        }

        // Parse @@ -old_start,old_count +new_start,new_count @@
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            i += 1;
            continue;
        }

        let (old_start, old_count) = parse_range(parts[1]);
        let (new_start, new_count) = parse_range(parts[2]);

        let mut hunk_lines = Vec::new();
        i += 1;
        while i < lines.len() {
            let l = &lines[i];
            if l.starts_with("@@") {
                break;
            }
            if let Some(hl) = parse_hunk_line(l) {
                hunk_lines.push(hl);
            }
            i += 1;
        }

        hunks.push(Hunk { old_start, old_count, new_start, new_count, lines: hunk_lines });
    }

    hunks
}

/// Parse a range string like "-1,5" or "+1,5" into (start_0based, count).
fn parse_range(s: &str) -> (usize, usize) {
    let s = s.trim_start_matches(['-', '+']);
    let parts: Vec<&str> = s.split(',').collect();
    let start = parts.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
    let count = parts.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
    (start.saturating_sub(1), count)
}

// ─── Apply hunks to reconstruct ──────────────────────────────────────────────

fn apply_hunks(original: &[String], hunks: &[Hunk]) -> Vec<String> {
    let mut result = Vec::new();
    let mut old_pos = 0usize;

    for hunk in hunks {
        // Copy unchanged lines before this hunk
        while old_pos < hunk.old_start && old_pos < original.len() {
            result.push(original[old_pos].clone());
            old_pos += 1;
        }

        for hl in &hunk.lines {
            match hl {
                HunkLine::Context(s) => {
                    // Context lines should match original; skip original line
                    old_pos += 1;
                    result.push(s.clone());
                }
                HunkLine::Delete(_) => {
                    // Deleted line; skip original line
                    old_pos += 1;
                }
                HunkLine::Insert(s) => {
                    // Inserted line; don't advance original position
                    result.push(s.clone());
                }
            }
        }
    }

    // Copy remaining lines after last hunk
    while old_pos < original.len() {
        result.push(original[old_pos].clone());
        old_pos += 1;
    }

    result
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Write a diff snapshot file for a dirty tab.
///
/// `dir` — directory to write into (injected by the caller, e.g. `ProductPaths::snapshots_dir`).
/// `filename` — snapshot filename (e.g. `"abc123.dirty"`).
/// `file_size` / `mtime_secs` — original file metadata for external-change detection.
/// `original` — lines of the file as loaded from disk (or empty for untitled).
/// `current` — lines of the dirty buffer.
pub fn write_snapshot(
    dir: &Path,
    filename: &str,
    file_size: u64,
    mtime_secs: i64,
    original: &[String],
    current: &[String],
) -> std::io::Result<()> {
    write_snapshot_internal(dir, filename, file_size, mtime_secs, None, original, current)
}

/// Write a snapshot with the exact disk revision used as its diff base.
pub fn write_snapshot_with_revision(
    dir: &Path,
    filename: &str,
    baseline: &crate::file_safety::DiskRevision,
    original: &[String],
    current: &[String],
) -> std::io::Result<()> {
    let mtime_secs = baseline
        .modified
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    write_snapshot_internal(
        dir,
        filename,
        baseline.size,
        mtime_secs,
        Some(baseline),
        original,
        current,
    )
}

fn write_snapshot_internal(
    dir: &Path,
    filename: &str,
    file_size: u64,
    mtime_secs: i64,
    baseline: Option<&crate::file_safety::DiskRevision>,
    original: &[String],
    current: &[String],
) -> std::io::Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }

    let header = SnapshotHeader {
        file_size,
        mtime_secs,
        baseline_revision: baseline.map(PersistedDiskRevision::from_disk_revision),
    };

    let orig_refs: Vec<&str> = original.iter().map(|s| s.as_str()).collect();
    let curr_refs: Vec<&str> = current.iter().map(|s| s.as_str()).collect();
    let diff = TextDiff::from_slices(&orig_refs, &curr_refs);

    let hunks = group_into_hunks(&diff);

    let mut out = String::new();
    // Header
    for line in header.to_header_lines() {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');

    // Hunks
    for hunk in &hunks {
        for line in hunk_to_lines(hunk) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str(CURRENT_CONTENT_BEGIN);
    out.push('\n');
    for line in current {
        out.push_str(&encode_snapshot_line(line));
        out.push('\n');
    }
    out.push_str(CURRENT_CONTENT_END);
    out.push('\n');

    let path = dir.join(filename);
    crate::persistence::atomic_write(&path, out.as_bytes())
}

fn encode_snapshot_line(line: &str) -> String {
    let mut encoded = String::with_capacity(line.len() * 2);
    for byte in line.as_bytes() {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

fn decode_snapshot_line(line: &str) -> Option<String> {
    if !line.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(line.len() / 2);
    let mut index = 0;
    while index < line.len() {
        let byte = u8::from_str_radix(&line[index..index + 2], 16).ok()?;
        bytes.push(byte);
        index += 2;
    }
    String::from_utf8(bytes).ok()
}

/// Read a diff snapshot and apply it to the original lines.
///
/// Returns `(restored_lines, header)` on success.
/// Returns `Err` if the file is missing or the diff cannot be applied.
pub fn read_and_apply(
    dir: &Path,
    filename: &str,
    original: &[String],
) -> std::io::Result<(Vec<String>, SnapshotHeader)> {
    let path = dir.join(filename);
    let content = std::fs::read_to_string(&path)?;

    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    // Find the blank line separating header from hunks
    let header_end = lines.iter().position(|l| l.is_empty()).unwrap_or(lines.len());

    let header_lines = &lines[..header_end];
    let header = SnapshotHeader::from_header_lines(header_lines).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid snapshot header")
    })?;

    let body = &lines[header_end..];
    let restored = if let Some(start) = body.iter().position(|line| line == CURRENT_CONTENT_BEGIN) {
        let end = body
            .iter()
            .position(|line| line == CURRENT_CONTENT_END)
            .filter(|end| *end > start)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid snapshot content")
            })?;
        body[start + 1..end]
            .iter()
            .map(|line| {
                decode_snapshot_line(line).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid encoded snapshot content",
                    )
                })
            })
            .collect::<std::io::Result<Vec<_>>>()?
    } else {
        let hunks = parse_hunks(body);
        apply_hunks(original, &hunks)
    };

    Ok((restored, header))
}

/// Delete snapshot files that are no longer referenced by any open tab.
///
/// `dir` — snapshot directory.
/// `active_filenames` — set of filenames currently in use (e.g. `"abc123.dirty"`).
pub fn cleanup_orphans(dir: &Path, active_filenames: &HashSet<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(&format!(".{}", SNAPSHOT_EXT)) && !active_filenames.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Resolve the snapshot filename for a given file path (named tab).
pub fn snapshot_id_for_path(path: &Path) -> String {
    snapshot_filename(&path_id(path))
}

/// Resolve the snapshot filename for an untitled tab, reusing an existing id if provided.
#[allow(dead_code)]
pub fn snapshot_id_for_untitled(existing_id: Option<&str>) -> String {
    let id = existing_id.map(|s| s.to_string()).unwrap_or_else(untitled_id);
    snapshot_filename(&id)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_path_id_deterministic() {
        let p = Path::new("/home/user/project/src/main.rs");
        let a = path_id(p);
        let b = path_id(p);
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn test_untitled_id_unique() {
        let a = untitled_id();
        let b = untitled_id();
        assert_ne!(a, b);
        assert!(a.starts_with("untitled_"));
    }

    #[test]
    fn test_snapshot_filename_format() {
        let f = snapshot_filename("abc123");
        assert_eq!(f, "abc123.dirty");
    }

    #[test]
    fn test_snapshot_id_for_path() {
        let p = Path::new("/tmp/test.txt");
        let f = snapshot_id_for_path(p);
        assert!(f.ends_with(".dirty"));
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let original: Vec<String> = vec![
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
            "line4".to_string(),
            "line5".to_string(),
        ];
        let current: Vec<String> = vec![
            "line1".to_string(),
            "line2 modified".to_string(),
            "line3".to_string(),
            "line4".to_string(),
            "line5".to_string(),
        ];

        let filename = "test.dirty";
        write_snapshot(snap_dir, filename, 0, 0, &original, &current).expect("write");

        let (restored, _header) = read_and_apply(snap_dir, filename, &original).expect("read");
        assert_eq!(restored, current);
    }

    #[test]
    fn test_read_uses_saved_content_when_disk_base_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = vec!["remote base".to_string()];
        let current = vec!["local unsaved edit".to_string()];

        write_snapshot(dir.path(), "changed-base.dirty", 10, 20, &original, &current)
            .expect("write");

        let (restored, _) =
            read_and_apply(dir.path(), "changed-base.dirty", &["remote replacement".to_string()])
                .expect("read");

        assert_eq!(restored, current);
    }

    #[test]
    fn test_untitled_snapshot_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let empty: Vec<String> = Vec::new();
        let content: Vec<String> =
            vec!["hello world".to_string(), "".to_string(), "foo bar".to_string()];

        let filename = "untitled_test.dirty";
        write_snapshot(snap_dir, filename, 0, 0, &empty, &content).expect("write");

        let (restored, _header) = read_and_apply(snap_dir, filename, &empty).expect("read");
        assert_eq!(restored, content);
    }

    #[test]
    fn test_external_modification_detected() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let original: Vec<String> = vec!["original".to_string(), "content".to_string()];
        let modified: Vec<String> = vec!["original".to_string(), "modified".to_string()];

        write_snapshot(snap_dir, "test.dirty", 100, 1000, &original, &modified).expect("write");

        let (_restored, header) = read_and_apply(snap_dir, "test.dirty", &original).expect("read");
        assert_eq!(header.file_size, 100);
        assert_eq!(header.mtime_secs, 1000);
    }

    #[test]
    fn test_orphan_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        std::fs::write(snap_dir.join("keep.dirty"), b"content").expect("write");
        std::fs::write(snap_dir.join("orphan.dirty"), b"content").expect("write");
        std::fs::write(snap_dir.join("other.txt"), b"not a snapshot").expect("write");

        let mut active = HashSet::new();
        active.insert("keep.dirty".to_string());

        cleanup_orphans(snap_dir, &active);

        assert!(snap_dir.join("keep.dirty").exists(), "referenced file kept");
        assert!(!snap_dir.join("orphan.dirty").exists(), "orphan deleted");
        assert!(snap_dir.join("other.txt").exists(), "non-snapshot file kept");
    }

    #[test]
    fn test_header_roundtrip() {
        let header =
            SnapshotHeader { file_size: 42, mtime_secs: 1234567890, baseline_revision: None };
        let lines = header.to_header_lines();
        let restored = SnapshotHeader::from_header_lines(&lines).unwrap();
        assert_eq!(restored.file_size, 42);
        assert_eq!(restored.mtime_secs, 1234567890);
    }

    #[test]
    fn test_snapshot_persists_exact_disk_revision() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "baseline\n").expect("file should be written");
        let baseline =
            crate::file_safety::capture_revision(&path).expect("revision should capture");
        let original = vec!["baseline".to_string()];
        let current = vec!["local edit".to_string()];

        write_snapshot_with_revision(
            directory.path(),
            "notes.dirty",
            &baseline,
            &original,
            &current,
        )
        .expect("snapshot should be written");

        let (_, header) = read_and_apply(directory.path(), "notes.dirty", &["remote".to_string()])
            .expect("snapshot should be readable");
        let persisted = header.baseline_revision.expect("baseline should be persisted");
        let restored =
            persisted.to_disk_revision(&path).expect("persisted baseline should be valid");
        assert_eq!(restored, baseline);
    }

    #[test]
    fn test_empty_diff_no_hunks() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let lines: Vec<String> = vec!["same".to_string()];
        write_snapshot(snap_dir, "empty.dirty", 0, 0, &lines, &lines).expect("write");

        let (restored, _) = read_and_apply(snap_dir, "empty.dirty", &lines).expect("read");
        assert_eq!(restored, lines);
    }

    #[test]
    fn test_full_insert_from_empty() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let empty: Vec<String> = Vec::new();
        let content: Vec<String> = vec!["new line 1".to_string(), "new line 2".to_string()];

        write_snapshot(snap_dir, "insert.dirty", 0, 0, &empty, &content).expect("write");
        let (restored, _) = read_and_apply(snap_dir, "insert.dirty", &empty).expect("read");
        assert_eq!(restored, content);
    }

    #[test]
    fn test_full_delete_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let original: Vec<String> = vec!["to delete".to_string()];
        let empty: Vec<String> = Vec::new();

        write_snapshot(snap_dir, "delete.dirty", 0, 0, &original, &empty).expect("write");
        let (restored, _) = read_and_apply(snap_dir, "delete.dirty", &original).expect("read");
        assert_eq!(restored, empty);
    }

    #[test]
    fn test_multiple_hunks() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let original: Vec<String> = (0..50).map(|i| format!("line {}", i)).collect();
        let mut current = original.clone();
        current[5] = "modified 5".to_string();
        current[25] = "modified 25".to_string();
        current[45] = "modified 45".to_string();

        write_snapshot(snap_dir, "multi.dirty", 0, 0, &original, &current).expect("write");
        let (restored, _) = read_and_apply(snap_dir, "multi.dirty", &original).expect("read");
        assert_eq!(restored, current);
    }

    #[test]
    fn test_read_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = read_and_apply(dir.path(), "missing.dirty", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_invalid_header() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.dirty"), "not a valid snapshot\n").unwrap();
        let result = read_and_apply(dir.path(), "bad.dirty", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cjk_content_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let original: Vec<String> =
            vec!["你好世界".to_string(), "こんにちは".to_string(), "hello".to_string()];
        let current: Vec<String> = vec![
            "你好世界".to_string(),
            "修改后的日本語".to_string(),
            "hello".to_string(),
            "新增的中文行".to_string(),
        ];

        write_snapshot(snap_dir, "cjk.dirty", 0, 0, &original, &current).unwrap();
        let (restored, _) = read_and_apply(snap_dir, "cjk.dirty", &original).unwrap();
        assert_eq!(restored, current);
    }

    #[test]
    fn test_special_chars_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let snap_dir = dir.path();

        let original: Vec<String> =
            vec!["line with\ttabs".to_string(), "line with \\backslash".to_string()];
        let current: Vec<String> = vec![
            "line with\ttabs".to_string(),
            "replaced \\backslash".to_string(),
            "new \"quoted\" line".to_string(),
        ];

        write_snapshot(snap_dir, "special.dirty", 0, 0, &original, &current).unwrap();
        let (restored, _) = read_and_apply(snap_dir, "special.dirty", &original).unwrap();
        assert_eq!(restored, current);
    }

    #[test]
    fn test_snapshot_id_for_untitled_reuses_existing() {
        let f1 = snapshot_id_for_untitled(Some("existing_id"));
        let f2 = snapshot_id_for_untitled(Some("existing_id"));
        assert_eq!(f1, f2);
        assert_eq!(f1, "existing_id.dirty");
    }

    #[test]
    fn test_snapshot_id_for_untitled_generates_new() {
        let f1 = snapshot_id_for_untitled(None);
        let f2 = snapshot_id_for_untitled(None);
        assert_ne!(f1, f2);
        assert!(f1.starts_with("untitled_"));
        assert!(f1.ends_with(".dirty"));
    }

    #[test]
    fn test_write_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");

        let original: Vec<String> = vec!["old".to_string()];
        let current: Vec<String> = vec!["new".to_string()];

        write_snapshot(&nested, "test.dirty", 0, 0, &original, &current).unwrap();
        assert!(nested.join("test.dirty").exists());
    }

    #[test]
    fn test_cleanup_orphans_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let empty = HashSet::new();
        // Should not panic on empty directory
        cleanup_orphans(dir.path(), &empty);
    }

    #[test]
    fn test_header_invalid_magic() {
        let lines = vec!["magic: WRONG".to_string(), "version: 1".to_string()];
        assert!(SnapshotHeader::from_header_lines(&lines).is_none());
    }
}
