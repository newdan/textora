# Code Quality Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the remaining code quality issues from Phase 3/4, including compiling warnings, app/ui API boundaries, Sidebar settings decoupling, and ThemeRegistry error handling.

**Architecture:** Use `cargo fix` to remove warnings. Shrink the public API by restricting modules in `app/src/lib.rs` and `ui/src/lib.rs` to `pub(crate)`. Extract Sidebar configuration out of global Settings/UiMetrics implicit reads and inject a localized `SidebarSettingsInput`. Handle `ThemeRegistry` errors gracefully with explicit `Result` propagation instead of panics or IO assumptions. Ensure all commits pass CI-grade validation.

**Tech Stack:** Rust, cargo, winit ecosystem.

## Global Constraints

- No `std::fs` calls inside `crates/ui`.
- CI strictly requires 0 warnings (`cargo clippy -- -D warnings`).
- Every commit must compile and pass all tests independently.

---

### Task 1: Resolve Unused Import Warnings

**Files:**
- Modify: `crates/app/src/*`
- Modify: `crates/ui/src/*`

**Interfaces:**
- Consumes: N/A
- Produces: A warning-free compilation state.

- [ ] **Step 1: Write the failing test**

We will treat the compiler warning detector as our test. There is no new code to write here, but we will assert the current failure state.

```bash
# No code modification needed for the failing test here.
```

- [ ] **Step 2: Run test to verify it fails**

Run: `RUSTFLAGS="-D warnings" cargo check -p edit-plus-ui -p edit-plus-app --lib`
Expected: FAIL with multiple `unused import` or `unused mut` errors.

- [ ] **Step 3: Write minimal implementation**

Execute cargo's automatic fix tool for the unused warnings.

```bash
cargo fix --lib -p edit-plus-ui --allow-dirty --allow-no-vcs
cargo fix --lib -p edit-plus-app --allow-dirty --allow-no-vcs
```

- [ ] **Step 4: Run test to verify it passes**

Run: `RUSTFLAGS="-D warnings" cargo check -p edit-plus-ui -p edit-plus-app --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/ crates/app/src/
git commit -m "style: resolve unused imports and mutability warnings"
```

### Task 2: Shrink App and UI Public APIs

**Files:**
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Consumes: The existing module structures.
- Produces: Strictly bounded APIs exposing only the required structs (like `App`, `AppEvent`).

- [ ] **Step 1: Write the failing test**

We write a sentinel ripgrep command that acts as our test to detect unauthorized public module exposures.

```bash
# Test shell script block
# This will fail if there are any unauthorized `pub mod` declarations.
```

- [ ] **Step 2: Run test to verify it fails**

Run: `rg "^pub mod (actions|app_dispatch|app_init);" crates/app/src/lib.rs`
Expected: Outputs the matching lines (this means the test "fails" because the modules are still exposed).

- [ ] **Step 3: Write minimal implementation**

Update `crates/app/src/lib.rs` to replace unauthorized `pub mod` with `pub(crate) mod`.

```rust
// In crates/app/src/lib.rs, modify the module declarations:
pub(crate) mod actions;
pub(crate) mod app_dispatch;
pub(crate) mod app_init;
pub(crate) mod app_lifecycle;
pub(crate) mod app_renderer;
pub(crate) mod app_reshape;
pub(crate) mod app_scroll;
pub(crate) mod app_search;
pub(crate) mod app_tab;
pub(crate) mod app_window;
// Keep `pub mod app` and others that are strictly needed for external binary usage if any, otherwise default to pub(crate).
```

Update `crates/ui/src/lib.rs` similarly:

```rust
// In crates/ui/src/lib.rs, modify the module declarations:
pub(crate) mod layout;
pub(crate) mod render_geom;
pub(crate) mod text_renderer;
pub(crate) mod view_mode;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo check --workspace`
Expected: PASS (No compilation errors regarding visibility in the binary workspace members).
Run: `rg "^pub mod (actions|app_dispatch|app_init);" crates/app/src/lib.rs`
Expected: No output.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/lib.rs crates/ui/src/lib.rs
git commit -m "refactor: enforce module visibility boundaries for app and ui"
```

### Task 3: Split SidebarSettingsInput and UiMetrics

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/types.rs`
- Modify: `crates/ui/src/widgets/sidebar/state.rs`
- Modify: `crates/app/src/ui_shell.rs`

**Interfaces:**
- Consumes: Global configuration in `ui_shell.rs`.
- Produces: `SidebarSettingsInput` which isolates the sidebar's settings needs.

- [ ] **Step 1: Write the failing test**

```rust
// In crates/ui/src/widgets/sidebar/widget_tests.rs
#[test]
fn sidebar_settings_input_is_independent() {
    let input = crate::widgets::sidebar::types::SidebarSettingsInput {
        dpi: 2.0,
        show_line_numbers: true,
        word_wrap: false,
        show_status_bar: true,
        theme_mode: crate::settings::ThemeMode::Dark,
    };
    assert_eq!(input.dpi, 2.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p edit-plus-ui widgets::sidebar::widget_tests::sidebar_settings_input_is_independent -- --exact`
Expected: FAIL with "cannot find type `SidebarSettingsInput`" or missing fields.

- [ ] **Step 3: Write minimal implementation**

```rust
// In crates/ui/src/widgets/sidebar/types.rs
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarSettingsInput {
    pub dpi: f32,
    pub show_line_numbers: bool,
    pub word_wrap: bool,
    pub show_status_bar: bool,
    pub theme_mode: crate::settings::ThemeMode,
}
```

Update `crates/ui/src/widgets/sidebar/state.rs` and `crates/ui/src/widgets/sidebar/menu.rs` to consume `SidebarSettingsInput` instead of relying on `UiMetrics` for behavior toggles.

Update `crates/app/src/ui_shell.rs` to construct and pass `SidebarSettingsInput`:

```rust
let sidebar_input = ui::widgets::sidebar::types::SidebarSettingsInput {
    dpi: metrics.dpi,
    show_line_numbers: metrics.show_line_numbers,
    word_wrap: metrics.word_wrap,
    show_status_bar: metrics.show_status_bar,
    theme_mode: metrics.theme_mode,
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p edit-plus-ui widgets::sidebar::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/sidebar/ crates/app/src/ui_shell.rs
git commit -m "refactor(ui): decouple sidebar input from general ui metrics"
```

### Task 4: ThemeRegistry Error Handling

**Files:**
- Modify: `crates/ui/src/theme.rs`

**Interfaces:**
- Consumes: `ThemeSource`
- Produces: Clean `Result<Theme, LoadError>` without panics.

- [ ] **Step 1: Write the failing test**

```rust
// In crates/ui/src/theme.rs
#[test]
fn invalid_theme_source_returns_error_instead_of_panic() {
    let mut registry = ThemeRegistry::new();
    let source = ThemeSource {
        id: "invalid".to_string(),
        path: std::path::PathBuf::from("invalid.toml"),
        content: "[[invalid_toml".to_string(),
    };
    
    let errors = registry.register_sources(vec![source]);
    assert!(!errors.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p edit-plus-ui theme::tests::invalid_theme_source_returns_error_instead_of_panic -- --exact`
Expected: FAIL (either doesn't compile due to missing API, or panics internally).

- [ ] **Step 3: Write minimal implementation**

Refactor `ThemeRegistry` in `crates/ui/src/theme.rs` to parse strings safely:

```rust
// Remove any uses of `expect()`, `unwrap()`, or `std::fs::read_to_string` inside `load_pending`.
// Ensure toml::from_str returns a Result that is appended to the returned Vec<LoadError>.
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p edit-plus-ui theme::tests::invalid_theme_source_returns_error_instead_of_panic -- --exact`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/theme.rs
git commit -m "refactor(ui): handle theme parsing errors gracefully"
```

### Task 5: CI Guard Script

**Files:**
- Create: `scripts/verify.sh`

**Interfaces:**
- Consumes: Codebase
- Produces: Bash exit code (0 for success).

- [ ] **Step 1: Write the failing test**

We write a script that runs the checks and acts as the test itself.

```bash
#!/usr/bin/env bash
set -e
echo "Checking for warnings..."
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 2: Run test to verify it fails**

Run: `./scripts/verify.sh`
Expected: Fails if the codebase still has warnings (though Task 1 should have cleared them).

- [ ] **Step 3: Write minimal implementation**

Finalize the script content:

```bash
#!/usr/bin/env bash
set -e

echo "Running tests..."
cargo test --workspace

echo "Checking for warnings..."
cargo clippy --workspace --all-targets -- -D warnings

echo "Verification passed."
```

- [ ] **Step 4: Run test to verify it passes**

Run: `chmod +x scripts/verify.sh && ./scripts/verify.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add scripts/verify.sh
git commit -m "chore: enforce zero-warnings in verify script"
```
