# Syncthing Cold Start Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove recursive content polling and defer Syncthing/file-monitor startup until after Textora's first visible frame.

**Architecture:** `LibraryFileMonitor` uses `notify::RecommendedWatcher`, which maps to FSEvents on macOS, and continues to feed candidate paths into the existing revision-aware file-safety pipeline. The renderer queues a semantic `StartBackgroundServices` event only after the first `present()`; the app handles that event idempotently and performs one immediate revision reconciliation after the watcher is ready.

**Tech Stack:** Rust 2024, winit `ApplicationHandler<AppEvent>`, notify 8.2 `RecommendedWatcher`, existing `FileSafetyWorker`, Cargo test/check/fmt.

## Global Constraints

- `ui` must remain independent from `app`, `DocumentView`, Syncthing DTOs, Keychain handles, and worker objects.
- Syncthing configuration, Keychain access, and REST probes remain on background workers.
- The first visible frame must not recursively walk or hash opened-document parent directories.
- Watcher or controller startup failure must not prevent editing, opening, saving, or first-frame presentation.
- Production code must not use `PollWatcher`, `with_poll_interval`, or `with_compare_contents`.
- Every task modifies at most three files and must compile before the next task starts.
- Use precise names, early returns, no new magic values, no `unwrap()`, and run `cargo fmt`.
- Because this change crosses monitoring, events, rendering, and lifecycle code, finish with `./scripts/verify.sh`.

---

### Task 1: Replace recursive content polling with the platform event backend

**Files:**
- Modify and test: `crates/app/src/library_file_monitor.rs:1-256`

**Interfaces:**
- Consumes: existing `LibraryFileMonitor::spawn`, `replace_roots`, `try_recv`, and `shutdown` API.
- Produces: the same API backed by `notify::RecommendedWatcher`; later tasks require no caller changes.

- [ ] **Step 1: Write the failing backend-boundary test**

Add this helper and test inside the existing `tests` module. Splitting forbidden identifiers prevents the test source itself from matching them.

```rust
fn production_source() -> &'static str {
    include_str!("library_file_monitor.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("library monitor production source should precede tests")
}

#[test]
fn production_monitor_uses_platform_event_backend() {
    let source = production_source();
    for forbidden_parts in [
        ["Poll", "Watcher"],
        ["with_poll", "_interval"],
        ["with_compare", "_contents"],
    ] {
        let forbidden = forbidden_parts.concat();
        assert!(!source.contains(&forbidden), "production monitor must not contain {forbidden}");
    }
    assert!(source.contains("RecommendedWatcher"));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test -p textora-app --lib -- library_file_monitor::tests::production_monitor_uses_platform_event_backend`

Expected: FAIL because production source still contains `PollWatcher`.

- [ ] **Step 3: Implement the minimal event-driven backend**

Use the platform-recommended concrete type and default event configuration:

```rust
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

let watcher = RecommendedWatcher::new(
    move |event| {
        let _ = event_sender.send(WorkerMessage::Notify(event));
    },
    Config::default(),
)
.map_err(|error| MonitorError::WatchFailed { message: error.to_string() })?;
```

Change the watcher parameters of `monitor_loop` and `replace_watched_roots` from `PollWatcher` to `RecommendedWatcher`. Do not change debounce behavior, path filtering, recursion semantics, or the public API.

- [ ] **Step 4: Verify GREEN and existing file events**

Run:

```bash
cargo test -p textora-app --lib -- library_file_monitor::tests
cargo check -p textora-app
cargo fmt --check
```

Expected: backend-boundary, recursive create/modify/rename/delete, debounce, and temporary-file tests PASS; check and formatting exit 0.

- [ ] **Step 5: Commit Task 1**

```bash
git add crates/app/src/library_file_monitor.rs
git commit -m "perf(app): use event driven file monitoring"
```

---

### Task 2: Add an idempotent post-frame background-service event

**Files:**
- Modify and test: `crates/app/src/app_event.rs:1-12`
- Modify: `crates/app/src/app.rs:162-216`
- Modify and test: `crates/app/src/app_lifecycle.rs:848-891`

**Interfaces:**
- Consumes: `App::event_loop_proxy`, `LibraryFileMonitor::spawn`, `SyncController::new_default`, `refresh_file_monitor_roots`, and `file_safety_next_check`.
- Produces: `AppEvent::StartBackgroundServices` and `App::start_background_services(&mut self)`, both consumed by Task 3.

- [ ] **Step 1: Write the failing event and lifecycle tests**

Add to `app_event.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::AppEvent;

    #[test]
    fn background_services_have_a_semantic_start_event() {
        assert!(matches!(AppEvent::StartBackgroundServices, AppEvent::StartBackgroundServices));
    }
}
```

Add to the existing `app_lifecycle.rs` tests module:

```rust
#[test]
fn user_event_routes_background_service_startup() {
    let source = include_str!("app_lifecycle.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("lifecycle production source should precede tests");
    assert!(source.contains(
        "AppEvent::StartBackgroundServices => self.start_background_services()"
    ));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test -p textora-app --lib -- app_event::tests::background_services_have_a_semantic_start_event`

Expected: compilation FAIL because the event does not exist. After adding only the enum variant, run `cargo test -p textora-app --lib -- app_lifecycle::tests::user_event_routes_background_service_startup`; expected FAIL because the route is absent.

- [ ] **Step 3: Implement the event, idempotent initializer, and route**

Add the enum variant:

```rust
/// The first visible frame has completed; start deferred background services.
StartBackgroundServices,
```

Add `App::start_background_services(&mut self)`. It must early-return without an event proxy, create the monitor only when `library_file_monitor.is_none()`, call `refresh_file_monitor_roots()`, set `file_safety_next_check = Instant::now()` after monitor readiness, and create the controller only when `sync_controller.is_none()`. Reuse the existing wake callbacks exactly so no worker gains app-state ownership.

Route the event in `ApplicationHandler::user_event`:

```rust
AppEvent::StartBackgroundServices => self.start_background_services(),
```

The existing `Option` checks make repeated events idempotent. The immediate next-check timestamp drives reconciliation through the existing `about_to_wait()` path.

- [ ] **Step 4: Verify GREEN and compile the intermediate state**

Run:

```bash
cargo test -p textora-app --lib -- app_event::tests::background_services_have_a_semantic_start_event
cargo test -p textora-app --lib -- app_lifecycle::tests::user_event_routes_background_service_startup
cargo check -p textora-app
cargo fmt --check
```

Expected: both tests PASS; compile and formatting exit 0. Early startup still exists at this checkpoint, so the new event route is harmlessly idempotent.

- [ ] **Step 5: Commit Task 2**

```bash
git add crates/app/src/app_event.rs crates/app/src/app.rs crates/app/src/app_lifecycle.rs
git commit -m "refactor(app): add deferred background startup event"
```

---

### Task 3: Queue background startup only after the first presented frame

**Files:**
- Modify and test: `crates/app/src/app.rs:162-197`
- Modify and test: `crates/app/src/app_renderer.rs:91-260,1183-1197`

**Interfaces:**
- Consumes: `AppEvent::StartBackgroundServices` and `App::start_background_services` from Task 2.
- Produces: first-frame ordering in which only `FileSafetyWorker` starts before `run_app`; monitor and controller start from the queued post-present event.

- [ ] **Step 1: Write failing critical-path source tests**

Add a test module in `app.rs` that extracts the production body between `pub fn set_event_loop_proxy` and `pub(crate) fn start_background_services`, then asserts it contains neither `LibraryFileMonitor::spawn` nor `SyncController::new_default`.

Add to the existing renderer tests:

```rust
#[test]
fn first_frame_queues_background_services_after_present() {
    let source = include_str!("app_renderer.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("renderer production source should precede tests");
    let present = source.find("output.present();").expect("frame should be presented");
    let startup_event = source
        .find("AppEvent::StartBackgroundServices")
        .expect("renderer should queue deferred startup");
    assert!(startup_event > present);
}
```

- [ ] **Step 2: Run both tests and verify RED**

Run:

```bash
cargo test -p textora-app --lib -- background_startup_boundary_tests::event_loop_proxy_registration_does_not_start_deferred_services
cargo test -p textora-app --lib -- app_renderer::tests::first_frame_queues_background_services_after_present
```

Expected: first test FAIL because proxy registration still starts monitor/controller; second test FAIL because renderer does not queue the event.

- [ ] **Step 3: Remove deferred services from proxy registration**

Keep event-loop proxy storage and `FileSafetyWorker` initialization in `set_event_loop_proxy()`. Delete only the `LibraryFileMonitor` and `SyncController` blocks. Assign the consumed proxy directly to `file_safety_proxy` after storing a clone.

- [ ] **Step 4: Queue the event after first present**

Inside the existing `if !self.first_frame_presented` block, after restoring window alpha, add:

```rust
if let Some(event_loop_proxy) = self.event_loop_proxy.as_ref() {
    let _ = event_loop_proxy.send_event(crate::app_event::AppEvent::StartBackgroundServices);
}
```

The `first_frame_presented` guard guarantees one enqueue. Event-loop delivery guarantees the initializer runs after the current render returns.

- [ ] **Step 5: Verify GREEN and focused behavior**

Run:

```bash
cargo test -p textora-app --lib -- background_startup_boundary_tests
cargo test -p textora-app --lib -- app_renderer::tests::first_frame_queues_background_services_after_present
cargo test -p textora-app --lib -- library_file_monitor::tests
cargo check -p textora-app
cargo fmt --check
```

Expected: all selected tests PASS; compile and formatting exit 0.

- [ ] **Step 6: Commit Task 3**

```bash
git add crates/app/src/app.rs crates/app/src/app_renderer.rs
git commit -m "perf(app): defer sync services until first frame"
```

---

### Task 4: Full verification and startup evidence

**Files:**
- Verify only; no production file changes expected.

**Interfaces:**
- Consumes: completed event-driven watcher and post-frame startup path.
- Produces: fresh evidence that formatting, architecture constraints, compilation, tests, and the repository verification gate pass.

- [ ] **Step 1: Verify forbidden polling configuration is absent**

Run: `rg -n "PollWatcher|with_poll_interval|with_compare_contents" crates/app/src/library_file_monitor.rs`

Expected: no production-code matches.

- [ ] **Step 2: Run the full required repository gate**

Run: `./scripts/verify.sh`

Expected: exit 0 with formatting, checks, lints, and tests passing.

- [ ] **Step 3: Inspect final ordering and worktree**

Run:

```bash
git diff a768d8b9..HEAD -- crates/app/src/app_event.rs crates/app/src/app.rs crates/app/src/app_lifecycle.rs crates/app/src/app_renderer.rs crates/app/src/library_file_monitor.rs
git status --short
```

Expected: diff shows platform event monitoring and post-present event delivery; worktree is clean.

- [ ] **Step 4: Report scope honestly**

Report that the blocking recursive content scan has been removed by code-path verification and automated tests. Do not claim an exact millisecond improvement without a controlled GUI A/B run; cite `[startup] first_frame_visible total` as the manual measurement hook.
