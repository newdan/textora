# Settings UI Phase 1: Leaf Controls and Unified Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide reusable macOS-style Label, Button, TextBox, Switch, and Checkbox widgets with stable identity, secure text payloads, and a unified action protocol.

**Architecture:** Interactive leaf widgets emit `WidgetAction::Control(ControlAction)` carrying their `WidgetId`; passive Label implements Widget with no actions. Settings-specific meaning remains in later parent views. Existing SearchBar is migrated from TextBox business callbacks to explicit child actions.

**Tech Stack:** Rust 2024, existing `ui::core::Widget`, DrawList, shaping, winit events, and `zeroize::Zeroizing<String>`.

## Global Constraints

- Keep `ui` independent from app and Syncthing.
- Do not introduce Label status variants or Button business variants.
- Button styles must support foreground, background, border, hover, pressed, selected, and disabled states.
- Masked TextBox must not emit per-keystroke plaintext and must not expose plaintext through Debug.
- Preserve existing SearchBar IME, selection, and clipboard behavior.
- Each task modifies at most three files and ends with a compiling commit.

---

### Task 1: Derive settings visual tokens from the existing palette

**Files:**
- Create: `crates/ui/src/theme/settings.rs`
- Modify: `crates/ui/src/theme/mod.rs`

**Interfaces:**
- Consumes: `crate::theme::ColorPalette`.
- Produces: `SettingsTheme` and `Theme::settings_theme() -> SettingsTheme`.

- [ ] **Step 1: Write the failing token mapping tests**

```rust
#[test]
fn settings_tokens_are_derived_from_palette() {
    let theme = crate::theme::test_theme();
    let tokens = theme.settings_theme();
    assert_eq!(tokens.modal_surface, theme.palette.bg_elevated);
    assert_eq!(tokens.sidebar_surface, theme.palette.bg_surface);
    assert_eq!(tokens.section_surface, theme.palette.bg_elevated);
    assert_eq!(tokens.focus_ring, theme.palette.accent);
    assert_eq!(tokens.text_primary, theme.palette.text_main);
    assert_eq!(tokens.text_secondary, theme.palette.text_muted);
}
```

- [ ] **Step 2: Run the focused test and observe failure**

Run: `cargo test -p textora-ui settings_tokens_are_derived_from_palette`

Expected: FAIL because `SettingsTheme` and `settings_theme` do not exist.

- [ ] **Step 3: Implement the semantic token projection**

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsTheme {
    pub modal_surface: [f32; 4],
    pub sidebar_surface: [f32; 4],
    pub section_surface: [f32; 4],
    pub section_border: [f32; 4],
    pub separator: [f32; 4],
    pub control_surface: [f32; 4],
    pub control_border: [f32; 4],
    pub focus_ring: [f32; 4],
    pub accent: [f32; 4],
    pub text_primary: [f32; 4],
    pub text_secondary: [f32; 4],
}

impl SettingsTheme {
    pub fn from_palette(palette: &super::ColorPalette) -> Self {
        Self {
            modal_surface: palette.bg_elevated,
            sidebar_surface: palette.bg_surface,
            section_surface: palette.bg_elevated,
            section_border: palette.border_subtle,
            separator: palette.border_subtle,
            control_surface: palette.input_bg,
            control_border: palette.input_border,
            focus_ring: palette.accent,
            accent: palette.accent,
            text_primary: palette.text_main,
            text_secondary: palette.text_muted,
        }
    }
}
```

Export `SettingsTheme` from `theme/mod.rs` and add `Theme::settings_theme()` as a pure projection.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui settings_tokens_are_derived_from_palette`

Expected: PASS.

```bash
git add crates/ui/src/theme/settings.rs crates/ui/src/theme/mod.rs
git commit -m "feat(ui): define settings theme tokens"
```

### Task 2: Add secure text payloads and unified control actions

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/ui/Cargo.toml`
- Modify: `crates/ui/src/core/widget.rs`

**Interfaces:**
- Produces: `SensitiveText`, `TextPayload`, `ControlAction`, and `WidgetAction::Control`.

- [ ] **Step 1: Write failing identity and redaction tests**

```rust
#[test]
fn sensitive_text_debug_is_redacted() {
    let secret = SensitiveText::new("never-print-me".into());
    assert_eq!(format!("{secret:?}"), "SensitiveText(<redacted>)");
    assert!(!format!("{:?}", TextPayload::Sensitive(secret)).contains("never-print-me"));
}

#[test]
fn control_action_preserves_widget_identity() {
    let id = WidgetId(42);
    assert_eq!(
        ControlAction::Toggled { id, checked: true }.id(),
        id
    );
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui core::widget::tests`

Expected: FAIL because the new types are absent.

- [ ] **Step 3: Add the dependency and types**

Add `zeroize = "1"` to workspace dependencies and `zeroize.workspace = true` to `crates/ui/Cargo.toml`.

```rust
pub struct SensitiveText(zeroize::Zeroizing<String>);

impl SensitiveText {
    pub fn new(value: String) -> Self { Self(zeroize::Zeroizing::new(value)) }
    pub fn expose(&self) -> &str { self.0.as_str() }
}

impl std::fmt::Debug for SensitiveText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SensitiveText(<redacted>)")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextPayload { Plain(String), Sensitive(SensitiveText) }

#[derive(Clone, Debug, PartialEq)]
pub enum ControlAction {
    Activated { id: WidgetId },
    Toggled { id: WidgetId, checked: bool },
    TextEdited { id: WidgetId, value: TextPayload },
    TextCommitted { id: WidgetId, value: TextPayload },
    FocusRequested { id: WidgetId },
}
```

Implement `Clone` and `PartialEq` for `SensitiveText` without exposing content through Debug, add `ControlAction::id()`, and add `WidgetAction::Control(ControlAction)` while retaining existing variants until their migration tasks.

Extend Widget with focus-composition hooks used by later generic containers:

```rust
fn is_focusable(&self) -> bool { false }
fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
    if self.is_focusable() && let Some(id) = self.id() { output.push(id); }
}
fn set_keyboard_focus(&mut self, _focused_id: Option<WidgetId>) {}
```

Interactive leaf controls override these methods; passive Label keeps the defaults.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui core::widget::tests`

Expected: PASS.

```bash
git add Cargo.toml crates/ui/Cargo.toml crates/ui/src/core/widget.rs
git commit -m "feat(ui): add secure control actions"
```

### Task 3: Add the passive Label widget

**Files:**
- Create: `crates/ui/src/widgets/label.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `Label`, `LabelStyle`, `LabelForeground`, and optional named leading/trailing icons.

- [ ] **Step 1: Write failing Label tests in `label.rs`**

```rust
#[test]
fn label_paints_icon_before_text_and_never_emits_actions() {
    let mut label = Label::new("Connected", LabelStyle::default());
    label.set_leading_icon(Some("check".into()));
    set_test_rect(&mut label, Rect::new(0.0, 0.0, 180.0, 28.0));
    let draw_list = paint_for_test(&label);
    assert!(draw_list.cmds.len() >= 2);
    assert_eq!(label.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx()), None);
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui label_paints_icon_before_text_and_never_emits_actions`

Expected: FAIL because the Label module does not exist.

- [ ] **Step 3: Implement Label**

```rust
const DEFAULT_LABEL_FONT_SIZE_LOGICAL: f32 = 13.0;
const DEFAULT_LABEL_ICON_GAP_LOGICAL: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LabelForeground {
    #[default]
    ThemeMain,
    ThemeMuted,
    Explicit([f32; 4]),
}

pub struct LabelStyle {
    pub font_size_logical: f32,
    pub font_weight: shaping::Weight,
    pub foreground: LabelForeground,
    pub gap_logical: f32,
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self {
            font_size_logical: DEFAULT_LABEL_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::NORMAL,
            foreground: LabelForeground::ThemeMain,
            gap_logical: DEFAULT_LABEL_ICON_GAP_LOGICAL,
        }
    }
}

pub struct Label {
    rect: Rect,
    text: String,
    leading_icon: Option<String>,
    trailing_icon: Option<String>,
    style: LabelStyle,
}
```

Implement `Widget` with layout, shaped text paint, optional passive icons, `hit`, and no event actions. Resolve `ThemeMain` and `ThemeMuted` from `PaintCtx.theme.palette`; `Explicit` is an appearance override only. Do not add success/warning/error variants—the caller expresses status with icon plus wording. Re-export through `widgets/mod.rs` and `lib.rs`.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui label`

Expected: all Label tests PASS.

```bash
git add crates/ui/src/widgets/label.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add reusable label widget"
```

### Task 4: Refactor Button onto ControlAction and release-click semantics

**Files:**
- Modify: `crates/ui/src/widgets/button.rs`
- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**
- Consumes: `ControlAction::Activated` from Task 2.
- Produces: `Button::new(id: WidgetId, style: ButtonStyle)` and background-capable state styles.

- [ ] **Step 1: Replace Button tests with failing press/release cases**

```rust
#[test]
fn button_activates_only_after_inside_press_and_release() {
    let mut button = make_button(WidgetId(7));
    assert_eq!(mouse_down(&mut button, 20.0, 10.0), Some(WidgetAction::Consumed));
    assert_eq!(
        mouse_up(&mut button, 20.0, 10.0),
        Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(7) }))
    );
}

#[test]
fn dragging_outside_cancels_button_activation() {
    let mut button = make_button(WidgetId(8));
    mouse_down(&mut button, 20.0, 10.0);
    assert_eq!(mouse_up(&mut button, 200.0, 200.0), Some(WidgetAction::Consumed));
}
```

- [ ] **Step 2: Run and observe the old MouseDown behavior fail**

Run: `cargo test -p textora-ui widgets::button::tests`

Expected: FAIL because current Button activates on MouseDown and has no identity.

- [ ] **Step 3: Implement the new Button state and style**

```rust
pub struct ButtonStyle {
    pub font_size_logical: f32,
    pub pad_x_logical: f32,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub hover_background: [f32; 4],
    pub pressed_background: [f32; 4],
    pub selected_background: [f32; 4],
    pub disabled_foreground: [f32; 4],
    pub disabled_background: [f32; 4],
    pub corner_radius_logical: f32,
}
```

Store `id`, `enabled`, `pressed`, and `selected`; capture between left MouseDown and MouseUp; return `ControlAction::Activated` only for an inside release. Remove `ButtonAction` and `WidgetAction::Button`, then update the exhaustive app event match to consume `WidgetAction::Control` without translating it yet.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui button`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/ui/src/widgets/button.rs crates/ui/src/core/widget.rs crates/app/src/events.rs
git commit -m "refactor(ui): unify button control actions"
```

### Task 5: Add Switch

**Files:**
- Create: `crates/ui/src/widgets/switch.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `Switch::new(id, checked)` and `ControlAction::Toggled`.

- [ ] **Step 1: Write failing mouse and Space-key tests**

```rust
#[test]
fn switch_toggles_with_click_and_space() {
    let mut switch = focused_switch(WidgetId(20), false);
    assert_toggle(click(&mut switch), WidgetId(20), true);
    assert_toggle(key_space(&mut switch), WidgetId(20), false);
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui switch_toggles_with_click_and_space`

Expected: FAIL because Switch is absent.

- [ ] **Step 3: Implement the macOS-style track and thumb**

```rust
pub struct Switch {
    id: WidgetId,
    rect: Rect,
    checked: bool,
    enabled: bool,
    focused: bool,
    hovered: bool,
}
```

Paint a rounded track and circular thumb from SettingsTheme tokens; use accent only when checked; emit `FocusRequested` on first mouse focus and `Toggled` on activation.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui switch`

Expected: PASS.

```bash
git add crates/ui/src/widgets/switch.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add switch control"
```

### Task 6: Add Checkbox

**Files:**
- Create: `crates/ui/src/widgets/checkbox.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `Checkbox::new(id, checked)` and `ControlAction::Toggled`.

- [ ] **Step 1: Write failing keyboard and paint tests**

```rust
#[test]
fn checkbox_uses_box_visual_and_toggle_action() {
    let mut checkbox = focused_checkbox(WidgetId(21), false);
    assert_toggle(key_space(&mut checkbox), WidgetId(21), true);
    assert!(paint_for_test(&checkbox).cmds.iter().any(is_checkbox_outline));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui checkbox_uses_box_visual_and_toggle_action`

Expected: FAIL because Checkbox is absent.

- [ ] **Step 3: Implement Checkbox with shared toggle semantics**

```rust
pub struct Checkbox {
    id: WidgetId,
    rect: Rect,
    checked: bool,
    enabled: bool,
    focused: bool,
    hovered: bool,
}
```

Keep the public widget separate from Switch; factor only private event-state helpers if duplication appears inside this file. Paint a square control and check icon using SettingsTheme tokens.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui checkbox`

Expected: PASS.

```bash
git add crates/ui/src/widgets/checkbox.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add checkbox control"
```

### Task 7: Add Plain and Masked TextBox modes

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`

**Interfaces:**
- Consumes: `SensitiveText` and `TextPayload` from Task 2.
- Produces: `EchoMode`, `TextBox::set_echo_mode`, and `TextBox::take_committed_payload`.

- [ ] **Step 1: Write failing masked rendering and submission tests**

```rust
#[test]
fn masked_textbox_paints_bullets_and_commits_sensitive_payload() {
    let mut box_ = TextBox::new();
    box_.set_echo_mode(EchoMode::Masked);
    box_.set_text("secret-value");
    let draw_list = paint_laid_out(&mut box_);
    assert!(!format!("{:?}", draw_list.cmds).contains("secret-value"));
    assert!(box_.on_key(KeyCode::Enter, Modifiers::NONE));
    assert!(matches!(
        box_.take_committed_payload(),
        Some(TextPayload::Sensitive(_))
    ));
}
```

- [ ] **Step 2: Run and observe plaintext failure**

Run: `cargo test -p textora-ui masked_textbox_paints_bullets_and_commits_sensitive_payload`

Expected: FAIL because EchoMode is absent and current paint uses plaintext.

- [ ] **Step 3: Implement mode-aware display and payload construction**

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EchoMode { #[default] Plain, Masked }

fn display_text(&self) -> String {
    match self.echo_mode {
        EchoMode::Plain => self.text.clone(),
        EchoMode::Masked => "•".repeat(self.text.chars().count()),
    }
}
```

Measure and paint display text, keep cursor byte logic against the original string, and construct SensitiveText only on explicit commit. Do not emit masked edits per key.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui text_box`

Expected: all existing and masked TextBox tests PASS.

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat(ui): support masked textbox input"
```

### Task 8: Make TextBox a Widget and remove business callbacks

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`
- Modify: `crates/ui/src/widgets/search_bar.rs`

**Interfaces:**
- Produces: `TextBox::with_id(id)`, Widget implementation, and identity-bearing text/focus actions.
- Preserves: SearchBarAction behavior and clipboard adapters.

- [ ] **Step 1: Write failing Widget action tests**

```rust
#[test]
fn textbox_widget_emits_plain_edit_and_commit_actions() {
    let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(30)));
    assert_eq!(
        key(&mut box_, KeyCode::Char('x')),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            id: WidgetId(30),
            value: TextPayload::Plain("x".into()),
        }))
    );
    assert!(matches!(key(&mut box_, KeyCode::Enter), Some(WidgetAction::Control(ControlAction::TextCommitted { .. }))));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui textbox_widget_emits_plain_edit_and_commit_actions`

Expected: FAIL because TextBox does not implement Widget.

- [ ] **Step 3: Implement Widget routing and migrate SearchBar**

Remove `on_changed`, `on_enter`, `on_escape`, and `on_focus` callback fields. Add an internal event method returning `Option<ControlAction>` and wrap it in `Widget::on_event`. SearchBar assigns private IDs to find/replace boxes, maps their ControlAction values to existing SearchBarAction variants, and retains only injected clipboard read/write adapters.

```rust
match text_box_action {
    ControlAction::TextEdited { id: FIND_BOX_ID, value: TextPayload::Plain(text) } => {
        Some(WidgetAction::SearchBar(SearchBarAction::QueryChanged(text)))
    }
    ControlAction::TextCommitted { id: FIND_BOX_ID, .. } => {
        Some(WidgetAction::SearchBar(SearchBarAction::Next))
    }
    _ => Some(WidgetAction::Consumed),
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui text_box`

Run: `cargo test -p textora-ui search_bar`

Run: `cargo check -p textora-app`

Expected: all commands exit 0.

```bash
git add crates/ui/src/widgets/text_box.rs crates/ui/src/widgets/search_bar.rs
git commit -m "refactor(ui): route textbox actions explicitly"
```

### Task 9: Phase verification

**Files:**
- Modify only a file implicated by a reproduced verification failure; keep any repair commit within three files.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p textora-ui`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Expected: every command exits 0 with no new warning, and existing SearchBar IME/clipboard tests remain green.
