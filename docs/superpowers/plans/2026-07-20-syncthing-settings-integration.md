# Syncthing Settings Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing modal Settings view the only Syncthing entry and add a complete fourth “同步” category containing every capability of the current standalone sync panel.

**Architecture:** `SettingsView` owns an embedded `SyncSettingsPage`. The page receives pure `SyncSettingsInput`, emits `SettingsViewAction::Sync(SyncSettingsAction)`, and uses existing Form containers. The app maps controller snapshots into the page and reuses the existing controller/`rfd` orchestration; the standalone `SyncPanelWidget` overlay and its action chain are removed after the embedded path is verified.

**Tech Stack:** Rust 2024, custom wgpu UI, winit 0.30, existing Form widgets, `zeroize`-backed `SensitiveText`, `textora-sync`, existing app reducer and overlay pipeline.

## Global Constraints

- Product name is `textora`; the Markdown package remains `textora-markdown`.
- `crates/ui` must not depend on app, `DocumentView`, Keychain, worker objects, or Syncthing REST DTOs.
- “设置” is the only user-visible Syncthing entry; no nested or standalone sync overlay remains.
- The fourth category migrates all current panel capabilities without adding Web UI, disconnect, or Device-ID-copy features.
- API Key must not appear in `SyncSettingsInput`, DrawList, notice text, logs, or ordinary `Debug` output.
- UI-thread code must not perform HTTP, Keychain, directory scanning, file reads, or hashing.
- Every task modifies at most three files.
- Every behavior change follows RED → GREEN → refactor; observe the intended failing test before implementation.
- Before every commit, run the task’s focused tests and `cargo check -p textora-app`.
- Preserve unrelated worktree changes; stage only files listed by the current task.
- Final verification is `./scripts/verify.sh`.
- Approved specification: `docs/superpowers/specs/2026-07-20-syncthing-settings-integration-design.md`.

---

## File Structure

- `crates/ui/src/widgets/settings_view/sync_types.rs`: pure sync settings input, state, notice, library, and action types.
- `crates/ui/src/widgets/settings_view/sync_page.rs`: embedded Form-based page, local drafts, secure control-action translation, focus, scrolling, and dynamic sections.
- `crates/ui/src/widgets/settings_view/types.rs`: fourth category and top-level `SettingsViewAction::Sync` routing type.
- `crates/ui/src/widgets/settings_view/widget.rs`: category navigation and active-page delegation.
- `crates/ui/src/widgets/form/view.rs`: section replacement that preserves scroll and focus during snapshot refresh.
- `crates/ui/src/widgets/text_box.rs`: masked edit actions use redacted `SensitiveText` and remain consumable inside UI.
- `crates/app/src/sync_view_model.rs`: `SyncControllerSnapshot` → `SyncSettingsInput`.
- `crates/app/src/settings_overlay.rs`: initial sync input and live embedded-page refresh.
- `crates/app/src/app_dispatch.rs`: `SyncSettingsAction` → controller / `rfd` / `AppEffect`.
- `crates/app/src/app_renderer.rs`: refresh embedded sync settings while the Settings overlay exists.
- Existing popup/action/chrome/UiShell files: remove the unreachable standalone panel chain in separately compiling tasks.

---

### Task 1: Define the pure sync settings contract and page shell

**Files:**
- Create: `crates/ui/src/widgets/settings_view/sync_types.rs`
- Create: `crates/ui/src/widgets/settings_view/sync_page.rs`
- Modify: `crates/ui/src/widgets/settings_view/mod.rs`

**Interfaces:**
- Consumes: `crate::core::widget::SensitiveText`.
- Produces: `SyncSettingsInput`, `SyncSettingsAction`, existing view enums under `ui::settings_view`, and `SyncSettingsPage::new(SyncSettingsInput)` / `set_input` / `input`.

- [ ] **Step 1: Write failing contract tests**

Add tests in `sync_types.rs` and `sync_page.rs`:

```rust
#[test]
fn sync_action_debug_redacts_api_key() {
    let action = SyncSettingsAction::ConfigureConnection {
        endpoint: "http://127.0.0.1:8384".to_owned(),
        api_key: SensitiveText::new("never-print-me".to_owned()),
    };
    let debug = format!("{action:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("never-print-me"));
}

#[test]
fn page_starts_with_pure_not_configured_input() {
    let page = SyncSettingsPage::new(SyncSettingsInput::default());
    assert_eq!(page.input().connection, SyncConnectionView::NotConfigured);
    assert!(!page.input().has_api_key);
}

#[test]
fn sync_settings_input_has_no_api_key_value_field() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/widgets/settings_view/sync_types.rs"
    ));
    assert!(!source.contains("pub api_key:"));
}
```

- [ ] **Step 2: Run RED tests**

Run: `cargo test -p textora-ui --lib -- sync_settings`

Expected: FAIL because `SyncSettingsAction`, `SyncSettingsInput`, and `SyncSettingsPage` do not exist.

- [ ] **Step 3: Implement the contract and minimal page shell**

Define these exact public shapes in `sync_types.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncConnectionView {
    NotConfigured,
    Connecting,
    Connected { device_id: String, version: String },
    AuthenticationRequired,
    Incompatible { found: String },
    Unavailable { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySyncState {
    Pending,
    Scanning,
    Syncing,
    UpToDate,
    Paused,
    AwaitingRemoteAcceptance,
    ConfigurationMismatch,
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryView {
    pub name: String,
    pub root_display: String,
    pub state: LibrarySyncState,
    pub can_repair: bool,
    pub can_remove_mapping: bool,
    pub can_unregister: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFolderView { pub folder_id: String, pub offered_by: String }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncNoticeSeverity { Info, Warning, Error }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncNoticeView { pub severity: SyncNoticeSeverity, pub message: String }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncSettingsInput {
    pub endpoint: String,
    pub has_api_key: bool,
    pub connection: SyncConnectionView,
    pub libraries: Vec<LibraryView>,
    pub pending_folders: Vec<PendingFolderView>,
    pub notices: Vec<SyncNoticeView>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncSettingsAction {
    TestConnection { endpoint: String, api_key: SensitiveText },
    ConfigureConnection { endpoint: String, api_key: SensitiveText },
    PublishLibrary {
        remote_device_id: String,
        remote_name: String,
        remote_addresses: Vec<String>,
    },
    AcceptRemoteLibrary { pending_index: usize },
    ScanLibrary { library_index: usize },
    SetLibraryPaused { library_index: usize, paused: bool },
    RepairLibrary { library_index: usize },
    RemoveLibraryMapping { library_index: usize },
    UnregisterLibrary { library_index: usize },
}
```

Implement `Default` for `SyncSettingsInput` with `NotConfigured` and empty collections. In `sync_page.rs`, add the minimal input-owning shell. In `mod.rs`, declare both modules and re-export their public types.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-ui --lib -- sync_settings`

Expected: PASS for both new tests.

Run: `cargo check -p textora-app`

Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/settings_view/sync_types.rs \
        crates/ui/src/widgets/settings_view/sync_page.rs \
        crates/ui/src/widgets/settings_view/mod.rs
git commit -m "feat(ui): define Syncthing settings contract"
```

---

### Task 2: Preserve FormView position during snapshot section replacement

**Files:**
- Modify: `crates/ui/src/widgets/form/view.rs`

**Interfaces:**
- Consumes: existing `FormView::set_sections` and focus state.
- Produces: `FormView::replace_sections_preserving_state(Vec<FormSection>, &mut LayoutCtx)`.

- [ ] **Step 1: Write the failing state-preservation test**

```rust
#[test]
fn replacing_sections_preserves_scroll_and_focus() {
    let mut view = tall_form_view();
    layout_form_view(&mut view, Rect::new(0.0, 0.0, 400.0, 120.0));
    scroll_form_view(&mut view, 96.0);
    view.set_keyboard_focus(Some(SECOND_FIELD_ID));
    let previous_scroll = view.scroll_offset();

    let mut ctx = test_layout_ctx();
    view.replace_sections_preserving_state(replacement_sections(), &mut ctx);

    assert_eq!(view.focused_id(), Some(SECOND_FIELD_ID));
    assert_eq!(view.scroll_offset(), previous_scroll);
}
```

Expose a test-only or public `focused_id()` accessor matching the existing `scroll_offset()` style.

- [ ] **Step 2: Run RED test**

Run: `cargo test -p textora-ui --lib -- replacing_sections_preserves_scroll_and_focus`

Expected: FAIL because the replacement API does not exist.

- [ ] **Step 3: Implement state-preserving replacement**

```rust
pub fn replace_sections_preserving_state(
    &mut self,
    sections: Vec<FormSection>,
    ctx: &mut LayoutCtx,
) {
    let previous_scroll = self.scroll_offset;
    let previous_focus = self.focused_id;
    self.sections = sections;
    self.pointer_section_index = None;
    self.layout_sections(ctx);
    self.scroll_offset = previous_scroll.clamp(0.0, self.max_scroll_offset());
    self.set_keyboard_focus(previous_focus);
}
```

Do not change `set_sections`; category changes must continue resetting scroll.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-ui --lib -- replacing_sections_preserves_scroll_and_focus`

Run: `cargo test -p textora-ui --lib -- form_view`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/form/view.rs
git commit -m "feat(ui): preserve form state during content refresh"
```

---

### Task 3: Emit redacted edit payloads from masked TextBox

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`

**Interfaces:**
- Consumes: `TextPayload::Sensitive(SensitiveText)`.
- Produces: masked `TextEdited` actions that can be consumed inside `SyncSettingsPage` without exposing plaintext through Debug.

- [ ] **Step 1: Write the failing masked-edit test**

```rust
#[test]
fn masked_text_edit_emits_only_sensitive_payload() {
    let mut text_box = TextBox::with_id(API_KEY_ID);
    text_box.set_echo_mode(EchoMode::Masked);
    text_box.set_focus(true);
    text_box.sync_text("never-print-m");
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    let action = text_box.on_event(
        &Event::KeyDown(KeyCode::Char('e'), Modifiers::NONE),
        &mut event_ctx,
    );

    assert!(matches!(
        action,
        Some(WidgetAction::Control(ControlAction::TextEdited {
            id: API_KEY_ID,
            value: TextPayload::Sensitive(_),
        }))
    ));
    assert!(!format!("{action:?}").contains("never-print-me"));
}
```

- [ ] **Step 2: Run RED test**

Run: `cargo test -p textora-ui --lib -- masked_text_edit_emits_only_sensitive_payload`

Expected: FAIL because masked edits currently return no `TextEdited` action.

- [ ] **Step 3: Implement secure masked edit actions**

Change only the masked branch of the edit-action builder:

```rust
EchoMode::Masked => Some(ControlAction::TextEdited {
    id,
    value: TextPayload::Sensitive(SensitiveText::new(self.text.clone())),
}),
```

Keep drawing masked and keep committed payloads sensitive.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-ui --lib -- masked_text`

Run: `cargo check -p textora-app`

Expected: all exit 0 and no test output contains plaintext.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat(ui): emit redacted masked text edits"
```

---

### Task 4: Implement the Form-based SyncSettingsPage and app action route

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/sync_page.rs`
- Modify: `crates/ui/src/widgets/settings_view/types.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**
- Consumes: Tasks 1–3 contract, `FormView::replace_sections_preserving_state`, and existing controller methods.
- Produces: `SettingsViewAction::Sync(SyncSettingsAction)` and `App::dispatch_sync_settings_action`.

- [ ] **Step 1: Write failing page action, layout, and app dispatch tests**

Add focused tests covering these exact behaviors:

```rust
#[test]
fn configure_button_emits_redacted_sync_settings_action() {
    let mut page = SyncSettingsPage::new(SyncSettingsInput::default());
    page.handle_control_action(ControlAction::TextEdited {
        id: ENDPOINT_ID,
        value: TextPayload::Plain("http://127.0.0.1:8384".to_owned()),
    });
    page.handle_control_action(ControlAction::TextEdited {
        id: API_KEY_ID,
        value: TextPayload::Sensitive(SensitiveText::new("never-print-me".to_owned())),
    });

    let action = page.handle_control_action(ControlAction::Activated {
        id: CONFIGURE_CONNECTION_ID,
    });
    assert!(matches!(
        action,
        Some(WidgetAction::Settings(SettingsViewAction::Sync(
            SyncSettingsAction::ConfigureConnection { .. }
        )))
    ));
    assert!(!format!("{action:?}").contains("never-print-me"));
}

#[test]
fn snapshot_refresh_preserves_drafts_focus_and_scroll() {
    let mut page = laid_out_scrollable_page();
    page.handle_control_action(ControlAction::TextEdited {
        id: API_KEY_ID,
        value: TextPayload::Sensitive(SensitiveText::new("draft-key".to_owned())),
    });
    page.set_keyboard_focus(Some(API_KEY_ID));
    page.scroll_for_test(96.0);
    let previous_scroll = page.scroll_offset();

    page.set_input(connected_input_with_two_libraries());
    page.rebuild_for_test();

    assert_eq!(page.focused_id(), Some(API_KEY_ID));
    assert_eq!(page.scroll_offset(), previous_scroll);
    assert_eq!(page.api_key_draft_for_test(), Some("draft-key"));
}

#[test]
fn dynamic_rows_and_notices_have_distinct_vertical_positions() {
    let page = laid_out_page(sync_input_with_two_pending_two_libraries_two_notices());
    let rows = page.dynamic_row_rects_for_test();
    assert_eq!(rows.len(), 6);
    assert!(rows.windows(2).all(|pair| pair[0].y < pair[1].y));
}

fn laid_out_page(input: SyncSettingsInput) -> SyncSettingsPage {
    let theme = crate::theme::test_theme();
    let mut measure = crate::core::measure::NoopMeasure;
    let mut layout = LayoutCtx {
        measure: &mut measure,
        ui_measure: None,
        theme: &theme,
        dpi: 1.0,
    };
    let mut page = SyncSettingsPage::new(input);
    page.set_rect(Rect::new(0.0, 0.0, 520.0, 260.0), &mut layout);
    page
}

fn laid_out_scrollable_page() -> SyncSettingsPage {
    laid_out_page(sync_input_with_two_pending_two_libraries_two_notices())
}

fn connected_input_with_two_libraries() -> SyncSettingsInput {
    SyncSettingsInput {
        endpoint: "http://127.0.0.1:8384".to_owned(),
        has_api_key: true,
        connection: SyncConnectionView::Connected {
            device_id: "LOCAL-DEVICE".to_owned(),
            version: "2.1.1".to_owned(),
        },
        libraries: vec![
            test_library("Notes", "/tmp/notes"),
            test_library("Archive", "/tmp/archive"),
        ],
        pending_folders: Vec::new(),
        notices: Vec::new(),
    }
}

fn sync_input_with_two_pending_two_libraries_two_notices() -> SyncSettingsInput {
    let mut input = connected_input_with_two_libraries();
    input.pending_folders = vec![
        PendingFolderView { folder_id: "incoming-a".to_owned(), offered_by: "REMOTE-A".to_owned() },
        PendingFolderView { folder_id: "incoming-b".to_owned(), offered_by: "REMOTE-B".to_owned() },
    ];
    input.notices = vec![
        SyncNoticeView { severity: SyncNoticeSeverity::Info, message: "同步完成".to_owned() },
        SyncNoticeView { severity: SyncNoticeSeverity::Warning, message: "需要刷新".to_owned() },
    ];
    input
}

fn test_library(name: &str, root_display: &str) -> LibraryView {
    LibraryView {
        name: name.to_owned(),
        root_display: root_display.to_owned(),
        state: LibrarySyncState::UpToDate,
        can_repair: false,
        can_remove_mapping: true,
        can_unregister: true,
    }
}

#[test]
fn sync_settings_action_reaches_existing_controller_validation() {
    let mut app = App::new(None);
    let effect = app.dispatch_settings_view_action(SettingsViewAction::Sync(
        SyncSettingsAction::TestConnection {
            endpoint: "https://example.com".to_owned(),
            api_key: SensitiveText::new("secret".to_owned()),
        },
    ));
    assert!(effect.redraw);
}
```

- [ ] **Step 2: Run RED tests**

Run: `cargo test -p textora-ui --lib -- sync_settings_page`

Run: `cargo test -p textora-app --lib -- sync_settings_action`

Expected: FAIL because the embedded widget, top-level action variant, and app route do not exist.

- [ ] **Step 3: Implement local drafts and Form sections**

Use a single draft struct with precise fields:

```rust
#[derive(Default)]
struct SyncSettingsDraft {
    endpoint: String,
    api_key: Option<SensitiveText>,
    remote_device_id: String,
    remote_name: String,
    remote_addresses: String,
}
```

`SyncSettingsPage` owns `input`, `draft`, `FormView`, `form_needs_rebuild`, and stable `WidgetId` constants. Declare field IDs needed by `SettingsView` focus tests as `pub(super)`, including `API_KEY_ID`. Build these sections explicitly: connection, publish, pending folders, registered libraries, notices. Use index-derived IDs through named base constants and checked `u64` addition. Map control actions as follows:

```rust
ControlAction::TextEdited { id: API_KEY_ID, value: TextPayload::Sensitive(value) } => {
    self.draft.api_key = Some(value);
    Some(WidgetAction::Consumed)
}
ControlAction::Activated { id: CONFIGURE_CONNECTION_ID } => {
    let api_key = self.draft.api_key.take().unwrap_or_else(|| SensitiveText::new(String::new()));
    self.form_needs_rebuild = true;
    Some(WidgetAction::Settings(SettingsViewAction::Sync(
        SyncSettingsAction::ConfigureConnection {
            endpoint: self.draft.endpoint.clone(),
            api_key,
        },
    )))
}
```

Implement every remaining activation arm explicitly:

```rust
TEST_CONNECTION_ID => SyncSettingsAction::TestConnection {
    endpoint: self.draft.endpoint.clone(),
    api_key: self.take_api_key_draft(),
},
PUBLISH_LIBRARY_ID => SyncSettingsAction::PublishLibrary {
    remote_device_id: self.draft.remote_device_id.clone(),
    remote_name: self.draft.remote_name.clone(),
    remote_addresses: parse_remote_addresses(&self.draft.remote_addresses),
},
id if pending_index_from_id(id).is_some() => SyncSettingsAction::AcceptRemoteLibrary {
    pending_index: pending_index_from_id(id).expect("pending ID was checked before mapping"),
},
id if scan_index_from_id(id).is_some() => SyncSettingsAction::ScanLibrary {
    library_index: scan_index_from_id(id).expect("scan ID was checked before mapping"),
},
id if pause_index_from_id(id).is_some() => {
    let library_index = pause_index_from_id(id).expect("pause ID was checked before mapping");
    SyncSettingsAction::SetLibraryPaused {
        library_index,
        paused: !matches!(self.input.libraries[library_index].state, LibrarySyncState::Paused),
    }
}
id if repair_index_from_id(id).is_some() => SyncSettingsAction::RepairLibrary {
    library_index: repair_index_from_id(id).expect("repair ID was checked before mapping"),
},
id if remove_index_from_id(id).is_some() => SyncSettingsAction::RemoveLibraryMapping {
    library_index: remove_index_from_id(id).expect("remove ID was checked before mapping"),
},
id if unregister_index_from_id(id).is_some() => SyncSettingsAction::UnregisterLibrary {
    library_index: unregister_index_from_id(id)
        .expect("unregister ID was checked before mapping"),
},
```

`set_input` updates endpoint only when the endpoint control is not focused and rebuilds through `replace_sections_preserving_state`.

- [ ] **Step 4: Add top-level action and reuse controller behavior**

In `types.rs` add `SettingsCategory::Sync` after `Interface` and add
`SettingsViewAction::Sync(SyncSettingsAction)`. In `app_dispatch.rs`, add this match arm:

```rust
SettingsViewAction::Sync(action) => return self.dispatch_sync_settings_action(action),
```

Implement `dispatch_sync_settings_action` by porting the non-`Close` arms of `dispatch_sync_panel_action`. Convert secrets only at the controller boundary:

```rust
let api_key = api_key.expose().to_owned();
controller.configure_connection(endpoint, api_key)
```

Keep the legacy panel dispatcher temporarily as a thin compatibility adapter; it is deleted in Task 9.

- [ ] **Step 5: Verify GREEN and compile**

Run: `cargo test -p textora-ui --lib -- sync_settings_page`

Run: `cargo test -p textora-app --lib -- sync_settings_action`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/widgets/settings_view/sync_page.rs \
        crates/ui/src/widgets/settings_view/types.rs \
        crates/app/src/app_dispatch.rs
git commit -m "feat(settings): implement Syncthing settings page"
```

---

### Task 5: Add the fourth category and delegate SettingsView behavior

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`

**Interfaces:**
- Consumes: `SyncSettingsPage` and `SettingsCategory::Sync` from Task 4.
- Produces: `SettingsView::set_sync_input`, four-item category navigation, and active-page paint/event/focus delegation.

- [ ] **Step 1: Write failing category and delegation tests**

```rust
#[test]
fn settings_view_exposes_sync_as_fourth_category() {
    let mut view = settings_fixture(SettingsCategory::Sync);
    let categories: Vec<SettingsCategory> =
        view.category_buttons.iter().map(|(category, _)| *category).collect();
    assert_eq!(
        categories,
        vec![
            SettingsCategory::Appearance,
            SettingsCategory::Editor,
            SettingsCategory::Interface,
            SettingsCategory::Sync,
        ],
    );
    assert!(view.category_is_selected(SettingsCategory::Sync));
}

#[test]
fn sync_category_delegates_wheel_keyboard_and_ime_to_sync_page() {
    let mut view = settings_fixture(SettingsCategory::Sync);
    layout_settings_view(&mut view, &test_theme(), Rect::new(0.0, 0.0, 720.0, 320.0));
    view.set_keyboard_focus(Some(API_KEY_ID));
    let before_scroll = view.sync_scroll_offset_for_test();

    dispatch_settings_event(
        &mut view,
        Event::Wheel { dx: 0.0, dy: 80.0, px: 500.0, py: 250.0 },
    );
    dispatch_settings_event(&mut view, Event::ImeCommit("密钥".to_owned()));

    assert!(view.sync_scroll_offset_for_test() > before_scroll);
    assert_eq!(view.sync_api_key_draft_for_test(), Some("密钥"));
}

fn dispatch_settings_event(
    view: &mut SettingsView,
    event: Event,
) -> Option<WidgetAction> {
    let theme = crate::theme::test_theme();
    let mut ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
    view.on_event(&event, &mut ctx)
}
```

- [ ] **Step 2: Run RED tests**

Run: `cargo test -p textora-ui --lib -- settings_view_exposes_sync`

Expected: FAIL because navigation has only three categories.

- [ ] **Step 3: Implement fourth-category routing**

Add `SYNC_CATEGORY_ID`, a `sync_page: SyncSettingsPage` field, and:

```rust
pub fn set_sync_input(&mut self, input: SyncSettingsInput) {
    self.sync_page.set_input(input);
}

pub fn sync_input(&self) -> &SyncSettingsInput {
    self.sync_page.input()
}
```

When `active_category == SettingsCategory::Sync`, layout, paint, focus collection, capture checks, and events target `sync_page`; otherwise retain the existing `FormView` path. Category activation resets the newly active page scroll and focuses its first enabled field. Add `#[cfg(test)]` delegating accessors for sync scroll offset and API-key draft so the routing test reads page state without exposing secrets in production APIs.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-ui --lib -- settings_view`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/settings_view/widget.rs
git commit -m "feat(settings): add Syncthing category"
```

---

### Task 6: Map controller snapshots and inject initial sync input

**Files:**
- Modify: `crates/app/src/sync_view_model.rs`
- Modify: `crates/app/src/settings_overlay.rs`

**Interfaces:**
- Consumes: `SyncControllerSnapshot`, `SyncNotice`, and `SyncSettingsInput`.
- Produces: `build_sync_settings_input`, `empty_sync_settings_input`, and initial `SettingsView::set_sync_input` injection.

- [ ] **Step 1: Write failing mapping and initial-overlay tests**

```rust
#[test]
fn view_model_builds_sync_settings_input_without_api_key() {
    let input = build_sync_settings_input(
        &snapshot(SyncConnectionState::NotConfigured),
        &[],
    );
    assert_eq!(input.endpoint, "http://127.0.0.1:8384");
    assert!(input.has_api_key);
    assert_eq!(input.connection, SyncConnectionView::NotConfigured);
}

#[test]
fn opening_settings_injects_sync_snapshot_into_embedded_page() {
    let mut app = App::new(None);
    app.open_settings_overlay();
    let view = active_settings_view(&mut app);
    assert_eq!(view.sync_input().connection, SyncConnectionView::NotConfigured);
}

fn active_settings_view(app: &mut App) -> &mut ui::settings_view::SettingsView {
    app.ui_shell
        .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
        .expect("settings overlay should use ModalFrame")
        .content_as_any_mut()
        .downcast_mut::<ui::settings_view::SettingsView>()
        .expect("modal content should be SettingsView")
}
```

- [ ] **Step 2: Run RED tests**

Run: `cargo test -p textora-app --lib -- sync_settings_input`

Expected: FAIL because the new builder and initial injection do not exist.

- [ ] **Step 3: Implement mapping and initial injection**

Rename the primary mapper to:

```rust
pub(crate) fn build_sync_settings_input(
    snapshot: &SyncControllerSnapshot,
    notices: &[SyncNotice],
) -> ui::settings_view::SyncSettingsInput
```

Add `empty_sync_settings_input()`. Keep a temporary `build_sync_panel_input` compatibility mapper for `app_renderer` until Task 7. In `open_settings_overlay`, obtain notices and snapshot before borrowing `ui_shell`, construct `SettingsView`, call `set_sync_input`, and then wrap it in `ModalFrame`.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-app --lib -- sync_settings_input`

Run: `cargo test -p textora-app --lib -- opening_preferences_creates_one_modal_settings_overlay`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/sync_view_model.rs crates/app/src/settings_overlay.rs
git commit -m "feat(app): inject Syncthing settings state"
```

---

### Task 7: Refresh embedded sync settings from live controller state

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/settings_overlay.rs`
- Modify: `crates/app/src/sync_view_model.rs`

**Interfaces:**
- Consumes: active `ModalFrame`/`SettingsView` downcast and controller notices.
- Produces: `App::refresh_sync_settings_overlay()`; removes the legacy renderer mapper.

- [ ] **Step 1: Write the failing live-refresh test**

```rust
#[test]
fn sync_results_refresh_embedded_page_without_creating_overlay() {
    let mut app = App::new(None);
    app.sync_controller = Some(crate::sync_controller::SyncController::new_default(|| {}));
    app.open_settings_overlay();
    app.refresh_sync_settings_overlay();
    assert_eq!(app.ui_shell.overlays_count(), 1);
    assert_eq!(
        active_settings_view(&mut app).sync_input().connection,
        SyncConnectionView::Connecting,
    );
}
```

- [ ] **Step 2: Run RED test**

Run: `cargo test -p textora-app --lib -- sync_results_refresh_embedded_page`

Expected: FAIL because refresh still targets `SyncPanelWidget`.

- [ ] **Step 3: Implement embedded refresh**

Add a helper that first checks for an active Settings overlay, then drains notices/builds input, then re-borrows the active `SettingsView` and calls `set_sync_input`. Delete the complete existing `if self.ui_shell.sync_panel_is_open()` block from the renderer and replace it with:

```rust
self.refresh_sync_settings_overlay();
```

Remove the temporary `build_sync_panel_input` compatibility mapper once no call sites remain.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-app --lib -- sync_results_refresh_embedded_page`

Run: `cargo test -p textora-app --lib -- sync_view_model`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/app_renderer.rs \
        crates/app/src/settings_overlay.rs \
        crates/app/src/sync_view_model.rs
git commit -m "feat(app): refresh embedded Syncthing settings"
```

---

### Task 8: Remove the unreachable popup entry and event translation

**Files:**
- Modify: `crates/ui/src/widgets/popup_menu/types.rs`
- Modify: `crates/ui/src/widgets/sidebar/menu.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**
- Consumes: fourth Settings category as the replacement entry.
- Produces: no `PopupMenuAction::OpenSyncPanel` and no popup-to-open-panel translation.

- [ ] **Step 1: Replace the old menu test with a failing absence test**

```rust
#[test]
fn settings_menu_does_not_expose_a_second_sync_entry() {
    let settings = crate::settings::Settings::new();
    let metrics = crate::settings::UiMetrics::from_settings(&settings, 1.0);
    let menu = build_settings_menu(
        None,
        &SidebarSettingsInput::default(),
        800.0,
        600.0,
        &metrics,
    )
    .expect("settings menu should be constructed");
    assert!(menu.items.iter().all(|item| item.label != "打开同步面板"));
}
```

Also add a source/action exhaustiveness assertion in `events.rs` proving sidebar settings still maps only to `OpenSidebarSettingsMenu`.

- [ ] **Step 2: Run RED test**

Run: `cargo test -p textora-ui --lib -- settings_menu_does_not_expose_a_second_sync_entry`

Expected: FAIL because the old item remains.

- [ ] **Step 3: Remove popup action and translation**

Delete `OpenSyncPanel` from `PopupMenuAction`, delete the menu item, and delete its `translate_popup_action` arm. Replace the legacy widget translation with an explicit no-op until Task 11 removes the variant:

```rust
WidgetAction::SyncPanel(_) => {}
```

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-ui --lib -- sidebar::menu`

Run: `cargo test -p textora-app --lib -- translate_popup`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/popup_menu/types.rs \
        crates/ui/src/widgets/sidebar/menu.rs \
        crates/app/src/events.rs
git commit -m "refactor(settings): remove standalone sync menu entry"
```

---

### Task 9: Remove standalone app actions and chrome dispatch

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/dispatch/chrome.rs`

**Interfaces:**
- Consumes: `SettingsViewAction::Sync` as the only sync UI action route.
- Produces: no `AppAction::OpenSyncPanel`, `AppAction::SyncPanel`, `ChromeDispatchAction::OpenSyncPanel`, or `CloseSyncPanel`.

- [ ] **Step 1: Add a failing source-boundary test**

```rust
#[test]
fn standalone_sync_app_actions_are_removed() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"));
    assert!(!source.contains("OpenSyncPanel"));
    assert!(!source.contains("SyncPanel("));
}
```

- [ ] **Step 2: Run RED test**

Run: `cargo test -p textora-app --lib -- standalone_sync_app_actions_are_removed`

Expected: FAIL because both variants still exist.

- [ ] **Step 3: Delete legacy app/chrome routing**

Remove both `AppAction` variants and reducer arms. Delete both chrome variants, their match arms, `empty_sync_panel_input`, and the standalone open/close test. Delete `dispatch_sync_panel_action`; retain only `dispatch_sync_settings_action` and shared library/controller helpers.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-app --lib -- standalone_sync_app_actions_are_removed`

Run: `cargo test -p textora-app --lib -- sync_settings_action`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/actions.rs \
        crates/app/src/app_dispatch.rs \
        crates/app/src/dispatch/chrome.rs
git commit -m "refactor(app): remove standalone sync panel actions"
```

---

### Task 10: Retire the SyncPanelWidget runtime and UiShell special case

**Files:**
- Modify: `crates/ui/src/widgets/sync_panel.rs`
- Modify: `crates/app/src/ui_shell.rs`

**Interfaces:**
- Consumes: embedded page and app refresh path.
- Produces: no `SyncPanelWidget` or UiShell panel lifecycle/layout; the temporary action variant is removed in Task 11.

- [ ] **Step 1: Add failing source and overlay-count tests**

```rust
#[test]
fn ui_shell_has_no_sync_panel_overlay_lifecycle() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui_shell.rs"));
    assert!(!source.contains("open_sync_panel"));
    assert!(!source.contains("layout_sync_panel"));
}
```

Add a UI source test asserting `sync_panel.rs` has no `SyncPanelWidget` declaration.

```rust
#[test]
fn compatibility_module_contains_no_widget_runtime() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/widgets/sync_panel.rs"
    ));
    assert!(!source.contains("struct SyncPanelWidget"));
    assert!(!source.contains("impl Widget for SyncPanelWidget"));
}
```

- [ ] **Step 2: Run RED tests**

Run: `cargo test -p textora-app --lib -- ui_shell_has_no_sync_panel_overlay_lifecycle`

Expected: FAIL because standalone lifecycle methods remain.

- [ ] **Step 3: Remove runtime and leave a temporary compatibility module**

Replace `sync_panel.rs` with re-exports of the new pure settings types only; remove its widget implementation and tests. Remove the `SyncPanelWidget` import, special layout pass, lifecycle methods, focus routing, and standalone panel tests from `ui_shell.rs`.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-app --lib -- ui_shell_has_no_sync_panel_overlay_lifecycle`

Run: `cargo test -p textora-ui --lib -- sync_settings`

Run: `cargo check -p textora-app`

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/sync_panel.rs \
        crates/app/src/ui_shell.rs
git commit -m "refactor(ui): retire standalone sync panel runtime"
```

---

### Task 11: Remove the legacy WidgetAction and focus ID

**Files:**
- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**
- Consumes: Task 10, which removes the last `SyncPanelWidget` producer.
- Produces: no `WidgetAction::SyncPanel` and no `ids::SYNC_PANEL`.

- [ ] **Step 1: Add the failing action-boundary test**

Add this test to `events.rs`:

```rust
#[test]
fn widget_action_has_no_standalone_sync_variant() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ui/src/core/widget.rs"
    ));
    assert!(!source.contains("SyncPanel("));
    assert!(!source.contains("SYNC_PANEL"));
}
```

- [ ] **Step 2: Run RED test**

Run: `cargo test -p textora-app --lib -- widget_action_has_no_standalone_sync_variant`

Expected: FAIL because the variant and ID still exist.

- [ ] **Step 3: Remove the variant, ID, and temporary no-op arm**

Delete `ids::SYNC_PANEL` and `WidgetAction::SyncPanel` from `core/widget.rs`. Delete the temporary `WidgetAction::SyncPanel(_) => {}` translation and any fallthrough-consumption mention from `events.rs`.

- [ ] **Step 4: Verify GREEN and compile**

Run: `cargo test -p textora-app --lib -- widget_action_has_no_standalone_sync_variant`

Run: `cargo check -p textora-app`

Expected: both exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/core/widget.rs crates/app/src/events.rs
git commit -m "refactor(ui): remove standalone sync widget action"
```

---

### Task 12: Remove the compatibility module and verify the complete migration

**Files:**
- Delete: `crates/ui/src/widgets/sync_panel.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Consumes: all callers using `ui::settings_view` types.
- Produces: public UI surface with no `sync_panel` module.

- [ ] **Step 1: Add the final source-boundary check**

Before deletion, add this temporary test in `sync_panel.rs`. It is deleted with the compatibility module after observing RED:

```rust
#[test]
fn standalone_sync_module_is_absent() {
    let widgets_module = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/widgets/mod.rs"
    ));
    let ui_root = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"));
    assert!(!widgets_module.contains("pub mod sync_panel"));
    assert!(!ui_root.contains("sync_panel"));
}
```

- [ ] **Step 2: Run RED boundary check**

Run: `cargo test -p textora-ui --lib -- standalone_sync_module_is_absent`

Expected: FAIL because the compatibility module is still exported.

- [ ] **Step 3: Delete compatibility exports and file**

Remove `pub mod sync_panel` from `widgets/mod.rs`, remove `sync_panel` from the root `pub use widgets::{...}` list, and delete `sync_panel.rs`.

- [ ] **Step 4: Run focused and crate verification**

Run: `rg -n "sync_panel|SyncPanel|OpenSyncPanel|SYNC_PANEL" crates/ui/src crates/app/src`

Expected: no output.

Run: `cargo fmt --all -- --check`

Run: `cargo test -p textora-ui`

Run: `cargo test -p textora-app --lib`

Run: `cargo check -p textora-app`

Expected: all commands exit 0.

- [ ] **Step 5: Run major-change verification**

Run: `./scripts/verify.sh`

Expected: exit 0 with all repository checks passing.

- [ ] **Step 6: Review diff and commit**

Run: `git diff --check`

Run: `git status --short`

Confirm only the three task files are staged; unrelated pre-existing changes remain unstaged.

```bash
git add crates/ui/src/widgets/sync_panel.rs \
        crates/ui/src/widgets/mod.rs \
        crates/ui/src/lib.rs
git commit -m "refactor(ui): make settings the only sync entry"
```

---

## Plan Self-Review Checklist

- Spec coverage: fourth category, complete current panel capability, live refresh, scroll/focus preservation, sensitive API Key, and old-path deletion each have a task.
- File limit: every task modifies no more than three files.
- Type consistency: all new UI callers use `ui::settings_view::{SyncSettingsInput, SyncSettingsAction}`.
- Transitional compilation: compatibility mapper/module remain until their callers are removed.
- TDD: every behavior task defines a focused RED command before implementation.
- Final verification: both crate suites, app check, formatting, source search, and `./scripts/verify.sh` are required.
