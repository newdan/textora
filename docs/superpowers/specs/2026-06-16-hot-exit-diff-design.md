# Hot Exit: Diff-Based Dirty Snapshot

## Summary

Replace `unsaved_lines` (full in-memory content serialized inline in `workspace.toml`) with diff-based dirty snapshots stored in separate files. For large files with small edits, the stored data goes from tens of MB to a few KB. For untitled tabs without a file path, full content is still stored, but in a separate file rather than inline in TOML.

## Storage Layout

```
~/.edit+/
  workspace.toml              ← tab metadata only
  snapshots/
    <path_id>.dirty           ← diff snapshot for dirty tab with file_path
    <uuid>.dirty              ← full-content snapshot for untitled dirty tab
```

- **Tabs with `file_path`**: snapshot filename is a deterministic identifier derived from the file path → `"{id}.dirty"` — no extra field needed in workspace.toml.
- **Untitled tabs** (no `file_path`): snapshot filename = a unique id → `"{id}.dirty"` — stored in `dirty_snapshot` field on `PersistedTab`.

## Data Model

### `PersistedTab` (workspace.rs)

```rust
struct PersistedTab {
    file_path: Option<PathBuf>,
    cursor_offset: usize,
    selection_anchor: Option<usize>,
    dirty: bool,
    scroll_anchor_line: Option<usize>,
    scroll_anchor_offset: Option<f32>,
    // REMOVED: unsaved_lines: Option<Vec<String>>
    // ADDED: only for untitled dirty tabs
    dirty_snapshot: Option<String>,  // UUID.dirty, None for tabs with file_path
}
```

### Snapshot File Format (unified diff)

For dirty tabs **with a file path** (diff mode):

```diff
# original_size: 1048576
# original_mtime: 1718400000
@@ -10,5 +10,7 @@
 unchanged line
 unchanged line
-fn foo() {
+fn bar() {
+    println!("added");
 }
 unchanged line
@@ -30,3 +30,2 @@
 context
-deleted line
 context
```

For dirty tabs **without a file path** (full mode, untitled):

```diff
# original_size: 0
# original_mtime: 0
@@ -0,0 +1,3 @@
+line one
+line two
+line three
```

First two `#` comment lines carry original file metadata for external-modification detection. A unified diff with `original_size: 0` means "full content" (no baseline file existed).

## Dependencies

- **`similar`** (v2.x): pure-Rust diff engine, computes line diffs between original file and in-memory content.
- Path → snapshot ID mapping uses `std::hash::DefaultHasher` (no extra dependency).
- Unified diff serialization is plain text; serde + toml are already present for workspace.toml.

## Save Flow

### `Workspace::save_snapshot()`

```
for each tab (index i, DocumentView dv):
    if dv.dirty && dv.file_path.is_some():
        let path_id = derive_id_from_path(&dv.file_path)
        let snapshot_path = "snapshots/{path_id}.dirty"

        // Read original file from disk
        let original = std::fs::read_to_string(&dv.file_path)
        if read fails → treat as empty original (or skip snapshot)

        // Compute diff
        let diff = similar::TextDiff::from_lines(&original_lines, &current_lines)

        // Write unified diff with metadata header
        write snapshot_path:
            # original_size: {metadata.len()}
            # original_mtime: {metadata.modified()}

        PersistedTab { dirty: true, .. }  // no dirty_snapshot field

    elif dv.dirty && dv.file_path.is_none():
        let uuid = generate_uuid()
        let snapshot_path = "snapshots/{uuid}.dirty"

        // Full content as unified diff against empty
        write snapshot_path with original_size: 0

        PersistedTab { dirty: true, dirty_snapshot: Some("{uuid}.dirty"), .. }

    else:
        // Clean tab: no snapshot file
        // If a snapshot file existed before (tab was dirty and got saved),
        // it was already deleted by the save() method.
        PersistedTab { dirty: false, dirty_snapshot: None, .. }
```

### Snapshot file write

Same atomic strategy as today: write to `.tmp` file, `sync_all()`, `rename`. Snapshot files go under `~/.edit+/snapshots/`.

## Restore Flow

### `Workspace::load_snapshot()`

```
for each tab in workspace.toml:
    if tab.dirty && tab.file_path.is_some():
        let path_id = derive_id_from_path(&tab.file_path)
        let snapshot_path = "snapshots/{path_id}.dirty"

        if snapshot file exists:
            parse metadata (# original_size, # original_mtime)
            stat original file from disk
            if size matches && mtime matches:
                read original file
                parse unified diff hunks
                apply hunks → lines
                build DocumentView from lines, mark dirty
            else:
                // External modification: discard diff
                delete snapshot_path
                load file from disk, mark clean
        else:
            // Snapshot missing (deleted externally?)
            load file from disk, mark dirty (preserve flag)

    elif tab.dirty && tab.dirty_snapshot.is_some():
        let snapshot_path = "snapshots/{tab.dirty_snapshot}"

        if snapshot file exists:
            read lines from snapshot (full content diff against empty)
            build DocumentView from lines, mark dirty
        else:
            create empty untitled DocumentView

    elif tab.file_path.is_some():
        load from disk (existing logic, unchanged)

    else:
        create empty untitled DocumentView (existing logic, unchanged)
```

### Unified diff apply

Parse `@@ -old_start,old_count +new_start,new_count @@` headers and line prefixes (` ` = context, `-` = delete, `+` = insert). Build result lines by walking old lines and applying each hunk in order. This reconstructs the exact in-memory state at snapshot time.

## Cleanup

| Trigger | Action |
|---------|--------|
| Tab closed (dirty) | Delete corresponding `.dirty` file |
| Tab saved (`save`/`save_as`) | `dirty` → `false`; delete corresponding `.dirty` file |
| App startup | Scan `snapshots/`, delete any `.dirty` file not referenced by `workspace.toml` (orphan cleanup) |

Snapshot file deletion uses the same deterministic naming: for tabs with `file_path`, derive `path_id`; for untitled tabs, use `dirty_snapshot` value.

## Per-Tab Persist Trigger (unchanged)

`save_snapshot` is called on:
- Tab switch
- Tab close
- Sidebar resize end
- Sidebar toggle
- App quit (`CloseRequested` + `quit_app`)

No change to the call sites. Diff is computed at persist time, not on every keystroke.

## Version

`WORKSPACE_VERSION` → `2`. Version 1 snapshots are silently ignored (load returns `None`, editor starts fresh).

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| Dirty tab, file deleted externally | `read_to_string` fails → treat as original_size=0 → store full content as diff against empty |
| Dirty tab, file modified externally between save_snapshot calls | Each persist overwrites the `.dirty` file with fresh diff against current disk state. If disk changed, the next diff picks it up naturally. |
| Dirty tab, file modified after snapshot but before restore | mtime/size mismatch → discard diff, load disk as clean |
| Large file (50MB+, 1M+ lines) | `similar` line diff is O(N) in line count, runs in ~100ms for 1M lines. Slight pause on tab switch/quit for such files, acceptable. |
| File path with special characters | Deterministic ID derived from path bytes → always a safe filename |
| Two tabs open on the same file | Not possible — `open_file` switches to existing tab via `find_by_path`, never creates a duplicate. |
| Snapshot dir missing on load | Treated as no snapshot → load from disk |
| Unicode / BOM in original file | `similar` works on lines (strings). BOM handling is the DocumentView loader's responsibility, diff just sees it as content. |
| CRLF files | Lines are already normalized by the time they reach diff computation (DocumentView joins with `\n`). Restored content goes through the same DocumentView constructor path. |

## Test Changes

All existing `unsaved_lines` tests in `workspace.rs` need replacement:
- `test_dirty_tab_with_path_saves_unsaved_lines` → tests diff generation + file write
- `test_active_tab_prefers_unsaved_lines_over_file_path` → tests diff apply + restore
- `test_dirty_tab_empty_content_saves_none_unsaved_lines` → tests empty dirty produces minimal diff

New tests:
- Diff roundtrip: modify lines → snapshot → restore → verify content
- External modification detection: change mtime → restore → verify diff discarded
- Untitled tab full-content snapshot roundtrip
- Orphan cleanup on startup
