# Hot Exit: Diff-Based Dirty Snapshot — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace inline `unsaved_lines` in `workspace.toml` with separate diff-based snapshot files, using unified diff format.

**Architecture:** New `dirty_snapshot` module handles diff computation (`similar`), unified-diff read/write, and snapshot file lifecycle. `workspace.rs` delegates save/restore to it. Snapshot filenames are deterministic from file path for named tabs, or a pre-generated ID for untitled tabs.

**Tech Stack:** Rust, `similar` crate (diff engine), `std::hash::DefaultHasher` (path → id), plain-text unified diff (no JSON).

---

### Task 1: Add `similar` dependency

**Files:**
- Modify: `crates/app/Cargo.toml`

- [ ] **Step 1: Add `similar` to Cargo.toml**

```toml
similar = "2"
```

Add it under the existing dependencies. Full context:

```toml
rfd = "0.15"
smallvec = { workspace = true }
similar = "2"
```

- [ ] **Step 2: Build to verify dependency resolves**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: no errors (just the new dep downloaded)

- [ ] **Step 3: Commit**

```bash
git add crates/app/Cargo.toml crates/app/Cargo.lock
git commit -m "deps: add similar crate for diff-based dirty snapshots"
```

---

### Task 2: Create `dirty_snapshot.rs` — path helpers and types

**Files:**
- Create: `crates/app/src/dirty_snapshot.rs`
- Modify: `crates/app/src/lib.rs` (or `main.rs`) — add `mod dirty_snapshot;`

- [ ] **Step 1: Check where to declare the module**

Run: `grep -n '^mod ' /Users/dan/proj/llmws/edit+/crates/app/src/main.rs | head -5`

- [ ] **Step 2: Create `crates/app/src/dirty_snapshot.rs` with path helpers**

```rust
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Directory where per-tab dirty snapshot files are stored.
pub(crate) fn snapshots_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".edit+").join("snapshots")
}

/// Deterministic filename-safe identifier derived from a file path.
/// Same path always produces the same id (within one binary version).
pub(crate) fn path_id(path: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Generate a unique snapshot id for an untitled tab.
pub(crate) fn untitled_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("untitled_{:x}_{:x}", ts, n)
}
```

- [ ] **Step 3: Add snapshot filename extension helper**

Append to `dirty_snapshot.rs`:

```rust
pub(crate) const SNAPSHOT_EXT: &str = "dirty";

pub(crate) fn snapshot_filename(id: &str) -> String {
    format!("{}.{}", id, SNAPSHOT_EXT)
}
```

- [ ] **Step 4: Declare the module**

Add to `crates/app/src/lib.rs` after the existing `mod` declarations (after `pub mod document_view;`):

```rust
pub(crate) mod dirty_snapshot;
```

- [ ] **Step 5: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles cleanly

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/dirty_snapshot.rs crates/app/src/lib.rs
git commit -m "feat: add dirty_snapshot module with path helpers"
```

---

### Task 3: Implement unified diff write (`write_snapshot`)

**Files:**
- Modify: `crates/app/src/dirty_snapshot.rs`

- [ ] **Step 1: Add internal hunk types and `group_into_hunks`**

Append to `dirty_snapshot.rs`:

```rust
use similar::{ChangeTag, TextDiff};

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
fn group_into_hunks(diff: &TextDiff<'_, '_, '_>) -> Vec<Hunk> {
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
                match change.tag() {
                    ChangeTag::Equal => {
                        old_count += 1;
                        new_count += 1;
                        hunk_lines.push(HunkLine::Context(change.value().to_string()));
                    }
                    ChangeTag::Delete => {
                        old_count += 1;
                        hunk_lines.push(HunkLine::Delete(change.value().to_string()));
                    }
                    ChangeTag::Insert => {
                        new_count += 1;
                        hunk_lines.push(HunkLine::Insert(change.value().to_string()));
                    }
                }
            }
        }

        hunks.push(Hunk { old_start, old_count, new_start, new_count, lines: hunk_lines });
    }

    hunks
}
```

- [ ] **Step 2: Add `write_snapshot` function**

Append:

```rust
use std::io::Write;

/// Write a dirty snapshot to `dir/filename` as unified diff.
/// Uses atomic write (temp file → rename).
pub(crate) fn write_snapshot(
    dir: &Path,
    filename: &str,
    original_size: u64,
    original_mtime: i64,
    original: &[String],
    current: &[String],
) -> std::io::Result<()> {
    let diff = TextDiff::from_lines(original, current);
    let hunks = group_into_hunks(&diff);

    let temp_path = dir.join(format!(".{}.tmp", filename));
    let final_path = dir.join(filename);

    let _ = std::fs::create_dir_all(dir);

    let mut f = std::fs::File::create(&temp_path)?;
    writeln!(f, "# original_size: {}", original_size)?;
    writeln!(f, "# original_mtime: {}", original_mtime)?;

    for hunk in &hunks {
        writeln!(
            f,
            "@@ -{},{} +{},{} @@",
            hunk.old_start + 1, // unified diff uses 1-based line numbers
            hunk.old_count,
            hunk.new_start + 1,
            hunk.new_count,
        )?;
        for line in &hunk.lines {
            match line {
                HunkLine::Context(s) => {
                    if s.is_empty() {
                        writeln!(f, " ")?;
                    } else {
                        writeln!(f, " {}", s)?;
                    }
                }
                HunkLine::Delete(s) => {
                    if s.is_empty() {
                        writeln!(f, "-")?;
                    } else {
                        writeln!(f, "-{}", s)?;
                    }
                }
                HunkLine::Insert(s) => {
                    if s.is_empty() {
                        writeln!(f, "+")?;
                    } else {
                        writeln!(f, "+{}", s)?;
                    }
                }
            }
        }
    }

    f.sync_all()?;
    std::fs::rename(&temp_path, &final_path)?;

    Ok(())
}
```

Note: `similar` will strip trailing newlines from lines, so a line that was `""` in the editor becomes an empty string. The special-case handling for empty strings ensures the ` `/`-`/`+` prefix is still written followed by nothing, producing valid unified diff output.

- [ ] **Step 3: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/dirty_snapshot.rs
git commit -m "feat: implement unified diff write in dirty_snapshot"
```

---

### Task 4: Implement unified diff parse and apply (`read_and_apply`)

**Files:**
- Modify: `crates/app/src/dirty_snapshot.rs`

- [ ] **Step 1: Add `SnapshotHeader` struct and `parse_snapshot_header`**

Append to `dirty_snapshot.rs`:

```rust
#[derive(Debug, Clone, Copy)]
pub(crate) struct SnapshotHeader {
    pub(crate) original_size: u64,
    pub(crate) original_mtime: i64,
}
```

- [ ] **Step 2: Add hunk parser helper**

Append:

```rust
/// Parse a unified diff hunk header: "@@ -old_start,old_count +new_start,new_count @@"
fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let line = line.strip_prefix("@@ ")?.strip_suffix(" @@")?;
    let (old_part, new_part) = line.split_once(' ')?;
    let old_part = old_part.strip_prefix('-')?;
    let new_part = new_part.strip_prefix('+')?;

    let (old_start_str, old_count_str) = old_part.split_once(',')?;
    let (new_start_str, new_count_str) = new_part.split_once(',')?;

    let old_start: usize = old_start_str.parse().ok()?;
    let old_count: usize = old_count_str.parse().ok()?;
    let new_start: usize = new_start_str.parse().ok()?;
    let new_count: usize = new_count_str.parse().ok()?;

    // Convert from 1-based (unified diff) to 0-based
    Some((old_start.saturating_sub(1), old_count, new_start.saturating_sub(1), new_count))
}
```

- [ ] **Step 3: Add `read_and_apply` function**

Append:

```rust
/// Read a snapshot file, parse unified diff hunks, and apply them to `original` lines.
/// Returns reconstructed lines and the snapshot header metadata.
pub(crate) fn read_and_apply(
    dir: &Path,
    filename: &str,
    original: &[String],
) -> Result<(Vec<String>, SnapshotHeader), String> {
    let path = dir.join(filename);
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("read snapshot {}: {e}", path.display()))?;

    let mut original_size: u64 = 0;
    let mut original_mtime: i64 = 0;
    let mut hunks: Vec<Hunk> = Vec::new();

    let raw_lines: Vec<&str> = content.lines().collect();
    let mut i = 0usize;

    // Parse header comments
    while i < raw_lines.len() && raw_lines[i].starts_with('#') {
        let line = raw_lines[i];
        if let Some(rest) = line.strip_prefix("# original_size: ") {
            original_size = rest.parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("# original_mtime: ") {
            original_mtime = rest.parse().unwrap_or(0);
        }
        i += 1;
    }

    // Parse hunks
    while i < raw_lines.len() {
        if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(raw_lines[i]) {
            i += 1;
            let mut hunk_lines = Vec::new();
            let mut seen_old = 0usize;
            let mut seen_new = 0usize;

            while i < raw_lines.len()
                && !raw_lines[i].starts_with("@@")
                && !raw_lines[i].starts_with('#')
            {
            {
                let line = raw_lines[i];
                if line.is_empty() {
                    // Bare line with no prefix → treat as context (empty line)
                    hunk_lines.push(HunkLine::Context(String::new()));
                    seen_old += 1;
                    seen_new += 1;
                } else if let Some(rest) = line.strip_prefix(' ') {
                    hunk_lines.push(HunkLine::Context(rest.to_string()));
                    seen_old += 1;
                    seen_new += 1;
                } else if let Some(rest) = line.strip_prefix('-') {
                    hunk_lines.push(HunkLine::Delete(rest.to_string()));
                    seen_old += 1;
                } else if let Some(rest) = line.strip_prefix('+') {
                    hunk_lines.push(HunkLine::Insert(rest.to_string()));
                    seen_new += 1;
                }
                i += 1;
            }

            hunks.push(Hunk { old_start, old_count, new_start, new_count, lines: hunk_lines });
        } else {
            i += 1;
        }
    }

    // Apply hunks
    let result = apply_hunks(original, &hunks);

    Ok((result, SnapshotHeader { original_size, original_mtime }))
}

/// Apply parsed hunks to original lines, producing the reconstructed content.
fn apply_hunks(original: &[String], hunks: &[Hunk]) -> Vec<String> {
    let mut result = Vec::new();
    let mut orig_pos = 0usize;

    for hunk in hunks {
        // Copy lines from original before this hunk's start
        while orig_pos < hunk.old_start && orig_pos < original.len() {
            result.push(original[orig_pos].clone());
            orig_pos += 1;
        }

        // Apply hunk lines
        for line in &hunk.lines {
            match line {
                HunkLine::Context(s) => {
                    result.push(s.clone());
                    orig_pos += 1;
                }
                HunkLine::Delete(_) => {
                    orig_pos += 1;
                }
                HunkLine::Insert(s) => {
                    result.push(s.clone());
                }
            }
        }
    }

    // Copy remaining lines after last hunk
    while orig_pos < original.len() {
        result.push(original[orig_pos].clone());
        orig_pos += 1;
    }

    result
}
```

- [ ] **Step 4: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/dirty_snapshot.rs
git commit -m "feat: implement unified diff parse and apply in dirty_snapshot"
```

---

### Task 5: Implement `delete_snapshot` and `cleanup_orphans`

**Files:**
- Modify: `crates/app/src/dirty_snapshot.rs`

- [ ] **Step 1: Add delete and cleanup functions**

Append to `dirty_snapshot.rs`:

```rust
use std::collections::HashSet;

/// Delete a single snapshot file. No-op if the file doesn't exist.
pub(crate) fn delete_snapshot(dir: &Path, filename: &str) {
    let path = dir.join(filename);
    let _ = std::fs::remove_file(&path);
}

/// Delete `.dirty` files in `dir` that are not in the `active` set.
pub(crate) fn cleanup_orphans(dir: &Path, active: &HashSet<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(&format!(".{}", SNAPSHOT_EXT)) && !active.contains(name_str.as_ref()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
```

- [ ] **Step 2: Build check and commit**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles

```bash
git add crates/app/src/dirty_snapshot.rs
git commit -m "feat: add snapshot deletion and orphan cleanup"
```

---

### Task 6: Add `dirty_snapshot_id` field to `DocumentView`

**Files:**
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/workspace.rs` (tab creation sites)

- [ ] **Step 1: Add field to DocumentView struct**

In `crates/app/src/document_view/mod.rs`, after `pub dirty: bool` (line 56), add:

```rust
    /// Snapshot id for dirty untitled tabs. None for tabs with file_path
    /// (which use path-derived ids). Generated once at tab creation.
    pub(crate) dirty_snapshot_id: Option<String>,
```

- [ ] **Step 2: Initialize in `DocumentView::new()`**

In the `Self { .. }` constructor block of `new()` (around line 94), add:

```rust
            dirty_snapshot_id: None,
```

After the `dirty: false,` line.

- [ ] **Step 3: Generate id in `Workspace::new_empty_tab()`**

In `workspace.rs`, find `new_empty_tab()`. After creating the DocumentView, set the snapshot id. The line after `let idx = self.doc_views.len();` / `self.doc_views.push(dv);` — add before push:

```rust
        let (visible_rows, viewport_height) = Settings::with(|settings| {
            (settings.visible_rows(screen_height, 32.0 * settings.dpi_scale),
             settings.visible_height_lines(screen_height, 32.0 * settings.dpi_scale))
        });
        let mut dv = DocumentView::new(vec![String::new()], visible_rows, viewport_height);
        dv.dirty_snapshot_id = Some(crate::dirty_snapshot::untitled_id());
        self.record_nav_step();
        let idx = self.doc_views.len();
        self.doc_views.push(dv);
```

- [ ] **Step 4: Clear id on `save_as`**

In `document_view/mod.rs`, in the `save_as()` method (around line 222), after `self.dirty = false;` add:

```rust
        self.dirty_snapshot_id = None; // now uses path_id derived from file_path
```

- [ ] **Step 5: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/document_view/mod.rs crates/app/src/workspace.rs
git commit -m "feat: add dirty_snapshot_id field to DocumentView"
```

---

### Task 7: Modify `PersistedTab` and `WORKSPACE_VERSION`

**Files:**
- Modify: `crates/app/src/workspace.rs`

- [ ] **Step 1: Update `PersistedTab` struct**

Replace `unsaved_lines` with `dirty_snapshot`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTab {
    file_path: Option<PathBuf>,
    cursor_offset: usize,
    selection_anchor: Option<usize>,
    dirty: bool,
    #[serde(default)]
    scroll_anchor_line: Option<usize>,
    #[serde(default)]
    scroll_anchor_offset: Option<f32>,
    #[serde(default)]
    dirty_snapshot: Option<String>, // snapshot filename for untitled dirty tabs
}
```

- [ ] **Step 2: Bump `WORKSPACE_VERSION`**

Change `const WORKSPACE_VERSION: u32 = 1;` to `const WORKSPACE_VERSION: u32 = 2;`

- [ ] **Step 3: Build check and commit**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: errors in `save_snapshot` and `load_snapshot` (references to `unsaved_lines`) — expected, will fix in next tasks.

```bash
git add crates/app/src/workspace.rs
git commit -m "refactor: replace unsaved_lines with dirty_snapshot in PersistedTab"
```

---

### Task 8: Modify `save_snapshot()` in workspace.rs

**Files:**
- Modify: `crates/app/src/workspace.rs` — `save_snapshot()` method

- [ ] **Step 1: Rewrite the tab iteration block in `save_snapshot()`**

Replace the current `.map(|dv| { ... })` block (lines 423-449) with:

```rust
        let snapshots_dir = crate::dirty_snapshot::snapshots_dir();
        let tabs: Vec<PersistedTab> = self
            .doc_views
            .iter()
            .map(|dv| {
                let cursor_offset = dv.cursor().snapshot_offset.unwrap_or(dv.cursor().offset);
                let selection_anchor =
                    dv.cursor().snapshot_selection_anchor.unwrap_or(dv.cursor().selection_anchor);

                let dirty_snapshot: Option<String> = if dv.dirty {
                    if let Some(ref file_path) = dv.file_path {
                        // Named file: diff mode
                        let id = crate::dirty_snapshot::path_id(file_path);
                        let filename = crate::dirty_snapshot::snapshot_filename(&id);

                        // Read original file from disk
                        let original_lines: Vec<String> =
                            match std::fs::read_to_string(file_path) {
                                Ok(content) => {
                                    if content.is_empty() {
                                        Vec::new()
                                    } else {
                                        content.lines().map(|s| s.to_string()).collect()
                                    }
                                }
                                Err(_) => Vec::new(), // file deleted → treat as empty
                            };

                        let meta = std::fs::metadata(file_path).ok();
                        let original_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let original_mtime = meta
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);

                        // Build current lines from buffer
                        let current_lines: Vec<String> = (0..dv.line_count())
                            .filter_map(|i| dv.doc_line_bytes(i))
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                            .collect();

                        if let Err(e) = crate::dirty_snapshot::write_snapshot(
                            &snapshots_dir,
                            &filename,
                            original_size,
                            original_mtime,
                            &original_lines,
                            &current_lines,
                        ) {
                            eprintln!("[workspace] write snapshot failed: {e}");
                        }

                        None // no dirty_snapshot field needed (deterministic from path)
                    } else {
                        // Untitled: full content mode
                        let id = dv.dirty_snapshot_id.clone().unwrap_or_else(|| {
                            crate::dirty_snapshot::untitled_id()
                        });
                        let filename = crate::dirty_snapshot::snapshot_filename(&id);

                        let current_lines: Vec<String> = (0..dv.line_count())
                            .filter_map(|i| dv.doc_line_bytes(i))
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                            .collect();

                        if let Err(e) = crate::dirty_snapshot::write_snapshot(
                            &snapshots_dir,
                            &filename,
                            0, // original_size: 0 = no baseline
                            0, // original_mtime: 0
                            &[], // empty original
                            &current_lines,
                        ) {
                            eprintln!("[workspace] write snapshot failed: {e}");
                        }

                        Some(filename)
                    }
                } else {
                    // Clean tab: no snapshot needed
                    None
                };

                PersistedTab {
                    file_path: dv.file_path.clone(),
                    cursor_offset,
                    selection_anchor,
                    dirty: dv.dirty,
                    scroll_anchor_line: Some(dv.display.viewport.scroll_anchor.doc_line),
                    scroll_anchor_offset: Some(dv.display.viewport.scroll_anchor.pixel_offset),
                    dirty_snapshot,
                }
            })
            .collect();
```

- [ ] **Step 2: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles (load_snapshot still references unsaved_lines → will be errors; fix in next task)

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/workspace.rs
git commit -m "feat: rewrite save_snapshot to use diff-based dirty snapshots"
```

---

### Task 9: Modify `load_snapshot()` in workspace.rs

**Files:**
- Modify: `crates/app/src/workspace.rs` — `load_snapshot()` method

- [ ] **Step 1: Replace the tab restoration block in `load_snapshot()`**

Replace the `if let Some(ref lines) = ts.unsaved_lines { ... } else if ...` block (lines 528-557) with:

```rust
            let mut dv = if ts.dirty {
                let snapshots_dir = crate::dirty_snapshot::snapshots_dir();

                if let Some(ref file_path) = ts.file_path {
                    // Dirty tab with file path: try to restore from diff snapshot
                    let id = crate::dirty_snapshot::path_id(file_path);
                    let filename = crate::dirty_snapshot::snapshot_filename(&id);

                    // Read original file from disk
                    let original_lines: Vec<String> =
                        match std::fs::read_to_string(file_path) {
                            Ok(content) => {
                                if content.is_empty() {
                                    Vec::new()
                                } else {
                                    content.lines().map(|s| s.to_string()).collect()
                                }
                            }
                            Err(_) => Vec::new(),
                        };

                    // Check for external modification
                    let snapshot_exists = snapshots_dir.join(&filename).exists();
                    let external_change = if snapshot_exists {
                        let meta = std::fs::metadata(file_path).ok();
                        let current_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let current_mtime = meta
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);

                        // Read just the header to get stored mtime/size
                        let snapshot_path = snapshots_dir.join(&filename);
                        match std::fs::read_to_string(&snapshot_path) {
                            Ok(content) => {
                                let mut stored_size: u64 = 0;
                                let mut stored_mtime: i64 = 0;
                                for line in content.lines() {
                                    if !line.starts_with('#') { break; }
                                    if let Some(rest) = line.strip_prefix("# original_size: ") {
                                        stored_size = rest.parse().unwrap_or(0);
                                    } else if let Some(rest) = line.strip_prefix("# original_mtime: ") {
                                        stored_mtime = rest.parse().unwrap_or(0);
                                    }
                                }
                                current_size != stored_size || current_mtime != stored_mtime
                            }
                            Err(_) => true,
                        }
                    } else {
                        true // no snapshot = can't restore, same as external change
                    };

                    if snapshot_exists && !external_change {
                        // Restore from diff
                        match crate::dirty_snapshot::read_and_apply(
                            &snapshots_dir,
                            &filename,
                            &original_lines,
                        ) {
                            Ok((lines, _header)) => {
                                let mut dv = DocumentView::new(lines, visible_rows, viewport_height);
                                dv.file_path = Some(file_path.clone());
                                dv
                            }
                            Err(e) => {
                                eprintln!("[workspace] failed to apply snapshot: {e}");
                                // Fall through to load from disk as clean
                                match DocumentView::from_file(file_path, visible_rows, viewport_height) {
                                    Ok(dv) => dv,
                                    Err(_) => {
                                        let mut stub = DocumentView::new(vec![String::new()], visible_rows, viewport_height);
                                        stub.file_path = Some(file_path.clone());
                                        stub
                                    }
                                }
                            }
                        }
                    } else {
                        // External modification or no snapshot: discard diff, load clean
                        if snapshot_exists {
                            crate::dirty_snapshot::delete_snapshot(&snapshots_dir, &filename);
                        }
                        let mut dv = match DocumentView::from_file(file_path, visible_rows, viewport_height) {
                            Ok(dv) => dv,
                            Err(_) => {
                                let mut stub = DocumentView::new(vec![String::new()], visible_rows, viewport_height);
                                stub.file_path = Some(file_path.clone());
                                stub
                            }
                        };
                        dv.dirty = false; // external change → not our dirty state
                        dv
                    }
                } else if let Some(ref snapshot_name) = ts.dirty_snapshot {
                    // Untitled dirty tab: restore full content from snapshot
                    let snapshot_path = snapshots_dir.join(snapshot_name);
                    if snapshot_path.exists() {
                        // Read original_size: 0 snapshot (full content)
                        let empty_original: Vec<String> = Vec::new();
                        match crate::dirty_snapshot::read_and_apply(
                            &snapshots_dir,
                            snapshot_name,
                            &empty_original,
                        ) {
                            Ok((lines, _header)) => {
                                let mut dv = DocumentView::new(lines, visible_rows, viewport_height);
                                dv.dirty_snapshot_id = Some(
                                    snapshot_name
                                        .strip_suffix(&format!(".{}", crate::dirty_snapshot::SNAPSHOT_EXT))
                                        .unwrap_or(snapshot_name)
                                        .to_string(),
                                );
                                dv
                            }
                            Err(_) => DocumentView::new(vec![String::new()], visible_rows, viewport_height),
                        }
                    } else {
                        DocumentView::new(vec![String::new()], visible_rows, viewport_height)
                    }
                } else if let Some(ref file_path) = ts.file_path {
                    // Dirty but no snapshot: load from disk, keep dirty flag
                    match DocumentView::from_file(file_path, visible_rows, viewport_height) {
                        Ok(mut dv) => {
                            dv.dirty = true;
                            dv
                        }
                        Err(_) => {
                            let mut stub = DocumentView::new(vec![String::new()], visible_rows, viewport_height);
                            stub.file_path = Some(file_path.clone());
                            stub
                        }
                    }
                } else {
                    DocumentView::new(vec![String::new()], visible_rows, viewport_height)
                }
            } else if let Some(ref path) = ts.file_path {
                // Clean tab with file path (existing logic)
                if is_active {
                    match DocumentView::from_file(path, visible_rows, viewport_height) {
                        Ok(dv) => dv,
                        Err(_) => {
                            let mut stub = DocumentView::new(vec![String::new()], visible_rows, viewport_height);
                            stub.file_path = Some(path.clone());
                            stub
                        }
                    }
                } else {
                    let mut stub = DocumentView::new(vec![String::new()], visible_rows, viewport_height);
                    stub.file_path = Some(path.clone());
                    stub
                }
            } else {
                DocumentView::new(vec![String::new()], visible_rows, viewport_height)
            };
```

- [ ] **Step 2: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/workspace.rs
git commit -m "feat: rewrite load_snapshot to restore from diff-based snapshots"
```

---

### Task 10: Wire up snapshot cleanup on tab save and close

**Files:**
- Modify: `crates/app/src/document_view/mod.rs` — `save_as()` method (line 209)
- Modify: `crates/app/src/workspace.rs` — `close_tab_inner()` method (line 288)

- [ ] **Step 1: Add snapshot cleanup to `save_as()`**

Current `save_as()` (lines 209-226):

```rust
    pub fn save_as(&mut self, path: &std::path::Path) -> Result<(), String> {
        // ... line ending / metadata setup ...
        core::file::save_file(buffer, path, &metadata)
            .map_err(|e| format!("save failed: {e}"))?;
        self.file_path = Some(path.to_path_buf());
        self.dirty = false;
        self.tb.mark_as_clean();
        Ok(())
    }
```

Add snapshot cleanup between `save_file()` and setting `file_path`:

```rust
    pub fn save_as(&mut self, path: &std::path::Path) -> Result<(), String> {
        let line_ending = if self.crlf {
            core::file::LineEnding::Crlf
        } else {
            core::file::LineEnding::Lf
        };
        let metadata = core::file::FileMetadata {
            line_ending,
            had_bom: self.had_bom,
        };
        let buffer = self.tb.gap_buffer();
        core::file::save_file(buffer, path, &metadata)
            .map_err(|e| format!("save failed: {e}"))?;

        // Delete old dirty snapshot — tab is now clean
        let snapshots_dir = crate::dirty_snapshot::snapshots_dir();
        if let Some(ref old_id) = self.dirty_snapshot_id {
            crate::dirty_snapshot::delete_snapshot(
                &snapshots_dir,
                &crate::dirty_snapshot::snapshot_filename(old_id),
            );
        }
        let new_id = crate::dirty_snapshot::path_id(path);
        crate::dirty_snapshot::delete_snapshot(
            &snapshots_dir,
            &crate::dirty_snapshot::snapshot_filename(&new_id),
        );

        self.file_path = Some(path.to_path_buf());
        self.dirty_snapshot_id = None;
        self.dirty = false;
        self.tb.mark_as_clean();
        Ok(())
    }
```

Note: `save()` calls `save_as()` so both paths are covered.

- [ ] **Step 2: Add snapshot cleanup to `close_tab_inner()`**

In `workspace.rs`, in `close_tab_inner()` (line 288), after `self.doc_views.remove(index);`, add:

```rust
        // Remove dirty snapshot file for the closed tab
        let snapshots_dir = crate::dirty_snapshot::snapshots_dir();
        if let Some(ref file_path) = dv.file_path {
            let id = crate::dirty_snapshot::path_id(file_path);
            crate::dirty_snapshot::delete_snapshot(
                &snapshots_dir,
                &crate::dirty_snapshot::snapshot_filename(&id),
            );
        }
        if let Some(ref snapshot_id) = dv.dirty_snapshot_id {
            crate::dirty_snapshot::delete_snapshot(
                &snapshots_dir,
                &crate::dirty_snapshot::snapshot_filename(snapshot_id),
            );
        }
```

- [ ] **Step 3: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/document_view/mod.rs crates/app/src/workspace.rs
git commit -m "feat: cleanup dirty snapshots on tab save and close"
```

---

### Task 11: Add orphan cleanup on startup

**Files:**
- Modify: `crates/app/src/app_window.rs` — `init_window()` where `load_snapshot` is called

- [ ] **Step 1: Find the startup path**

Run: `grep -n 'load_snapshot\|init_window' /Users/dan/proj/llmws/edit+/crates/app/src/app_window.rs`

- [ ] **Step 2: Add orphan cleanup after `load_snapshot`**

After the workspace is loaded (or after a fresh start), add:

```rust
        // Clean up orphaned snapshot files
        {
            let dir = crate::dirty_snapshot::snapshots_dir();
            let active: std::collections::HashSet<String> = self
                .workspace
                .doc_views
                .iter()
                .filter_map(|dv| {
                    if dv.dirty {
                        if let Some(ref path) = dv.file_path {
                            Some(crate::dirty_snapshot::snapshot_filename(
                                &crate::dirty_snapshot::path_id(path),
                            ))
                        } else if let Some(ref id) = dv.dirty_snapshot_id {
                            Some(crate::dirty_snapshot::snapshot_filename(id))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
            crate::dirty_snapshot::cleanup_orphans(&dir, &active);
        }
```

This should run right after `load_snapshot` restores the workspace, and also when starting fresh.

For the fresh-start path, `cleanup_orphans` with an empty set will delete all orphaned `.dirty` files (since no tabs reference any snapshots).

- [ ] **Step 3: Build check**

Run: `cargo check -p edit-plus-app 2>&1`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/app_window.rs
git commit -m "feat: cleanup orphaned dirty snapshots on startup"
```

---

### Task 12: Update and add tests

**Files:**
- Modify: `crates/app/src/workspace.rs` — test module

- [ ] **Step 1: Remove old `unsaved_lines` tests**

Delete these test functions:
- `test_dirty_tab_with_path_saves_unsaved_lines` (line 895)
- `test_dirty_tab_empty_content_saves_none_unsaved_lines` (line 950)
- `test_active_tab_prefers_unsaved_lines_over_file_path` (line 990)

- [ ] **Step 2: Add a helper function for test setup**

Add to the test module:

```rust
    fn setup_test_dirs() -> (TempDir, PathBuf) {
        Settings::init(Settings::new());
        let tmp = TempDir::new().expect("tempdir");
        let snap_dir = tmp.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).expect("create snapshots dir");
        (tmp, snap_dir)
    }

    fn dv_from_lines(lines: Vec<&str>) -> DocumentView {
        let string_lines: Vec<String> = lines.into_iter().map(|s| s.to_string()).collect();
        DocumentView::new(string_lines, 40, 600.0)
    }
```

- [ ] **Step 3: Add diff roundtrip test**

```rust
    #[test]
    fn test_diff_snapshot_roundtrip() {
        use crate::dirty_snapshot;

        let dir = TempDir::new().expect("tempdir");
        let snap_dir = dir.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).expect("mkdir");

        let original: Vec<String> = vec![
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
            "line4".to_string(),
            "line5".to_string(),
        ];

        let current: Vec<String> = vec![
            "line1".to_string(),
            "line2-modified".to_string(),
            "line3".to_string(),
            "line4".to_string(),
            "line5".to_string(),
        ];

        let filename = "test.dirty";
        dirty_snapshot::write_snapshot(&snap_dir, filename, 0, 0, &original, &current)
            .expect("write");

        let (restored, _header) =
            dirty_snapshot::read_and_apply(&snap_dir, filename, &original).expect("read");

        assert_eq!(restored, current);
    }
```

- [ ] **Step 4: Add untitled snapshot roundtrip test**

```rust
    #[test]
    fn test_untitled_snapshot_roundtrip() {
        use crate::dirty_snapshot;

        let dir = TempDir::new().expect("tempdir");
        let snap_dir = dir.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).expect("mkdir");

        let empty: Vec<String> = Vec::new();
        let content: Vec<String> = vec![
            "hello world".to_string(),
            "".to_string(),
            "foo bar".to_string(),
        ];

        let filename = "untitled_test.dirty";
        dirty_snapshot::write_snapshot(&snap_dir, filename, 0, 0, &empty, &content)
            .expect("write");

        let (restored, _header) =
            dirty_snapshot::read_and_apply(&snap_dir, filename, &empty).expect("read");

        assert_eq!(restored, content);
    }
```

- [ ] **Step 5: Add external-modification detection test**

```rust
    #[test]
    fn test_external_modification_detected() {
        use crate::dirty_snapshot;

        let tmp = TempDir::new().expect("tempdir");

        // Create original file
        let file_path = tmp.path().join("real.txt");
        std::fs::write(&file_path, "original\ncontent\n").expect("write");

        // Create a snapshot
        let snap_dir = tmp.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).expect("mkdir");

        let original_lines: Vec<String> = vec!["original".to_string(), "content".to_string()];
        let modified_lines: Vec<String> = vec!["original".to_string(), "modified".to_string()];

        let meta = std::fs::metadata(&file_path).expect("metadata");
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let size = meta.len();

        dirty_snapshot::write_snapshot(
            &snap_dir, "test.dirty", size, mtime,
            &original_lines, &modified_lines,
        ).expect("write");

        // Now modify the file externally
        std::fs::write(&file_path, "changed\nexternally\n").expect("overwrite");
        let new_meta = std::fs::metadata(&file_path).expect("metadata");
        let new_mtime = new_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let new_size = new_meta.len();

        // mtime or size should differ
        assert!(
            new_size != size || new_mtime != mtime,
            "external modification should change mtime or size"
        );
    }
```

- [ ] **Step 6: Add orphan cleanup test**

```rust
    #[test]
    fn test_orphan_cleanup() {
        use crate::dirty_snapshot;
        use std::collections::HashSet;

        let dir = TempDir::new().expect("tempdir");
        let snap_dir = dir.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).expect("mkdir");

        // Create two snapshot files
        std::fs::write(snap_dir.join("keep.dirty"), b"content").expect("write");
        std::fs::write(snap_dir.join("orphan.dirty"), b"content").expect("write");

        // Active set only includes "keep.dirty"
        let mut active = HashSet::new();
        active.insert("keep.dirty".to_string());

        dirty_snapshot::cleanup_orphans(&snap_dir, &active);

        assert!(snap_dir.join("keep.dirty").exists(), "referenced file kept");
        assert!(!snap_dir.join("orphan.dirty").exists(), "orphan deleted");
    }
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p edit-plus-app -- workspace::tests 2>&1`
Expected: all tests pass

- [ ] **Step 8: Run full test suite**

Run: `cargo test -p edit-plus-app 2>&1`
Expected: all tests pass

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/workspace.rs
git commit -m "test: update workspace tests for diff-based snapshots"
```

---

### Verification

- [ ] **Step 1: Build release**

Run: `cargo build -p edit-plus-app --release 2>&1`
Expected: builds successfully

- [ ] **Step 2: Manual smoke test**

1. Open the app, create a new untitled tab, type some text
2. Open a file, make edits
3. Switch tabs — verify no crash
4. Quit and restart — verify dirty tabs restore correctly
5. Modify a file externally while editor is closed — verify restart loads clean version

- [ ] **Step 3: Clean up old workspace.toml (if testing locally)**

```bash
rm ~/.edit+/workspace.toml
rm -rf ~/.edit+/snapshots/
```

