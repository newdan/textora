# Settings Popup Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix settings button (gear icon) not working, and add toggleable settings (line numbers, word wrap, theme mode) to its popup menu.

**Architecture:** All changes stay within the existing popup menu path. The bug is that `SidebarState::paint()` never renders `self.open_menu`. New settings use the same PopupMenuAction → SidebarAction → AppAction dispatch chain already used by SetViewMode / OpenSettingsFile. Theme mode is a new enum persisted to settings.yaml.

**Tech Stack:** Rust, winit, cosmic-text, serde, existing widget framework

---

### Task 1: Add ThemeMode and line_height_ratio to Settings

**Files:**
- Modify: `crates/ui/src/settings.rs`

- [ ] **Step 1: Add ThemeMode enum and new Settings fields**

Add before `pub struct Settings`:

```rust
/// Theme mode: follow system, force dark, or force light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}
```

Add two fields inside `Settings` struct, after `view_mode`:

```rust
/// Theme mode override (System follows winit theme).
pub theme_mode: ThemeMode,
/// Line height multiplier relative to font_size.
pub line_height_ratio: f32,
```

- [ ] **Step 2: Update Settings::new() defaults**

In `Settings::new()`, add after `view_mode: ViewMode::default(),`:

```rust
theme_mode: ThemeMode::default(),
line_height_ratio: 1.618,
```

Change the hardcoded `line_height: 24.27` line to:

```rust
line_height: 15.0 * 1.618, // font_size * line_height_ratio
```

- [ ] **Step 3: Update set_font_size to use line_height_ratio**

```rust
pub fn set_font_size(&mut self, size: f32) {
    self.font_size = size;
    self.line_height = size * self.line_height_ratio;
    self.version += 1;
}
```

- [ ] **Step 4: Add set_line_height_ratio method**

```rust
/// Update line height ratio and recalculate line_height.
pub fn set_line_height_ratio(&mut self, ratio: f32) {
    self.line_height_ratio = ratio;
    self.line_height = self.font_size * ratio;
    self.version += 1;
}
```

- [ ] **Step 5: Run existing tests**

```bash
cargo test -p ui --lib settings
```

Expected: all pass (new fields have defaults so existing tests should pass).

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/settings.rs
git commit -m "feat: add ThemeMode enum and line_height_ratio to Settings"
```

---

### Task 2: Add Theme::resolve() and wire theme override

**Files:**
- Modify: `crates/ui/src/theme.rs`
- Modify: `crates/app/src/app_lifecycle.rs:225-228`
- Modify: `crates/app/src/app_init.rs:56`
- Modify: `crates/app/src/app.rs` (init and theme-rebuild sections)

- [ ] **Step 1: Add Theme::resolve() to theme.rs**

Replace `from_winit` with a new method that accepts `ThemeMode`:

```rust
/// Resolve theme from ThemeMode + system theme.
pub fn resolve(mode: crate::settings::ThemeMode, system_theme: winit::window::Theme) -> Self {
    let mut t = match mode {
        crate::settings::ThemeMode::System => match system_theme {
            winit::window::Theme::Dark => Self::dark(),
            winit::window::Theme::Light => Self::light(),
        },
        crate::settings::ThemeMode::Dark => Self::dark(),
        crate::settings::ThemeMode::Light => Self::light(),
    };
    t.gamma_correct();
    t
}
```

Keep `from_winit` as-is for backward compat (some tests use it).

- [ ] **Step 2: Add a rebuild-theme helper on App**

In `crates/app/src/app.rs`, add a method next to `handle_sidebar_key_action`:

```rust
fn rebuild_theme(&mut self) {
    let system_theme = self.window.as_ref()
        .and_then(|w| w.theme())
        .unwrap_or(winit::window::Theme::Dark);
    let mode = ui::settings::Settings::with(|s| s.theme_mode);
    self.current_theme = ui::Theme::resolve(mode, system_theme);
    self.needs_redraw = true;
}
```

- [ ] **Step 3: Update ThemeChanged to respect override**

In `crates/app/src/app_lifecycle.rs:225-228`, change:

```rust
WindowEvent::ThemeChanged(theme) => {
    self.current_theme = Theme::from_winit(theme);
    self.needs_redraw = true;
}
```

To:

```rust
WindowEvent::ThemeChanged(_system_theme) => {
    let mode = ui::settings::Settings::with(|s| s.theme_mode);
    if mode == ui::settings::ThemeMode::System {
        self.rebuild_theme();
    }
}
```

- [ ] **Step 4: Update app init theme**

In `crates/app/src/app_init.rs:56`, change:

```rust
current_theme: Theme::dark(),
```

To load from settings:

```rust
current_theme: {
    let mode = ui::settings::Settings::with(|s| s.theme_mode);
    ui::Theme::resolve(mode, winit::window::Theme::Dark)
},
```

(At init we don't have a window yet, so default to Dark for system theme fallback — first ThemeChanged event will correct it.)

- [ ] **Step 5: Build check**

```bash
cargo build 2>&1 | head -40
```

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/theme.rs crates/app/src/app_lifecycle.rs crates/app/src/app_init.rs crates/app/src/app.rs
git commit -m "feat: add Theme::resolve() with ThemeMode override support"
```

---

### Task 3: Add new PopupMenuAction and SidebarAction variants

**Files:**
- Modify: `crates/ui/src/widgets/popup_menu/types.rs` (PopupMenuAction enum)
- Modify: `crates/ui/src/widgets/sidebar/types.rs` (SidebarAction enum, dispatch_menu_click)

- [ ] **Step 1: Add new PopupMenuAction variants**

In `crates/ui/src/widgets/popup_menu/types.rs`, inside `pub enum PopupMenuAction`, add after `OpenSettingsFile`:

```rust
/// Toggle line number display.
ToggleLineNumbers,
/// Toggle word wrap.
ToggleWordWrap,
/// Set theme mode.
SetThemeMode(crate::settings::ThemeMode),
```

- [ ] **Step 2: Add new SidebarAction variants**

In `crates/ui/src/widgets/sidebar/types.rs`, inside `pub enum SidebarAction`, add after `OpenSettingsFile`:

```rust
/// Toggle line number display.
ToggleLineNumbers,
/// Toggle word wrap.
ToggleWordWrap,
/// Set theme mode.
SetThemeMode(crate::settings::ThemeMode),
```

- [ ] **Step 3: Update dispatch_menu_click**

In `crates/ui/src/widgets/sidebar/types.rs`, `dispatch_menu_click` method (around line 363-373), add cases:

```rust
PMA::ToggleLineNumbers => Some(SidebarAction::ToggleLineNumbers),
PMA::ToggleWordWrap => Some(SidebarAction::ToggleWordWrap),
PMA::SetThemeMode(mode) => Some(SidebarAction::SetThemeMode(*mode)),
```

- [ ] **Step 4: Build check**

```bash
cargo build 2>&1 | head -20
```

Expected: might have match exhaustiveness errors in events.rs/app.rs (fix in next task).

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/popup_menu/types.rs crates/ui/src/widgets/sidebar/types.rs
git commit -m "feat: add ToggleLineNumbers, ToggleWordWrap, SetThemeMode action variants"
```

---

### Task 4: Fix menu rendering + update settings menu items

Both `build_settings_menu()` (used by app.rs `OpenSidebarSettingsMenu` handler) and `SidebarState::open_settings_menu()` (used by tests) must be updated with the new items.

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/types.rs` (paint method, build_settings_menu, open_settings_menu)

- [ ] **Step 1: Add menu rendering to SidebarState::paint()**

At the end of `pub fn paint()` (after step 5 "Settings button", before the closing `}`), add:

```rust
// 6) Settings popup menu
if let Some(ref menu) = self.open_menu {
    // Shadow
    let shadow_rect = Rect::new(
        menu.menu_rect.x + 4.0 * ctx.dpi,
        menu.menu_rect.y + 4.0 * ctx.dpi,
        menu.menu_rect.w,
        menu.menu_rect.h,
    );
    ctx.list.fill_rect(shadow_rect, ctx.theme.menu_shadow, ctx.dpi);
    // Background
    ctx.list.fill_rect(menu.menu_rect, ctx.theme.menu_bg, ctx.dpi);
    // Border
    ctx.list.stroke_rect(menu.menu_rect, ctx.theme.menu_border, 1.0 * ctx.dpi, ctx.dpi);
    // Items
    let font_size = 13.0 * ctx.dpi;
    for (i, item) in menu.items.iter().enumerate() {
        if item.is_separator {
            continue;
        }
        let r = &menu.item_rects[i];
        if item.is_active {
            ctx.list.fill_rect(*r, ctx.theme.menu_selected, ctx.dpi);
        }
        ctx.list.text(
            r.x + 12.0 * ctx.dpi,
            r.y + r.h * 0.5 + font_size * 0.35,
            font_size,
            ctx.theme.menu_text,
            &item.label,
        );
    }
}
```

- [ ] **Step 2: Update open_settings_menu item list**

Replace the item list in `open_settings_menu` (lines 317-341) with:

```rust
let show_line_numbers = Settings::with(|s| s.show_line_numbers);
let word_wrap = Settings::with(|s| s.word_wrap);
let current_theme = Settings::with(|s| s.theme_mode);

let items = vec![
    PopupMenuItem {
        label: "显示行号".into(),
        is_active: show_line_numbers,
        is_separator: false,
        action: PMA::ToggleLineNumbers,
    },
    PopupMenuItem {
        label: "自动换行".into(),
        is_active: word_wrap,
        is_separator: false,
        action: PMA::ToggleWordWrap,
    },
    PopupMenuItem {
        label: "".into(),
        is_active: false,
        is_separator: true,
        action: PMA::ToggleLineNumbers, // unused for separator
    },
    PopupMenuItem {
        label: "跟随系统".into(),
        is_active: current_theme == ThemeMode::System,
        is_separator: false,
        action: PMA::SetThemeMode(ThemeMode::System),
    },
    PopupMenuItem {
        label: "深色模式".into(),
        is_active: current_theme == ThemeMode::Dark,
        is_separator: false,
        action: PMA::SetThemeMode(ThemeMode::Dark),
    },
    PopupMenuItem {
        label: "浅色模式".into(),
        is_active: current_theme == ThemeMode::Light,
        is_separator: false,
        action: PMA::SetThemeMode(ThemeMode::Light),
    },
    PopupMenuItem {
        label: "".into(),
        is_active: false,
        is_separator: true,
        action: PMA::ToggleLineNumbers, // unused for separator
    },
    PopupMenuItem {
        label: "打开 settings.yaml".into(),
        is_active: false,
        is_separator: false,
        action: PMA::OpenSettingsFile,
    },
    PopupMenuItem {
        label: "".into(),
        is_active: false,
        is_separator: true,
        action: PMA::ToggleLineNumbers, // unused for separator
    },
    PopupMenuItem {
        label: "Sidebar 模式".into(),
        is_active: matches!(current_view_mode, ViewMode::Sidebar),
        is_separator: false,
        action: PMA::SetViewMode(ViewMode::Sidebar),
    },
    PopupMenuItem {
        label: "Tabs 模式".into(),
        is_active: matches!(current_view_mode, ViewMode::Tabs),
        is_separator: false,
        action: PMA::SetViewMode(ViewMode::Tabs),
    },
];
```

Note: `use crate::settings::ThemeMode;` needs to be added at top of file.

Add `let current_view_mode = Settings::with(|s| s.view_mode);` before the items vec.

- [ ] **Step 2b: Update build_settings_menu() with same items**

The standalone `build_settings_menu()` function (line 825) is called by `app.rs:1170`. Replace its item list with the same items as above. The anchor position calculation stays the same.

```rust
pub fn build_settings_menu(_layout: Option<&SidebarLayout>, screen_w: f32, screen_h: f32) -> Option<PopupMenu> {
    use crate::widgets::popup_menu::PopupMenuItem;
    use crate::widgets::popup_menu::PopupMenuAction as PMA;
    use crate::settings::ThemeMode;
    let dpi = crate::settings::Settings::with(|s| s.dpi_scale);
    let item_h = constants::ROW_HEIGHT * dpi;
    let menu_w = 220.0 * dpi; // slightly wider for theme labels
    let (anchor_x, anchor_y) = (screen_w * 0.025, screen_h * 0.50); // centered-ish

    let show_line_numbers = crate::settings::Settings::with(|s| s.show_line_numbers);
    let word_wrap = crate::settings::Settings::with(|s| s.word_wrap);
    let current_theme = crate::settings::Settings::with(|s| s.theme_mode);
    let current_view_mode = crate::settings::Settings::with(|s| s.view_mode);

    let items = vec![
        // ... same items as open_settings_menu above ...
    ];
    // ... rest of function unchanged (anchor calc, rect building, Some(PopupMenu{...}))
}
```

- [ ] **Step 3: Update dispatch_menu_click for theme mode**

In `dispatch_menu_click`, make sure ThemeMode import is available and the match handles `SetThemeMode(mode)`. (Already done in Task 3.)

- [ ] **Step 4: Build check**

```bash
cargo build 2>&1 | head -20
```

Expected: compiles (or only app-layer match errors).

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/sidebar/types.rs
git commit -m "fix: render settings popup menu + populate with toggle/radio items"
```

---

### Task 5: Handle new actions in app layer

**Files:**
- Modify: `crates/app/src/actions.rs` (add AppAction variants if needed)
- Modify: `crates/app/src/events.rs` (translate_sidebar_action)
- Modify: `crates/app/src/app.rs` (handle new actions)

- [ ] **Step 1: Add ToggleLineNumbers, ToggleWordWrap, SetThemeMode to AppAction**

In `crates/app/src/actions.rs`, add after `SetViewMode(ViewMode)`:

```rust
/// Toggle line number display.
ToggleLineNumbers,
/// Toggle word wrap.
ToggleWordWrap,
/// Set theme mode (System / Dark / Light).
SetThemeMode(ui::settings::ThemeMode),
```

- [ ] **Step 2: Wire SidebarAction → AppAction in events.rs**

In `translate_sidebar_action` function (around line 413-463), add cases before the closing `}`:

```rust
S::ToggleLineNumbers => actions.push(AppAction::ToggleLineNumbers),
S::ToggleWordWrap => actions.push(AppAction::ToggleWordWrap),
S::SetThemeMode(mode) => actions.push(AppAction::SetThemeMode(*mode)),
```

- [ ] **Step 3: Handle new actions in app.rs**

Find the match on `AppAction` variants (search for `AppAction::SetViewMode`). Add after the `AppAction::SetViewMode` handler:

```rust
AppAction::ToggleLineNumbers => {
    ui::settings::Settings::with_mut(|s| {
        s.show_line_numbers = !s.show_line_numbers;
    });
    self.needs_redraw = true;
}
AppAction::ToggleWordWrap => {
    ui::settings::Settings::with_mut(|s| {
        s.word_wrap = !s.word_wrap;
    });
    self.needs_redraw = true;
}
AppAction::SetThemeMode(mode) => {
    ui::settings::Settings::with_mut(|s| {
        s.theme_mode = mode;
    });
    self.rebuild_theme();
    self.ui_shell.sidebar_set_open_menu(None); // close menu after selecting
}
```

Also add `ToggleLineNumbers | ToggleWordWrap | SetThemeMode(_)` to the no-op match arm at `handle_sidebar_key_action` (around line 1190) alongside the existing `OpenSettingsMenu` etc.:

```rust
| ui::widgets::sidebar::SidebarAction::ToggleLineNumbers
| ui::widgets::sidebar::SidebarAction::ToggleWordWrap
| ui::widgets::sidebar::SidebarAction::SetThemeMode(_)
```

- [ ] **Step 4: Build check**

```bash
cargo build 2>&1
```

Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/actions.rs crates/app/src/events.rs crates/app/src/app.rs
git commit -m "feat: handle ToggleLineNumbers, ToggleWordWrap, SetThemeMode in app layer"
```

---

### Task 6: Persist theme_mode to settings.yaml

**Files:**
- Modify: `crates/app/src/settings_io.rs`

- [ ] **Step 1: Add theme_mode to PersistedSettings**

```rust
use ui::settings::ThemeMode;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PersistedSettings {
    pub view_mode: ViewMode,
    pub theme_mode: ThemeMode,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
}
```

- [ ] **Step 2: Wire load path**

In `crates/app/src/app_init.rs:50`, add after `s.view_mode = persisted.view_mode;`:

```rust
s.theme_mode = persisted.theme_mode;
```

- [ ] **Step 3: Wire save path**

In `crates/app/src/app.rs`, `save_window_geometry` method (line 283-293), add before `crate::settings_io::save(&settings);`:

```rust
settings.theme_mode = ui::settings::Settings::with(|s| s.theme_mode);
```

- [ ] **Step 4: Add roundtrip test**

In `settings_io.rs` tests:

```rust
#[test]
fn theme_mode_roundtrip() {
    let s = PersistedSettings {
        theme_mode: ThemeMode::Dark,
        ..Default::default()
    };
    let yaml = serde_yml::to_string(&s).unwrap();
    let parsed: PersistedSettings = serde_yml::from_str(&yaml).unwrap();
    assert_eq!(parsed.theme_mode, ThemeMode::Dark);
}

#[test]
fn theme_mode_default_is_system() {
    let s = PersistedSettings::default();
    assert_eq!(s.theme_mode, ThemeMode::System);
}
```

- [ ] **Step 5: Run tests + build**

```bash
cargo test -p app --lib settings_io
cargo build
```

Expected: all pass, clean compile.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/settings_io.rs crates/app/src/app_init.rs crates/app/src/app.rs
git commit -m "feat: persist theme_mode to settings.yaml"
```

---

### Task 7: Manual smoke test

- [ ] **Step 1: Build and run**

```bash
cargo run
```

- [ ] **Step 2: Verify checklist**

1. Click gear button in sidebar — menu appears
2. Click "显示行号" — line numbers toggle on/off instantly
3. Click "自动换行" — word wrap toggles instantly
4. Click "深色模式" — switches to dark theme, menu closes
5. Reopen menu — "深色模式" has checkmark
6. Click "跟随系统" — reverts to system theme
7. Click "打开 settings.yaml" — file opens
8. Switch Sidebar ↔ Tabs — works as before
9. Close menu by clicking outside — menu dismisses
10. Press Escape — menu dismisses
11. Restart app — theme_mode persists
