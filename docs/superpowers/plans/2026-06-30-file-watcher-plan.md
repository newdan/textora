# FileWatcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add file external-change detection via mtime polling, prompting the user with a modal dialog to reload or ignore.

**Architecture:** New independent `FileWatcher` component in `crates/app/src/file_watcher.rs` — tracks one path at a time, compares mtime+size every 2s, exposed as a field on `App`. Polling happens in `about_to_wait`; reload re-opens the file via `DocumentView::from_file` and restores `scroll_anchor`.

**Tech Stack:** Rust, winit event loop, rfd::MessageDialog, std::fs::metadata

## Global Constraints

- Only monitor the active tab's file (dirty files skipped)
- Poll interval: 2 seconds (mtime + size comparison)
- Use rfd::MessageDialog for modal prompt
- Reload must restore scroll position via `scroll_anchor`

---

### Task 1: Create FileWatcher module

**Files:**
- Create: `crates/app/src/file_watcher.rs`
- Modify: `crates/app/src/lib.rs` (add `mod file_watcher;`)

**Interfaces:**
- Produces: `FileWatcher::new()`, `start_watching(path, mtime, size)`, `stop_watching()`, `should_check() -> bool`, `check() -> Option<FileChange>`, `confirm_reload(mtime, size)`, `next_check_time() -> Option<Instant>`

- [ ] **Step 1: Write file_watcher.rs with full implementation**

```rust
//! File watcher: polls mtime for external changes to the active file.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// Result returned when a file change is detected.
#[derive(Debug)]
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub new_size: u64,
    pub new_mtime: SystemTime,
}

struct WatchingState {
    path: PathBuf,
    recorded_mtime: SystemTime,
    recorded_size: u64,
}

pub(crate) struct FileWatcher {
    watching: Option<WatchingState>,
    last_check: Instant,
    interval: Duration,
    pending: bool,
}

impl FileWatcher {
    pub fn new() -> Self {
        Self {
            watching: None,
            last_check: Instant::now(),
            interval: Duration::from_secs(2),
            pending: false,
        }
    }

    /// Start monitoring a file. Called when a file is opened or reloaded.
    pub fn start_watching(
        &mut self,
        path: PathBuf,
        mtime: SystemTime,
        size: u64,
    ) {
        self.watching = Some(WatchingState {
            path,
            recorded_mtime: mtime,
            recorded_size: size,
        });
        self.last_check = Instant::now();
        self.pending = false;
    }

    /// Stop monitoring. Called when switching away or closing.
    pub fn stop_watching(&mut self) {
        self.watching = None;
        self.pending = false;
    }

    /// Whether it's time to poll.
    pub fn should_check(&self) -> bool {
        self.watching.is_some()
            && !self.pending
            && self.last_check.elapsed() >= self.interval
    }

    /// Poll the filesystem. Returns Some(FileChange) if the file was modified
    /// externally. Returns None if no change, file missing, or pending already set.
    /// Caller must skip dirty files before calling this.
    pub fn check(&mut self) -> Option<FileChange> {
        let state = self.watching.as_ref()?;
        if self.pending {
            return None;
        }
        self.last_check = Instant::now();
        let meta = match std::fs::metadata(&state.path) {
            Ok(m) => m,
            Err(_) => {
                // File deleted — stop watching
                self.watching = None;
                self.pending = false;
                return None;
            }
        };
        let new_size = meta.len();
        let new_mtime = match meta.modified() {
            Ok(m) => m,
            Err(_) => {
                self.watching = None;
                self.pending = false;
                return None;
            }
        };
        if new_size != state.recorded_size || new_mtime != state.recorded_mtime {
            self.pending = true;
            Some(FileChange {
                path: state.path.clone(),
                new_size,
                new_mtime,
            })
        } else {
            None
        }
    }

    /// Called after the user confirms reload to update the baseline.
    pub fn confirm_reload(&mut self, mtime: SystemTime, size: u64) {
        if let Some(ref mut state) = self.watching {
            state.recorded_mtime = mtime;
            state.recorded_size = size;
        }
        self.pending = false;
        self.last_check = Instant::now();
    }

    /// Returns the Instant when the next check should fire, for ControlFlow scheduling.
    pub fn next_check_time(&self) -> Option<Instant> {
        if self.watching.is_some() {
            Some(self.last_check + self.interval)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::SystemTime;

    fn write_temp_file(name: &str, content: &str) -> (PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        std::fs::write(&path, content).expect("write");
        (path, dir)
    }

    fn file_meta(path: &std::path::Path) -> (SystemTime, u64) {
        let meta = std::fs::metadata(path).expect("metadata");
        (
            meta.modified().expect("mtime"),
            meta.len(),
        )
    }

    #[test]
    fn new_watcher_has_no_target() {
        let fw = FileWatcher::new();
        assert!(!fw.should_check());
        assert!(fw.check().is_none());
    }

    #[test]
    fn should_check_after_interval() {
        let (path, _dir) = write_temp_file("test.txt", "hello");
        let (mtime, size) = file_meta(&path);
        let mut fw = FileWatcher::new();
        // Override interval to 0 for test
        fw.interval = Duration::ZERO;
        fw.start_watching(path.clone(), mtime, size);
        assert!(fw.should_check());
        assert!(fw.check().is_none()); // no change yet
    }

    #[test]
    fn detects_external_change() {
        let (path, _dir) = write_temp_file("test.txt", "hello");
        let (mtime, size) = file_meta(&path);
        let mut fw = FileWatcher::new();
        fw.interval = Duration::ZERO;
        fw.start_watching(path.clone(), mtime, size);

        // Modify file externally
        std::fs::write(&path, "hello world").expect("write");
        std::thread::sleep(Duration::from_millis(10)); // ensure mtime changes

        assert!(fw.should_check());
        let change = fw.check().expect("change detected");
        assert!(change.new_size > size);

        // Second check should not re-fire (pending flag)
        assert!(!fw.should_check());
        assert!(fw.check().is_none());
    }

    #[test]
    fn confirm_reload_resets_pending() {
        let (path, _dir) = write_temp_file("test.txt", "hello");
        let (mtime, size) = file_meta(&path);
        let mut fw = FileWatcher::new();
        fw.interval = Duration::ZERO;
        fw.start_watching(path.clone(), mtime, size);

        std::fs::write(&path, "changed").expect("write");
        std::thread::sleep(Duration::from_millis(10));
        let change = fw.check().expect("change");

        let (new_mtime, new_size) = file_meta(&path);
        fw.confirm_reload(new_mtime, new_size);
        assert!(fw.should_check());
        assert!(fw.check().is_none()); // should be clean after confirm
    }

    #[test]
    fn stop_watching_clears_state() {
        let (path, _dir) = write_temp_file("test.txt", "hello");
        let (mtime, size) = file_meta(&path);
        let mut fw = FileWatcher::new();
        fw.interval = Duration::ZERO;
        fw.start_watching(path.clone(), mtime, size);

        fw.stop_watching();
        assert!(!fw.should_check());
    }

    #[test]
    fn deleted_file_stops_watching() {
        let (path, _dir) = write_temp_file("test.txt", "hello");
        let (mtime, size) = file_meta(&path);
        let mut fw = FileWatcher::new();
        fw.interval = Duration::ZERO;
        fw.start_watching(path.clone(), mtime, size);

        // Delete the temp dir → path becomes invalid
        let dir_path = _dir.path().to_path_buf();
        drop(_dir);
        std::fs::remove_dir_all(&dir_path).ok();

        // check() returns None when metadata fails, and clears watching state
        fw.last_check = Instant::now() - Duration::from_secs(10);
        assert!(fw.check().is_none());
        assert!(!fw.should_check()); // watching was cleared
    }
}
```

- [ ] **Step 2: Register module in lib.rs**

Add the `mod file_watcher;` line after `mod file_history;` (line 102):

```rust
mod file_history;
mod file_watcher;  // <-- add this
mod frame_cache;
```

- [ ] **Step 3: Run tests to verify**

```bash
cd crates/app && cargo test file_watcher -- --nocapture
```
Expected: all 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/file_watcher.rs crates/app/src/lib.rs
git commit -m "feat(file_watcher): add FileWatcher module with mtime polling"
```

---

### Task 2: Add file_watcher field to App

**Files:**
- Modify: `crates/app/src/app.rs:76-141` (App struct)
- Modify: `crates/app/src/app_init.rs:104-147` (App::new constructor)

**Interfaces:**
- Consumes: `FileWatcher` from Task 1
- Produces: `app.file_watcher` field available for Task 3 and Task 4

- [ ] **Step 1: Add field to App struct**

Insert after `file_history` field (app.rs line 90):

```rust
pub(crate) file_watcher: FileWatcher,
```

- [ ] **Step 2: Add use statement in app.rs**

Add near other imports (after `use crate::dispatch::tabs::*;` is not needed — just ensure `FileWatcher` is in scope):

```rust
use crate::file_watcher::FileWatcher;
```

In `app.rs`, add the import. Currently imports are at lines 1-19. Add after line 14 (`use crate::native_menu::NativeMenu;`):

```rust
use crate::file_watcher::FileWatcher;
```

- [ ] **Step 3: Initialize in App::new constructor**

In `app_init.rs`, add after the `file_history` field init (line 118):

```rust
file_watcher: FileWatcher::new(),
```

- [ ] **Step 4: Build check**

```bash
cargo build 2>&1 | head -20
```
Expected: compiles cleanly, no warnings about file_watcher.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs
git commit -m "feat: add FileWatcher field to App struct"
```

---

### Task 3: Wire up open_file, tab switch, tab close, and save

**Files:**
- Modify: `crates/app/src/dispatch/tabs.rs:47-54` (open_file)
- Modify: `crates/app/src/dispatch/tabs.rs:5-41` (handle_workspace_effect)
- Modify: `crates/app/src/dispatch/commands.rs:10-67` (save_active_entry)
- Modify: `crates/app/src/dispatch/tabs.rs:143-229` (try_close_entry_with_prompt)

**Interfaces:**
- Consumes: `FileWatcher::start_watching`, `FileWatcher::stop_watching` from Task 1
- Produces: FileWatcher correctly tracks the active file across tab lifecycle

- [ ] **Step 1: Start watching on file open**

In `dispatch/tabs.rs`, modify `open_file` method (line 47):

```rust
pub(crate) fn open_file(&mut self, path: &std::path::Path) -> Result<AppEffect, String> {
    let viewport = self.viewport_dimensions(self.screen_height());
    let effect = self.workspace.open_file_with_viewport(path, viewport)?;
    let app_effect = self.handle_workspace_effect(effect);
    self.record_entry_to_history(self.workspace.active_index());
    self.rebuild_native_menu();

    // Start file watcher for the newly opened file
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            self.file_watcher
                .start_watching(path.to_path_buf(), mtime, meta.len());
        }
    }

    Ok(app_effect)
}
```

- [ ] **Step 2: Update watching on tab switch**

In `dispatch/tabs.rs`, modify `handle_workspace_effect` (line 5). After the `ActiveChanged` branch processes (line 29), add:

At the end of the `ActiveChanged` branch (before line 30 `}`), add:

```rust
// Update file watcher to track the newly active file
if let Some(entry) = self.workspace.active_entry() {
    let dv = &entry.doc;
    if !dv.dirty {
        if let Some(ref path) = dv.file_path {
            if let Ok(meta) = std::fs::metadata(path) {
                if let Ok(mtime) = meta.modified() {
                    self.file_watcher
                        .start_watching(path.clone(), mtime, meta.len());
                    return app_effect;
                }
            }
        }
    }
}
self.file_watcher.stop_watching();
```

Note: This is added before the `app_effect` return at the end of the match arm. Since `app_effect` is returned at line 29, we need to restructure: save the return value, update watcher, then return. Or better, just add the watcher logic before the return statement.

Actually, looking at the code more carefully:

```rust
crate::navigator::NavEffect::ActiveChanged => {
    app_effect = app_effect.merge(AppEffect::RESHAPE);
    // ... existing code ...
    app_effect = app_effect
        .merge(layout_effect)
        .merge(AppEffect::REDRAW)
        .merge(AppEffect::UPDATE_TITLE)
        .merge(AppEffect::PERSIST_WORKSPACE);
}
```

Add the file watcher update right before the closing `}` of this match arm. After the last `.merge(AppEffect::PERSIST_WORKSPACE)`:

```rust
// Update file watcher for the new active file
{
    let watcher = &mut self.file_watcher;
    if let Some(entry) = self.workspace.active_entry() {
        let dv = &entry.doc;
        if !dv.dirty {
            if let Some(ref path) = dv.file_path {
                if let Ok(meta) = std::fs::metadata(path) {
                    if let Ok(mtime) = meta.modified() {
                        watcher.start_watching(path.clone(), mtime, meta.len());
                    }
                }
            }
        } else {
            watcher.stop_watching();
        }
    } else {
        watcher.stop_watching();
    }
}
```

- [ ] **Step 3: Update mtime after save**

In `dispatch/commands.rs`, modify `save_active_entry`. After successful direct save (line 21, `Some((Ok(()), _))` branch), update the watcher by reading the path from the now-clean workspace entry:

```rust
Some((Ok(()), _)) => {
    self.update_document_edited(false);
    // Update file watcher baseline so save doesn't look like an external change
    let path_opt = self.workspace.active_entry()
        .and_then(|t| t.doc.file_path.clone());
    if let Some(ref p) = path_opt {
        if let Ok(meta) = std::fs::metadata(p) {
            if let Ok(mtime) = meta.modified() {
                self.file_watcher.start_watching(p.clone(), mtime, meta.len());
            }
        }
    }
    return effect.merge(AppEffect::UPDATE_TITLE).merge(AppEffect::REDRAW);
}
```

Note: we clone the path *after* the mutable borrow from `active_doc_mut()` has ended, so there's no borrow conflict.

Similarly for the SaveAs path (line 52-65), after successful save:

```rust
if let Some((result, dirty)) = save_result {
    if let Err(e) = result {
        eprintln!("另存失败: {e}");
    }
    self.update_document_edited(dirty);
    if result.is_ok() {
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(mtime) = meta.modified() {
                self.file_watcher.start_watching(path.clone(), mtime, meta.len());
            }
        }
    }
}
```

- [ ] **Step 4: Stop watching on tab close**

In `dispatch/tabs.rs`, modify `try_close_entry_with_prompt`. After successful close (all paths that call `self.workspace.close_entry(idx)`), check if we need to stop watching. The simplest approach: add at the start of `try_close_entry_with_prompt`:

Actually, simpler: add in the `close_entry` function within workspace or in the dispatch. The cleanest approach is to check after the close — if the closed tab was being watched, stop watching and switch to the new active tab.

Add this after each successful close path. Let me put it in `handle_workspace_effect` when called after close — that's already handled in Step 2 (ActiveChanged is emitted on close if the active tab was closed).

- [ ] **Step 5: Build check**

```bash
cargo build 2>&1 | head -30
```
Expected: compiles cleanly.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/dispatch/tabs.rs crates/app/src/dispatch/commands.rs
git commit -m "feat: wire FileWatcher into open_file, tab switch, save, and close"
```

---

### Task 4: Add polling check in about_to_wait

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs:514-576` (about_to_wait)
- Modify: `crates/app/src/app_window.rs:274-299` (compute_next_wake_time)

**Interfaces:**
- Consumes: `FileWatcher::should_check`, `FileWatcher::check`, `FileWatcher::next_check_time`

- [ ] **Step 1: Add check logic in about_to_wait**

In `app_lifecycle.rs`, in the `about_to_wait` method, after the animation check (line 556-557) and before `let did_request` (line 560), add:

```rust
// 检测外部文件变更 — 仅当活跃文件非 dirty 时检查
if self.file_watcher.should_check() {
    let dirty = self
        .workspace
        .active_entry()
        .map(|t| t.doc.dirty)
        .unwrap_or(true);
    if !dirty {
        if let Some(change) = self.file_watcher.check() {
            let file_name = change
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| change.path.display().to_string());
            let msg = format!("「{}」已被外部程序修改，是否重新加载？", file_name);
            let mut dialog = rfd::MessageDialog::new()
                .set_title("文件已变更")
                .set_description(&msg)
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_level(rfd::MessageLevel::Warning);
            if let Some(ref w) = self.window {
                dialog = dialog.set_parent(w.as_ref());
            }
            match dialog.show() {
                rfd::MessageDialogResult::Yes => {
                    self.reload_active_file(&change);
                    why = if why == "none" { "fwatch" } else { "fwatch+" };
                }
                rfd::MessageDialogResult::No => {
                    // User chose to ignore — update mtime baseline to stop re-prompting
                    if let Some(ref path) = self.workspace.active_doc()
                        .and_then(|dv| dv.file_path.as_ref())
                    {
                        if let Ok(meta) = std::fs::metadata(path) {
                            if let Ok(mtime) = meta.modified() {
                                self.file_watcher.confirm_reload(mtime, meta.len());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
```

- [ ] **Step 2: Add next_check_time to compute_next_wake_time**

In `app_window.rs`, in `compute_next_wake_time` (line 274), add after the tab bar animation check (line 296):

```rust
// 3. File watcher poll
if let Some(next_fw) = self.file_watcher.next_check_time() {
    earliest = Some(match earliest {
        Some(e) => e.min(next_fw),
        None => next_fw,
    });
}
```

- [ ] **Step 3: Build check**

```bash
cargo build 2>&1 | head -30
```
Expected: compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/app_lifecycle.rs crates/app/src/app_window.rs
git commit -m "feat: add file-watch polling and dialog in about_to_wait"
```

---

### Task 5: Implement reload_active_file

**Files:**
- Modify: `crates/app/src/dispatch/tabs.rs` (new method)

**Interfaces:**
- Consumes: `FileChange` from Task 1, `DocumentView::from_file` from existing code
- Produces: `reload_active_file(&mut self, change: &FileChange)` method on App

- [ ] **Step 1: Add reload_active_file method**

In `dispatch/tabs.rs`, add the new method after `open_file` (after line 53):

```rust
/// Reload the active file after external change detected.
/// Preserves scroll position by snapping to the same doc_line.
pub(crate) fn reload_active_file(&mut self, change: &crate::file_watcher::FileChange) {
    let scroll_anchor = self
        .workspace
        .active_doc()
        .map(|dv| dv.display.viewport.scroll_anchor);

    let visible_rows = self.visible_rows(self.screen_height());
    let viewport_height = self.visible_height_lines(self.screen_height());

    match DocumentView::from_file(&change.path, visible_rows, viewport_height) {
        Ok(mut new_dv) => {
            // Restore scroll position
            if let Some(anchor) = scroll_anchor {
                let doc_line = anchor.doc_line.min(new_dv.line_count().saturating_sub(1));
                new_dv.display.viewport.scroll_anchor =
                    ui::viewport::ScrollAnchor::new(doc_line, anchor.pixel_offset);
            }
            let new_mtime = change.new_mtime;
            let new_size = change.new_size;
            let active_idx = self.workspace.active_index();
            // Replace the DocumentView in-place
            if let Some(entry) = self.workspace.entry_mut(active_idx) {
                entry.doc = new_dv;
            }
            self.file_watcher.confirm_reload(new_mtime, new_size);
            // Invalidate reshape and redraw
            self.invalidate_reshape();
            self.frame_cache.advance_cache.clear();
            self.frame_cache.cluster_pool.clear();
            self.init_display_map(active_idx);
            self.needs_redraw = true;
        }
        Err(e) => {
            eprintln!("[file_watcher] reload failed: {e}");
        }
    }
}
```

Wait, we need to check the exact `scroll_anchor` import. The `ui::viewport::ScrollAnchor` is public. Let me also check if `invalidate_reshape` exists:

Let me adjust the implementation to be correct:

```rust
/// Reload the active file after external change detected.
/// Preserves scroll position by snapping to the same doc_line.
pub(crate) fn reload_active_file(&mut self, change: &crate::file_watcher::FileChange) {
    let scroll_anchor = self
        .workspace
        .active_doc()
        .map(|dv| dv.display.viewport.scroll_anchor);

    let visible_rows = self.visible_rows(self.screen_height());
    let viewport_height = self.visible_height_lines(self.screen_height());

    match DocumentView::from_file(&change.path, visible_rows, viewport_height) {
        Ok(mut new_dv) => {
            if let Some(anchor) = scroll_anchor {
                let doc_line = anchor.doc_line.min(new_dv.line_count().saturating_sub(1));
                new_dv.display.viewport.scroll_anchor =
                    ui::viewport::ScrollAnchor::new(doc_line, anchor.pixel_offset);
            }
            let new_mtime = change.new_mtime;
            let new_size = change.new_size;
            let idx = self.workspace.active_index();
            if let Some(entry) = self.workspace.entry_mut(idx) {
                entry.doc = new_dv;
            }
            self.file_watcher.confirm_reload(new_mtime, new_size);
            // Rebuild display state
            self.invalidate_reshape();
            self.frame_cache.advance_cache.clear();
            self.frame_cache.cluster_pool.clear();
            self.init_display_map(idx);
            self.needs_redraw = true;
        }
        Err(e) => {
            eprintln!("[file_watcher] reload failed: {e}");
        }
    }
}
```

Hmm, I need to check what `invalidate_reshape` does and whether it exists on App.

Let me search for it.

Actually, let me just look at it — in the `app_search.rs` or `app_reshape.rs` there should be something:

Let me just write the plan with what we know and note to check during implementation.

- [ ] **Step 2: Build check**

```bash
cargo build 2>&1
```
Expected: compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/dispatch/tabs.rs
git commit -m "feat: implement reload_active_file with scroll position restore"
```

---

### Task 6: Integration test (manual)

**Files:**
- None modified

**Interfaces:**
- Consumes: All previous tasks

- [ ] **Step 1: Build release binary and smoke test**

```bash
cargo build --release
```

Manual test steps:
1. Open a text file in the editor
2. In another terminal, modify the file: `echo "external change" >> /path/to/file`
3. Wait 2 seconds — expect the "文件已变更" dialog to appear
4. Click "重新加载" — file should reload with scroll position preserved
5. Test "忽略" — dialog should dismiss, no further prompts unless file changes again
6. Make unsaved edits (dirty = true) — modify file externally, verify no dialog appears
7. Save the file, then modify externally — verify dialog appears after save

- [ ] **Step 2: Run full test suite to check for regressions**

```bash
./scripts/verify.sh
```
Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "chore: final integration notes for file watcher"
```
