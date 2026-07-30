# Save & Close Tab Fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix blank display when discarding unsaved tabs; fix Cmd+S / menu Save silent-fail on untitled files; implement menu SaveAs.

**Architecture:** Four small changes across three files: one line in workspace.rs to lazy-load the new active tab on close; a shared `save_active_tab` helper in dispatch/commands.rs that tries direct save first then falls back to dialog; wire both keyboard and menu Save/SaveAs paths through that helper.

**Tech Stack:** Rust, winit, rfd (file dialog), no new dependencies

## Global Constraints

- No new dependencies
- No API or config changes
- Must handle untitled (no file_path) and named files identically across keyboard and menu paths

---

## File Map

| File | Role |
|------|------|
| `crates/app/src/workspace.rs` | Tab lifecycle; `close_tab_inner` |
| `crates/app/src/dispatch/commands.rs` | Menu command dispatch; new `save_active_tab` helper |
| `crates/app/src/dispatch/editor.rs` | Keyboard command dispatch; delegate Save/SaveAs to helper |

---

### Task 1: Fix blank display after discarding unsaved tab

**Files:**
- Modify: `crates/app/src/workspace.rs:407`

**Interfaces:**
- Consumes: `self.lazy_load_tab(usize)` (already defined, line 291)
- Produces: N/A (same return type)

**Root cause:** `close_tab_inner` removes the active tab and sets a new `active_index`, but never calls `lazy_load_tab`. If the new active tab is a stub (restored from workspace snapshot, never visited), it stays empty — rendering blank. `switch_to` does call `lazy_load_tab`; `close_tab_inner` should too.

- [ ] **Step 1: Write the test**

Add to the test module in `crates/app/src/workspace.rs`, inside `mod tests`, before the last `}`:

```rust
#[test]
fn close_tab_inner_lazy_loads_new_active_stub() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("lazy.txt");
    let mut f = std::fs::File::create(&file_path).unwrap();
    f.write_all(b"loaded content\nline 2\n").unwrap();
    f.flush().unwrap();
    drop(f);

    let mut ws = Workspace::new();

    // Tab 0: stub with file_path (simulating workspace restore)
    let mut stub = DocumentView::new(vec![String::new()], 10, 10.0);
    stub.file_path = Some(file_path.clone());
    ws.push_view(View::Editor(stub));

    // Tab 1: active, dirty
    let mut dirty = DocumentView::new(vec!["dirty".to_string()], 10, 10.0);
    dirty.dirty = true;
    ws.push_view(View::Editor(dirty));
    ws.active_index = 1;

    assert_eq!(ws.len(), 2);

    // Close active dirty tab (tab 1)
    let result = ws.close_tab_inner(1);
    assert!(result.is_ok());
    assert_eq!(ws.active_index, 0);
    assert_eq!(ws.len(), 1);

    // After fix: tab 0 should be lazy-loaded (content from file, not empty stub)
    let doc = ws.active_doc().unwrap();
    assert!(doc.buffer_len() > 0, "stub should have been lazy-loaded, not empty");
    assert_eq!(doc.line_count(), 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p app close_tab_inner_lazy_loads_new_active_stub
```

Expected: FAIL — `buffer_len()` is 0, stub was not loaded.

- [ ] **Step 3: Implement the fix**

In `crates/app/src/workspace.rs`, `close_tab_inner`, change lines 407-409 from:

```rust
        if was_active {
            Ok(WorkspaceEffect::ActiveTabChanged)
        } else {
            Ok(WorkspaceEffect::LayoutChanged)
        }
```

To:

```rust
        if was_active {
            self.lazy_load_tab(self.active_index);
            Ok(WorkspaceEffect::ActiveTabChanged)
        } else {
            Ok(WorkspaceEffect::LayoutChanged)
        }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p app close_tab_inner_lazy_loads_new_active_stub
```

Expected: PASS

- [ ] **Step 5: Run existing tests to confirm no regressions**

```bash
cargo test -p app workspace
```

Expected: all workspace tests PASS

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/workspace.rs
git commit -m "fix(workspace): lazy-load new active tab after closing dirty tab

close_tab_inner did not call lazy_load_tab when the closed tab was
active, so if the new active tab was a stub (restored from workspace
snapshot but never visited), it rendered blank. switch_to already
does this; close_tab_inner now does it too."
```

---

### Task 2: Extract shared `save_active_tab` helper

**Files:**
- Modify: `crates/app/src/dispatch/commands.rs` (add new method, refactor existing `SaveActiveTab` and `SaveActiveTabAs`)

**Interfaces:**
- Consumes: `self.workspace`, `self.window`, `self.update_document_edited(bool)`
- Produces: `pub(crate) fn save_active_tab(&mut self, force_dialog: bool) -> AppEffect`

**Design:** `force_dialog: false` → try direct save, fallback to dialog on "no file path" error. `force_dialog: true` → always show dialog (SaveAs behavior). Both menu and keyboard paths call this one function.

- [ ] **Step 1: Add the helper method**

In `crates/app/src/dispatch/commands.rs`, inside `impl App`, add before `dispatch_app_command`:

```rust
    /// Save the active tab. If `force_dialog` is true, always show the SaveAs
    /// dialog. Otherwise try a direct save first, falling back to the dialog
    /// when the file has no path (untitled).
    pub(crate) fn save_active_tab(&mut self, force_dialog: bool) -> AppEffect {
        let mut effect = AppEffect::NONE;
        let active_idx = self.workspace.active_index();

        if !force_dialog {
            // Try direct save
            if let Some(dv) = self.workspace.active_doc_mut() {
                match dv.save() {
                    Ok(()) => {
                        self.update_document_edited(dv.dirty);
                        return effect.merge(AppEffect::UPDATE_TITLE).merge(AppEffect::REDRAW);
                    }
                    Err(ref e) if e == "no file path" => {
                        // fall through to dialog
                    }
                    Err(e) => {
                        eprintln!("save error: {e}");
                        self.update_document_edited(dv.dirty);
                        return effect.merge(AppEffect::REDRAW);
                    }
                }
            } else {
                return effect;
            }
        }

        // SaveAs dialog
        let default_name = self
            .workspace
            .view(active_idx)
            .map(|v| v.doc())
            .and_then(|dv| dv.file_path.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "未命名".to_string());

        let mut dialog = rfd::FileDialog::new().set_file_name(&default_name);
        if let Some(ref w) = self.window {
            dialog = dialog.set_parent(w);
        }

        if let Some(path) = dialog.save_file() {
            if let Some(dv) = self.workspace.view_mut(active_idx).map(|v| v.doc_mut()) {
                if let Err(e) = dv.save_as(&path) {
                    eprintln!("另存失败: {e}");
                }
                self.update_document_edited(dv.dirty);
            }
            effect = effect.merge(AppEffect::UPDATE_TITLE);
        }
        effect.merge(AppEffect::REDRAW)
    }
```

- [ ] **Step 2: Rewire `SaveActiveTab` to use the helper**

In `dispatch_app_command`, replace the `SaveActiveTab` arm (lines 22-35):

```rust
            crate::menu_handler::AppCommand::SaveActiveTab => {
                effect = effect.merge(self.save_active_tab(false));
            }
```

- [ ] **Step 3: Implement `SaveActiveTabAs` using the helper**

In `dispatch_app_command`, replace the `SaveActiveTabAs` arm (lines 36-38):

```rust
            crate::menu_handler::AppCommand::SaveActiveTabAs => {
                effect = effect.merge(self.save_active_tab(true));
            }
```

- [ ] **Step 4: Build check**

```bash
cargo build -p app 2>&1
```

Expected: compiles clean

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/dispatch/commands.rs
git commit -m "feat(menu): implement SaveAs, extract save_active_tab helper

Extract shared save_active_tab(force_dialog) helper that tries direct
save first then falls back to SaveAs dialog for untitled files.
Wire menu Save and SaveAs through it. Previously SaveAs was a noop
and Save silently failed on untitled files."
```

---

### Task 3: Route keyboard Cmd+S / Cmd+Shift+S through helper

**Files:**
- Modify: `crates/app/src/dispatch/editor.rs:306-364`

**Interfaces:**
- Consumes: `self.save_active_tab(bool)` (produced by Task 2)
- Produces: N/A (keyboard dispatch still returns `AppEffect`)

- [ ] **Step 1: Replace `EditCommand::Save` handler**

In `dispatch/editor.rs`, replace lines 306-337 (`EditCommand::Save` arm) with:

```rust
            EditCommand::Save => {
                effect = effect.merge(self.save_active_tab(false));
                return effect;
            }
```

- [ ] **Step 2: Replace `EditCommand::SaveAs` handler**

In `dispatch/editor.rs`, replace lines 339-364 (`EditCommand::SaveAs` arm) with:

```rust
            EditCommand::SaveAs => {
                effect = effect.merge(self.save_active_tab(true));
                return effect;
            }
```

- [ ] **Step 3: Build check**

```bash
cargo build -p app 2>&1
```

Expected: compiles clean

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/dispatch/editor.rs
git commit -m "fix(editor): route keyboard Save/SaveAs through shared helper

Cmd+S now falls back to SaveAs dialog when the file has no path,
instead of silently logging an error. Cmd+Shift+S delegates to
the same helper as the menu for consistent behavior."
```

---

### Task 4: Integration verification

- [ ] **Step 1: Run full app test suite**

```bash
cargo test -p app 2>&1
```

Expected: all tests PASS

- [ ] **Step 2: Clippy check**

```bash
cargo clippy -p app -- -D warnings 2>&1
```

Expected: no warnings

