# Startup Critical Path Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce perceived cold-start latency by measuring time to first visible frame without regressing the native recent-files menu.

**Architecture:** Keep startup timing owned by `App`: record the instant at `App::new`, then emit one total when the first frame is presented and made opaque. Preserve the native recent-files menu with a loading placeholder while a background worker validates paths and posts its result back to the main thread.

**Tech Stack:** Rust 2024, winit, wgpu, existing unit-test framework.

## Global Constraints

- Keep `ui` independent of application state.
- Use semantic names and `cargo fmt` formatting.
- Do not use `unwrap()` in new production code.
- Keep each implementation phase to at most three production files.

---

### Task 1: Record App-to-first-visible-frame latency

**Files:**

- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/app_renderer.rs`

**Interfaces:**

- Produces: `App::startup_started_at: Instant`, initialized before synchronous settings/font initialization.
- Produces: one `[startup] first_frame_visible total: …` line, emitted exactly once after the first `present()` and alpha restoration.

- [x] **Step 1: Add an initialization invariant test**

Add a unit test in `crates/app/src/app_init.rs` that constructs `App::new(None)` and asserts `startup_started_at.elapsed()` is less than one minute. This fails to compile until the field exists.

- [x] **Step 2: Run the focused test and observe the expected compile failure**

Run: `cargo test -p textora-app startup_timestamp_is_initialized`

Expected: compilation fails because `App` has no `startup_started_at` field.

- [x] **Step 3: Add the minimal timing field and first-frame log**

Add `pub(crate) startup_started_at: Instant` to `App`, set it at the beginning of `App::new`, and, inside the existing `if !self.first_frame_presented` block immediately after `set_window_alpha(w, 1.0)`, log `self.startup_started_at.elapsed()`.

- [x] **Step 4: Run focused and crate tests**

Run: `cargo test -p textora-app startup_timestamp_is_initialized`

Expected: PASS.

- [x] **Step 5: Build the release binary and manually verify the new startup milestone**

Run: `cargo build --release -p textora-app`

Expected: exit 0. Launch the release binary with a short-lived diagnostic harness and verify exactly one `first_frame_visible` line appears.

### Task 2: Validate recent files in the background

**Files:**

- Modify: `crates/app/src/native_menu.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_event.rs`

**Interfaces:**

- Produces: `AppEvent::RecentFilesLoaded(Vec<PathBuf>)` and a background loader that validates persisted recent paths outside the main thread.
- Produces: a loading native menu with a persistent “打开最近的文件” submenu, later rebuilt with validated items on the main thread.

- [x] **Step 1: Add a failing behavior test**

Add a focused `app_lifecycle` test for a missing and an existing recent path. It must prove that the worker-side loader returns only the existing path before any event-loop code is added.

- [x] **Step 2: Run the focused test and observe the expected compile failure**

Run: `cargo test -p textora-app recent_file_loader_filters_missing_paths`

Expected: compilation fails because the worker-side loader does not exist.

- [x] **Step 3: Implement the background load and menu refresh**

Build the native menu immediately with a disabled “正在加载最近文件…” item. Spawn a background loader from `do_resumed`; it sends validated paths through `EventLoopProxy`. In `user_event`, rebuild the native menu with those paths. A completed empty result must retain the submenu with a disabled “没有最近文件” item.

- [x] **Step 4: Run focused and full app-library tests**

Run: `cargo test -p textora-app recent_file_loader_filters_missing_paths`

Run: `cargo test -p textora-app --lib`

Expected: both exit 0.

- [x] **Step 5: Verify the complete critical path**

Run: `cargo build --release -p textora-app`

Expected: exit 0. Launch the release binary, verify the first-frame metric is emitted once, then verify the recent submenu remains present before and after background loading.
