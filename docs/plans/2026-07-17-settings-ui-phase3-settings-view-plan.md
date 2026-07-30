# Settings UI Phase 3: Real Settings View and Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the singleton macOS-style settings modal for real editor settings, with validation, immediate application, persistence, failure visibility, and retry.

**Architecture:** `SettingsView` owns three categories—Appearance, Editor, and Interface—and builds them from generic Form containers. It receives pure data and emits `SettingsViewAction`; app translates those actions into existing settings/effect machinery. Syncthing is explicitly deferred.

**Tech Stack:** Phase 1 controls, Phase 2 Form/Modal infrastructure, existing `ui::settings::Settings`, `PersistedSettings`, AppEffect, theme registry, and winit UI shell.

## Global Constraints

- Categories in this phase are exactly Appearance, Editor, and Interface.
- Do not create a Syncthing category, Syncthing view model, REST/Keychain dependency, or connection action.
- Settings apply immediately only after valid submission; incomplete TextBox edits remain local.
- Font size range is `6.0..=72.0`, line-height ratio is `1.0..=3.0`, and Tab width is `1..=16`.
- Theme and view mode use selected Button groups; boolean feature settings use Switch.
- Persistence failure does not roll back runtime state and is visible with a retry Button.
- Existing “Open settings.toml” action remains available through `AppAction::OpenSettingsFile`; native Preferences opens the new modal.
- Each task modifies at most three files and ends with a compiling commit.

---

### Task 1: Define pure SettingsView inputs, actions, and validation

**Files:**
- Create: `crates/ui/src/widgets/settings_view/types.rs`
- Create: `crates/ui/src/widgets/settings_view/mod.rs`
- Modify: `crates/ui/src/widgets/mod.rs`

**Interfaces:**
- Produces: `SettingsCategory`, `SettingsPersistenceView`, `SettingsViewInput`, `SettingsViewAction`, and typed validation functions.

- [ ] **Step 1: Write failing validation tests**

```rust
#[test]
fn numeric_settings_accept_only_documented_ranges() {
    assert_eq!(parse_font_size("6"), Ok(6.0));
    assert_eq!(parse_font_size("72"), Ok(72.0));
    assert_eq!(parse_font_size("5.9"), Err(ValidationError::OutOfRange));
    assert_eq!(parse_line_height_ratio("1.618"), Ok(1.618));
    assert_eq!(parse_tab_width("16"), Ok(16));
    assert_eq!(parse_tab_width("0"), Err(ValidationError::OutOfRange));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui numeric_settings_accept_only_documented_ranges`

Expected: FAIL because the settings_view module is absent.

- [ ] **Step 3: Implement pure types and validators**

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsCategory { #[default] Appearance, Editor, Interface }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SettingsPersistenceView {
    #[default]
    Saved,
    SaveFailed { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsViewInput {
    pub theme_mode: crate::settings::ThemeMode,
    pub font_family: String,
    pub font_size: f32,
    pub line_height_ratio: f32,
    pub word_wrap: bool,
    pub show_line_numbers: bool,
    pub tab_width: usize,
    pub view_mode: crate::view_mode::ViewMode,
    pub show_status_bar: bool,
    pub persistence: SettingsPersistenceView,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsViewAction {
    SetThemeMode(crate::settings::ThemeMode),
    SetFontFamily(String),
    SetFontSize(f32),
    SetLineHeightRatio(f32),
    SetWordWrap(bool),
    SetShowLineNumbers(bool),
    SetTabWidth(usize),
    SetViewMode(crate::view_mode::ViewMode),
    SetShowStatusBar(bool),
    RetryPersistence,
}
```

Implement parse functions in `types.rs` with trim, finite-number checks, exact ranges, and non-empty font-family validation. ValidationError contains stable UI-facing messages and no broad stringly state. Register `pub mod settings_view;` in `widgets/mod.rs` so the focused tests compile immediately.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui settings_view::types`

Expected: PASS.

```bash
git add crates/ui/src/widgets/settings_view/types.rs crates/ui/src/widgets/settings_view/mod.rs crates/ui/src/widgets/mod.rs
git commit -m "feat(ui): define settings view contract"
```

### Task 2: Build category navigation and appearance form

**Files:**
- Create: `crates/ui/src/widgets/settings_view/widget.rs`
- Modify: `crates/ui/src/widgets/settings_view/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

**Interfaces:**
- Consumes: Phase 1 controls, Phase 2 FormView, and Task 1 types.
- Produces: `SettingsView::new(input)` and appearance actions.

- [ ] **Step 1: Write failing category and appearance tests**

```rust
#[test]
fn appearance_category_uses_selected_buttons_and_validated_textboxes() {
    let mut view = settings_fixture(SettingsCategory::Appearance);
    assert!(view.category_button(SettingsCategory::Appearance).is_selected());
    assert_eq!(activate_theme(&mut view, ThemeMode::Dark), SettingsViewAction::SetThemeMode(ThemeMode::Dark));
    assert_eq!(commit_field(&mut view, FONT_SIZE_ID, "18"), SettingsViewAction::SetFontSize(18.0));
    assert!(commit_field_result(&mut view, FONT_SIZE_ID, "999").is_validation_error());
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui appearance_category_uses_selected_buttons_and_validated_textboxes`

Expected: FAIL because SettingsView is absent.

- [ ] **Step 3: Implement SettingsView shell and appearance section**

```rust
pub struct SettingsView {
    rect: Rect,
    input: SettingsViewInput,
    active_category: SettingsCategory,
    category_buttons: Vec<Button>,
    form: FormView,
    validation: Option<FieldValidation>,
}
```

Use private stable WidgetId constants for category, theme, font-family, font-size, and line-height controls. Build Appearance from generic FormSection/FormRow/InlineGroup. Intercept child ControlAction, validate text commits, and return `WidgetAction::Settings(SettingsViewAction)`; category selection rebuilds sections and resets FormView scroll.

Add `WidgetAction::Settings(crate::widgets::settings_view::SettingsViewAction)` in `core/widget.rs`. Category Button activation is handled entirely inside SettingsView and does not emit a business action to app.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui settings_view::widget::tests::appearance`

Expected: PASS.

```bash
git add crates/ui/src/widgets/settings_view/widget.rs crates/ui/src/widgets/settings_view/mod.rs crates/ui/src/core/widget.rs
git commit -m "feat(ui): build appearance settings form"
```

### Task 3: Add Editor and Interface forms

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`

**Interfaces:**
- Produces: all remaining SettingsViewAction variants from real controls.

- [ ] **Step 1: Write failing action-mapping tests**

```rust
#[test]
fn editor_and_interface_controls_emit_typed_actions() {
    let mut editor = settings_fixture(SettingsCategory::Editor);
    assert_eq!(toggle(&mut editor, WORD_WRAP_ID, false), SettingsViewAction::SetWordWrap(false));
    assert_eq!(toggle(&mut editor, LINE_NUMBERS_ID, false), SettingsViewAction::SetShowLineNumbers(false));
    assert_eq!(commit(&mut editor, TAB_WIDTH_ID, "8"), SettingsViewAction::SetTabWidth(8));

    let mut interface = settings_fixture(SettingsCategory::Interface);
    assert_eq!(activate(&mut interface, VIEW_TABS_ID), SettingsViewAction::SetViewMode(ViewMode::Tabs));
    assert_eq!(toggle(&mut interface, STATUS_BAR_ID, true), SettingsViewAction::SetShowStatusBar(true));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui editor_and_interface_controls_emit_typed_actions`

Expected: FAIL because those forms are not built.

- [ ] **Step 3: Implement both category builders**

```rust
fn build_editor_sections(input: &SettingsViewInput) -> Vec<FormSection> {
    vec![FormSection::new(
        "编辑器",
        vec![word_wrap_row(input), line_numbers_row(input), tab_width_row(input)],
    )]
}

fn build_interface_sections(input: &SettingsViewInput) -> Vec<FormSection> {
    vec![FormSection::new(
        "界面",
        vec![view_mode_row(input), status_bar_row(input)],
    )]
}
```

Keep each builder below 50 lines by extracting one helper per row. Use Switch for word wrap, line numbers, and status bar; TextBox for Tab width; selected Buttons for view mode. No Checkbox is forced into these settings because none has checkbox semantics.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui settings_view`

Expected: PASS.

```bash
git add crates/ui/src/widgets/settings_view/widget.rs
git commit -m "feat(ui): add editor and interface settings forms"
```

### Task 4: Add persistence-failure presentation and public exports

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: public `ui::settings_view` module and retry action.

- [ ] **Step 1: Write failing failure-banner test**

```rust
#[test]
fn save_failure_shows_icon_label_and_retry_button() {
    let mut input = input_fixture();
    input.persistence = SettingsPersistenceView::SaveFailed { message: "permission denied".into() };
    let mut view = SettingsView::new(input);
    assert!(view.visible_text().contains("当前修改尚未保存"));
    assert_eq!(click_retry(&mut view), SettingsViewAction::RetryPersistence);
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui save_failure_shows_icon_label_and_retry_button`

Expected: FAIL because no banner exists.

- [ ] **Step 3: Add the generic composition and exports**

Compose the banner from Label with warning icon plus ordinary Button in InlineGroup. Export `settings_view` from widgets and crate root; do not create a failure-specific widget.

```rust
fn unsaved_changes_label() -> Label {
    let mut label = Label::new("当前修改尚未保存", LabelStyle::default());
    label.set_leading_icon(Some("warning".into()));
    label
}

fn persistence_banner(input: &SettingsViewInput) -> Option<InlineGroup> {
    match &input.persistence {
        SettingsPersistenceView::Saved => None,
        SettingsPersistenceView::SaveFailed { .. } => Some(InlineGroup::new(vec![
            InlineChild::content(Box::new(unsaved_changes_label()), UNSAVED_LABEL_WIDTH_LOGICAL),
            InlineChild::fixed(Box::new(retry_button()), RETRY_BUTTON_WIDTH_LOGICAL),
        ])),
    }
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui settings_view`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/ui/src/widgets/settings_view/widget.rs crates/ui/src/lib.rs
git commit -m "feat(ui): expose settings view"
```

### Task 5: Lock the public UI boundary

**Files:**
- Modify: `crates/ui/tests/public_api.rs`
- Modify: `crates/ui/tests/public_boundaries.rs`

**Interfaces:**
- Verifies: public construction and absence of app/Syncthing dependencies.

- [ ] **Step 1: Add failing external-consumer assertions**

```rust
#[test]
fn settings_foundation_is_public_without_business_dependencies() {
    assert_widget::<ui::label::Label>();
    assert_widget::<ui::button::Button>();
    assert_widget::<ui::switch::Switch>();
    assert_widget::<ui::checkbox::Checkbox>();
    assert_widget::<ui::settings_view::SettingsView>();
}
```

Add source-boundary assertions rejecting `DocumentView`, `textora_sync`, `SyncthingClient`, and `Keychain` under `crates/ui/src`.

- [ ] **Step 2: Run and observe any export gaps**

Run: `cargo test -p textora-ui --test public_api --test public_boundaries`

Expected: FAIL until all intended paths and forbidden-name checks are correct.

- [ ] **Step 3: Correct only the public paths/boundary list**

Use the exports introduced in Task 4. Do not re-export app-only types.

```rust
pub use widgets::{
    button, checkbox, form, inline_group, label, modal_frame, settings_view, switch, text_box,
};
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui --test public_api --test public_boundaries`

Expected: PASS.

```bash
git add crates/ui/tests/public_api.rs crates/ui/tests/public_boundaries.rs
git commit -m "test(ui): lock settings component boundaries"
```

### Task 6: Open the modal from native Preferences

**Files:**
- Create: `crates/app/src/settings_overlay.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/dispatch/commands.rs`

**Interfaces:**
- Consumes: `SettingsView`, `ModalFrame`, and policy-bearing `UiShell::push_overlay`.
- Produces: `App::open_settings_overlay()` and `App::refresh_settings_overlay()`.

- [ ] **Step 1: Write failing singleton/open tests in `settings_overlay.rs`**

```rust
#[test]
fn opening_preferences_creates_one_modal_settings_overlay() {
    let mut app = App::new(None);
    app.open_settings_overlay();
    app.open_settings_overlay();
    assert_eq!(app.ui_shell.overlays_count(), 1);
    assert!(app.ui_shell.active_overlay_is_modal());
    let frame = app
        .ui_shell
        .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
        .expect("settings overlay must use ModalFrame");
    assert!(frame
        .content_as_any_mut()
        .downcast_mut::<ui::settings_view::SettingsView>()
        .is_some());
}
```

- [ ] **Step 2: Run and observe current file-opening behavior**

Run: `cargo test -p textora-app --lib -- opening_preferences_creates_one_modal_settings_overlay`

Expected: FAIL because Preferences opens settings.toml and no helper exists.

- [ ] **Step 3: Implement input mapping and modal construction**

```rust
impl App {
    pub(crate) fn settings_view_input(&self) -> ui::settings_view::SettingsViewInput {
        ui::settings_view::SettingsViewInput {
            theme_mode: self.settings.theme_mode,
            font_family: self.settings.font_family.clone(),
            font_size: self.settings.font_size,
            line_height_ratio: self.settings.line_height_ratio,
            word_wrap: self.settings.word_wrap,
            show_line_numbers: self.settings.show_line_numbers,
            tab_width: self.settings.tab_width,
            view_mode: self.settings.view_mode,
            show_status_bar: self.settings.show_status_bar,
            persistence: ui::settings_view::SettingsPersistenceView::Saved,
        }
    }

    pub(crate) fn open_settings_overlay(&mut self) -> AppEffect {
        let view = ui::settings_view::SettingsView::new(self.settings_view_input());
        let frame = ui::modal_frame::ModalFrame::new("设置", Box::new(view));
        self.ui_shell.push_overlay_with_policy(
            Box::new(frame),
            ui::OverlayLayout::Centered {
                preferred_size: (900.0, 640.0),
                min_margin: 24.0,
                max_width_ratio: 0.92,
                max_height_ratio: 0.90,
            },
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );
        AppEffect::REDRAW
    }

    pub(crate) fn refresh_settings_overlay(&mut self) {
        let input = self.settings_view_input();
        let Some(frame) = self.ui_shell.active_overlay_widget_mut::<ui::modal_frame::ModalFrame>() else { return; };
        let Some(view) = frame.content_as_any_mut().downcast_mut::<ui::settings_view::SettingsView>() else { return; };
        view.set_input(input);
    }
}
```

Change `AppCommand::OpenSettings` to call this method. Keep `OpenSettingsFile` behavior untouched for the existing sidebar/popup action.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- settings_overlay`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/app/src/settings_overlay.rs crates/app/src/lib.rs crates/app/src/dispatch/commands.rs
git commit -m "feat(app): open modal settings view"
```

### Task 7: Translate SettingsView actions into app intents

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/actions.rs`

**Interfaces:**
- Consumes: `WidgetAction::Settings(SettingsViewAction)` from Task 2.
- Produces: `AppAction::Settings(SettingsViewAction)`.

- [ ] **Step 1: Write failing exhaustive translation test**

```rust
#[test]
fn every_settings_view_action_translates_once_and_consumes_input() {
    for action in settings_action_fixtures() {
        let (app_actions, consumed) = translate_fixture(WidgetAction::Settings(action.clone()));
        assert!(consumed);
        assert_eq!(app_actions.len(), 1);
        assert!(matches!(&app_actions[0], AppAction::Settings(mapped) if mapped == &action));
    }
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-app --lib -- every_settings_view_action_translates_once_and_consumes_input`

Expected: FAIL because the WidgetAction/AppAction variants are absent.

- [ ] **Step 3: Add pure exhaustive translation**

Add the variants, mark Settings actions consumed in mouse dispatch, and map them without mutating Settings or performing persistence inside `events.rs`.

```rust
WidgetAction::Settings(action) => {
    actions.push(AppAction::Settings(action.clone()));
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- translate_settings`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/app/src/events.rs crates/app/src/actions.rs
git commit -m "feat(app): translate settings view actions"
```

### Task 8: Apply value-based settings actions with correct effects

**Files:**
- Modify: `crates/app/src/dispatch/chrome.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/ui/src/settings.rs`

**Interfaces:**
- Consumes: `AppAction::Settings`.
- Produces: value-based SettingsDispatchAction variants and exact AppEffects.

- [ ] **Step 1: Write failing effect tests**

```rust
#[test]
fn settings_actions_apply_values_and_return_required_effects() {
    let mut app = App::new(None);
    assert!(app.dispatch_settings_view_action(SettingsViewAction::SetFontSize(18.0)).reshape);
    assert_eq!(app.settings.font_size, 18.0);
    assert!(app.dispatch_settings_view_action(SettingsViewAction::SetThemeMode(ThemeMode::Light)).redraw);
    assert_eq!(app.settings.theme_mode, ThemeMode::Light);
    assert!(app.dispatch_settings_view_action(SettingsViewAction::SetViewMode(ViewMode::Tabs)).sync_window_chrome);
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-app --lib -- settings_actions_apply_values_and_return_required_effects`

Expected: FAIL because SettingsView actions are not dispatched.

- [ ] **Step 3: Implement value-based setters and reuse existing invalidation**

Add exact setters to `ui::settings::Settings` for tab width, line-number visibility, status-bar visibility, and view/theme values; each increments version once. Extend SettingsDispatchAction with value-bearing variants. Font family/size/line-height and word-wrap changes request reshape; theme requests rebuild+redraw; view mode requests window chrome sync; every valid mutation includes `PERSIST_SETTINGS`. Refresh the open SettingsView input after mutation.

```rust
match action {
    SettingsViewAction::SetThemeMode(value) => {
        self.dispatch_settings_action(SettingsDispatchAction::SetThemeMode(value))
    }
    SettingsViewAction::SetFontFamily(value) => {
        self.settings.set_font_family(value);
        AppEffect::RESHAPE.merge(AppEffect::PERSIST_SETTINGS)
    }
    SettingsViewAction::SetFontSize(value) => self.apply_zoom(value),
    SettingsViewAction::SetLineHeightRatio(value) => {
        self.settings.set_line_height_ratio(value);
        AppEffect::RESHAPE.merge(AppEffect::PERSIST_SETTINGS)
    }
    SettingsViewAction::SetWordWrap(value) => {
        self.dispatch_settings_action(SettingsDispatchAction::SetWordWrap(value))
    }
    SettingsViewAction::SetShowLineNumbers(value) => {
        self.dispatch_settings_action(SettingsDispatchAction::SetShowLineNumbers(value))
    }
    SettingsViewAction::SetTabWidth(value) => {
        self.settings.set_tab_width(value);
        AppEffect::RESHAPE.merge(AppEffect::PERSIST_SETTINGS)
    }
    SettingsViewAction::SetViewMode(value) => {
        self.dispatch_settings_action(SettingsDispatchAction::SetViewMode(value))
    }
    SettingsViewAction::SetShowStatusBar(value) => {
        self.dispatch_settings_action(SettingsDispatchAction::SetShowStatusBar(value))
    }
    SettingsViewAction::RetryPersistence => AppEffect::PERSIST_SETTINGS,
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- settings_actions`

Run: `cargo test -p textora-ui settings::tests`

Run: `cargo check -p textora-app`

Expected: all commands exit 0.

```bash
git add crates/app/src/dispatch/chrome.rs crates/app/src/app_dispatch.rs crates/ui/src/settings.rs
git commit -m "feat(app): apply settings view changes"
```

### Task 9: Track persistence failure and retry

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/settings_overlay.rs`

**Interfaces:**
- Produces: app `SettingsPersistenceState`, SettingsView failure mapping, and retry behavior.

- [ ] **Step 1: Write failing save-failure tests**

```rust
#[test]
fn failed_persistence_keeps_runtime_value_and_exposes_retry() {
    let mut app = App::new(None);
    app.dispatch_settings_view_action(SettingsViewAction::SetFontSize(20.0));
    app.record_settings_persistence_result(Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "permission denied",
    )));
    assert_eq!(app.settings.font_size, 20.0);
    assert!(matches!(app.settings_persistence, SettingsPersistenceState::SaveFailed { .. }));
    assert!(matches!(app.settings_view_input().persistence, SettingsPersistenceView::SaveFailed { .. }));
}
```

- [ ] **Step 2: Run and observe current stderr-only behavior**

Run: `cargo test -p textora-app --lib -- failed_persistence_keeps_runtime_value_and_exposes_retry`

Expected: FAIL because save failure is only printed.

- [ ] **Step 3: Implement typed state and retry**

```rust
pub(crate) enum SettingsPersistenceState {
    Saved,
    SaveFailed { message: String },
}

impl SettingsPersistenceState {
    fn to_view(&self) -> ui::settings_view::SettingsPersistenceView {
        match self {
            Self::Saved => ui::settings_view::SettingsPersistenceView::Saved,
            Self::SaveFailed { message } => {
                ui::settings_view::SettingsPersistenceView::SaveFailed { message: message.clone() }
            }
        }
    }
}

impl App {
    pub(crate) fn record_settings_persistence_result(&mut self, result: std::io::Result<()>) {
        self.settings_persistence = match result {
            Ok(()) => SettingsPersistenceState::Saved,
            Err(error) => SettingsPersistenceState::SaveFailed {
                message: error.to_string(),
            },
        };
        self.refresh_settings_overlay();
    }
}
```

Implement `SettingsPersistenceState::to_view()` in `settings_overlay.rs` and change `settings_view_input()` from the temporary Saved value introduced in Task 6 to `self.settings_persistence.to_view()`. Initialize Saved in App. On persist success set Saved; on error set SaveFailed and request redraw without rolling back Settings. `RetryPersistence` returns `AppEffect::PERSIST_SETTINGS`. Refresh the active SettingsView after every persistence result.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- settings_persistence`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs crates/app/src/settings_overlay.rs
git commit -m "feat(app): surface settings save failures"
```

### Task 10: Verify persistence round-trip

**Files:**
- Modify: `crates/app/src/settings_io.rs`
- Modify: `crates/app/src/settings_boundary_tests.rs`

**Interfaces:**
- Verifies: every exposed setting round-trips and modal input isolation remains intact.

- [ ] **Step 1: Add failing round-trip matrix test**

```rust
#[test]
fn settings_view_fields_roundtrip_through_toml() {
    let mut settings = ui::settings::Settings::new();
    settings.set_font_family("Iosevka".into());
    settings.set_font_size(18.0);
    settings.set_line_height_ratio(1.5);
    settings.set_tab_width(8);
    settings.set_word_wrap(false);
    settings.set_show_line_numbers(false);
    settings.set_show_status_bar(true);
    let persisted = roundtrip_editor_settings(&settings);
    assert_editor_fields_equal(&persisted, &settings);
}

fn roundtrip_editor_settings(settings: &ui::settings::Settings) -> PersistedSettings {
    let directory = tempfile::tempdir().expect("temporary settings directory must be created");
    let path = directory.path().join("settings.toml");
    let mut persisted = PersistedSettings::default();
    persisted.apply_editor_settings(settings);
    crate::settings_io::save_to(&path, &persisted)
        .expect("settings fixture must be serializable");
    crate::settings_io::load_from(&path)
        .expect("serialized settings fixture must load")
}
```

- [ ] **Step 2: Run and observe any missing setter/persistence mapping**

Run: `cargo test -p textora-app --lib -- settings_view_fields_roundtrip_through_toml`

Expected: FAIL if any exposed field is omitted from persistence or load mapping.

- [ ] **Step 3: Correct the exact persistence mapping**

Ensure PersistedSettings apply/load covers theme mode, view mode, font family, font size, line-height ratio, Tab width, word wrap, line numbers, and status bar. Do not add transient category/focus/validation state to TOML.

```rust
fn assert_editor_fields_equal(persisted: &PersistedSettings, settings: &ui::settings::Settings) {
    assert_eq!(persisted.theme_mode, settings.theme_mode);
    assert_eq!(persisted.view_mode, settings.view_mode);
    assert_eq!(persisted.font_family, settings.font_family);
    assert_eq!(persisted.font_size, settings.font_size);
    assert_eq!(persisted.line_height_ratio, settings.line_height_ratio);
    assert_eq!(persisted.tab_width, settings.tab_width);
    assert_eq!(persisted.word_wrap, settings.word_wrap);
    assert_eq!(persisted.show_line_numbers, settings.show_line_numbers);
    assert_eq!(persisted.show_status_bar, settings.show_status_bar);
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- settings_`

Expected: PASS.

```bash
git add crates/app/src/settings_io.rs crates/app/src/settings_boundary_tests.rs
git commit -m "test(app): verify settings view persistence"
```

### Task 11: Phase and program verification

**Files:**
- Modify only a file implicated by a reproduced verification failure; keep any repair commit within three files.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p textora-ui`.
- [ ] Run `cargo test -p textora-app --lib`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Run `./scripts/verify.sh`.
- [ ] Manual: open Preferences twice and confirm one modal; change theme/font/wrap and observe background update; verify editor input remains blocked; close and verify focus restoration.
- [ ] Expected: every command exits 0; the manual path works without a Syncthing category or dependency.
