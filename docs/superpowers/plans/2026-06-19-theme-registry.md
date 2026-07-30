# Theme Registry + ThemeMode Simplification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce `ThemeDefinition` (pure data), `ThemeRegistry` (key→definition map), `ActiveThemePair`, simplify `ThemeMode` to System/Dark/Light, and remove old per-theme constructors.

**Architecture:** ThemeDefinition is a pure-data copy of Theme's fields (sans HashMap for scopes — uses BTreeMap). ThemeRegistry holds built-in defaults in dedicated fields + user-registered themes in a HashMap. App owns the registry alongside Settings. Theme::from_definition() constructs Theme from data and applies gamma correction.

**Tech Stack:** Rust, no new dependencies.

**Prerequisite:** Plans 1+2 (Theme struct modularization + consumer migration) must be complete.

---

### Task 1: Define ThemeDefinition and Theme::from_definition

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Add ThemeDefinition struct after the existing type definitions**

```rust
/// Pure-data theme definition in sRGB space.
/// from_definition() converts to linear-space Theme via gamma correction.
#[derive(Debug, Clone)]
pub struct ThemeDefinition {
    pub display_name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub scopes: BTreeMap<String, [f32; 4]>,
}
```

Add `use std::collections::BTreeMap;` to imports.

- [ ] **Step 2: Implement Theme::from_definition()**

```rust
impl Theme {
    /// Build a gamma-corrected Theme from a pure-data definition.
    /// Definition values are sRGB; gamma correction converts to linear.
    pub fn from_definition(def: &ThemeDefinition) -> Self {
        let mut theme = Self {
            name: def.display_name.clone(),
            is_dark: def.is_dark,
            palette: def.palette.clone(),
            editor: def.editor.clone(),
            markdown: def.markdown.clone(),
            scopes: def.scopes.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        };
        theme.gamma_correct();
        theme
    }
}
```

- [ ] **Step 3: Add built-in definition constructors**

```rust
impl ThemeDefinition {
    /// Built-in default dark theme (Claude Dark values, sRGB).
    pub fn default_dark() -> Self {
        Self {
            display_name: "默认黑".into(),
            is_dark: true,
            palette: ColorPalette {
                bg_base: [0.04, 0.038, 0.036, 1.0],
                bg_surface: [0.055, 0.053, 0.051, 1.0],
                bg_elevated: [0.07, 0.068, 0.066, 1.0],
                bg_hover: [0.08, 0.078, 0.076, 1.0],
                bg_active: [0.10, 0.098, 0.096, 1.0],
                text_main: [0.9608, 0.9529, 0.9412, 1.0],
                text_muted: [0.65, 0.64, 0.62, 1.0],
                text_inverse: [0.9, 0.9, 0.9, 1.0],
                border_subtle: [0.05, 0.048, 0.046, 1.0],
                border_strong: [0.05, 0.048, 0.046, 1.0],
                shadow: [0.0, 0.0, 0.0, 0.5],
                accent: [0.8706, 0.4510, 0.3373, 1.0],
                highlight: [1.0, 0.65, 0.2, 0.75],
                danger: [0.8941, 0.3373, 0.2863, 1.0],
                warning: [0.8745, 0.7569, 0.5176, 1.0],
                input_bg: [0.055, 0.053, 0.051, 1.0],
                input_border: [0.08, 0.078, 0.076, 1.0],
                input_fg: [0.9608, 0.9529, 0.9412, 1.0],
            },
            // ... editor, markdown, scopes — all Claude Dark values
        }
    }
}
```

Repeat `default_light()` with Claude Light values (all fields explicit).

Note: included abridged for plan readability. During implementation, copy full values from the existing `claude_dark()` / `claude_light()` constructors before they are deleted.

- [ ] **Step 4: Run `cargo check -p edit-plus-ui`**

---

### Task 2: Create ThemeRegistry module

**Files:**
- Create: `crates/ui/src/theme_registry.rs`
- Modify: `crates/ui/src/lib.rs` (add `pub mod theme_registry;`)

- [ ] **Step 1: Create `crates/ui/src/theme_registry.rs`**

```rust
use std::collections::HashMap;
use crate::theme::ThemeDefinition;

/// Maps theme identifier → definition.
/// Built-in defaults stored separately — always available as fallback.
#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: HashMap<String, ThemeDefinition>,
    default_dark: ThemeDefinition,
    default_light: ThemeDefinition,
}

impl ThemeRegistry {
    /// Create registry with built-in defaults.
    pub fn new() -> Self {
        Self {
            themes: HashMap::new(),
            default_dark: ThemeDefinition::default_dark(),
            default_light: ThemeDefinition::default_light(),
        }
    }

    /// Register a user theme. Returns Err if id clashes with built-in reserved keys.
    pub fn register(&mut self, id: String, def: ThemeDefinition) -> Result<(), RegisterError> {
        if id == "default-dark" || id == "default-light" {
            return Err(RegisterError::ReservedId(id));
        }
        self.themes.insert(id, def);
        Ok(())
    }

    /// Remove a user-registered theme. Built-ins cannot be removed.
    pub fn unregister(&mut self, id: &str) -> bool {
        self.themes.remove(id).is_some()
    }

    /// Look up with fallback. NEVER returns None.
    pub fn get_or_default(&self, id: &str, prefer_dark: bool) -> &ThemeDefinition {
        self.themes.get(id).unwrap_or_else(|| {
            if prefer_dark { &self.default_dark } else { &self.default_light }
        })
    }

    /// Raw lookup without fallback.
    pub fn get(&self, id: &str) -> Option<&ThemeDefinition> {
        self.themes.get(id)
    }

    /// All registered identifiers (excluding built-in reserved keys).
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.themes.keys().map(|s| s.as_str())
    }

    /// List built-in identifiers.
    pub fn builtin_ids(&self) -> &[&str] {
        &["default-dark", "default-light"]
    }

    /// Number of user-registered themes.
    pub fn len(&self) -> usize {
        self.themes.len()
    }
}

#[derive(Debug)]
pub enum RegisterError {
    ReservedId(String),
}
```

- [ ] **Step 2: Add `pub mod theme_registry;` to `crates/ui/src/lib.rs`**

- [ ] **Step 3: Run `cargo check -p edit-plus-ui`**

---

### Task 3: Simplify ThemeMode enum

**Files:**
- Modify: `crates/ui/src/settings.rs`

- [ ] **Step 1: Replace ThemeMode enum**

```rust
/// Which color scheme to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    /// Follow the system appearance.
    #[default]
    System,
    /// Force dark theme.
    Dark,
    /// Force light theme.
    Light,
}
```

Remove `ClaudeLight`, `ClaudeDark` variants.

- [ ] **Step 2: Add ActiveThemePair to settings.rs**

```rust
/// Which specific themes are active for dark/light.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveThemePair {
    pub dark: String,
    pub light: String,
}

impl Default for ActiveThemePair {
    fn default() -> Self {
        Self {
            dark: "default-dark".into(),
            light: "default-light".into(),
        }
    }
}
```

- [ ] **Step 3: Add `active_themes` to Settings struct**

```rust
pub struct Settings {
    pub theme_mode: ThemeMode,
    pub active_themes: ActiveThemePair,
    // ... all other fields unchanged
}
```

Update `Settings::default()` to include `active_themes: ActiveThemePair::default()`.

- [ ] **Step 4: Run `cargo check -p edit-plus-ui` — expect errors from ClaudeLight/ClaudeDark references elsewhere**

---

### Task 4: Update Theme::resolve() and from_winit()

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Rewrite `resolve()` to accept registry + pair**

```rust
impl Theme {
    pub fn resolve(
        mode: crate::settings::ThemeMode,
        system_theme: winit::window::Theme,
        pair: &crate::settings::ActiveThemePair,
        registry: &crate::theme_registry::ThemeRegistry,
    ) -> Self {
        let want_dark = match mode {
            crate::settings::ThemeMode::System => matches!(system_theme, winit::window::Theme::Dark),
            crate::settings::ThemeMode::Dark => true,
            crate::settings::ThemeMode::Light => false,
        };
        let id = if want_dark { &pair.dark } else { &pair.light };
        let def = registry.get_or_default(id, want_dark);
        Self::from_definition(def)
    }
}
```

- [ ] **Step 2: Rewrite `from_winit()` similarly**

```rust
pub fn from_winit(
    theme: winit::window::Theme,
    pair: &crate::settings::ActiveThemePair,
    registry: &crate::theme_registry::ThemeRegistry,
) -> Self {
    let want_dark = matches!(theme, winit::window::Theme::Dark);
    let id = if want_dark { &pair.dark } else { &pair.light };
    let def = registry.get_or_default(id, want_dark);
    Self::from_definition(def)
}
```

- [ ] **Step 3: Remove old `Theme::claude_dark()` and `Theme::claude_light()` constructors**

- [ ] **Step 4: Remove old `Theme::dark()` and `Theme::light()` constructors** (edit+ values are not built-in defaults)

- [ ] **Step 5: Run `cargo check -p edit-plus-ui` — expect errors from call sites of removed constructors and changed resolve/from_winit signatures**

---

### Task 5: Update test_theme() to use from_definition

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Replace `test_theme()` — build via ThemeDefinition**

```rust
pub fn test_theme() -> Theme {
    let def = ThemeDefinition {
        display_name: "test".into(),
        is_dark: true,
        palette: ColorPalette {
            bg_base: [0.0; 4], bg_surface: [0.0; 4], bg_elevated: [0.0; 4],
            bg_hover: [0.0; 4], bg_active: [0.0; 4],
            text_main: [0.0; 4], text_muted: [0.0; 4], text_inverse: [0.0; 4],
            border_subtle: [0.0; 4], border_strong: [0.0; 4], shadow: [0.0; 4],
            accent: [0.0; 4], highlight: [0.0; 4], danger: [0.0; 4], warning: [0.0; 4],
            input_bg: [0.0; 4], input_border: [0.0; 4], input_fg: [0.0; 4],
        },
        editor: EditorTheme {
            background: [0.0; 4], foreground: [0.0; 4], gutter_bg: [0.0; 4],
            line_number: [0.0; 4], selection: [0.0; 4], cursor: [0.0; 4],
            scrollbar_track: [0.0; 4], scrollbar_thumb: [0.0; 4],
        },
        markdown: MarkdownTheme {
            heading: [0.0; 4], link: [0.0; 4], inline_code: [0.0; 4],
            code_bg: [0.0; 4], code_block_bg: [0.0; 4],
            toc_background: [0.0; 4], toc_active_background: [0.0; 4],
            toc_text: [0.0; 4], toc_level_indicator: [0.0; 4],
            spacing: MarkdownSpacing {
                paragraph_spacing: 0.0, heading_spacing_top: 0.0,
                heading_spacing_bottom: 0.0, list_item_spacing: 0.0,
                list_group_spacing: 0.0, list_indent: 0.0,
                code_block_padding: 0.0, blockquote_padding: 0.0,
                table_cell_padding: 0.0, rule_spacing: 0.0,
                rule_thickness: 0.0, rule_width_ratio: 0.0,
                border_radius_base: 0.0, border_radius_small: 0.0,
                code_line_height: 0.0,
            },
        },
        scopes: BTreeMap::new(),
    };
    Theme::from_definition(&def)
}
```

- [ ] **Step 2: Fix all tests in theme.rs that use removed constructors**

Replace `Theme::dark()` → `Theme::from_definition(&ThemeDefinition::default_dark())`
Replace `Theme::light()` → `Theme::from_definition(&ThemeDefinition::default_light())`
Replace `Theme::claude_dark()` → `Theme::from_definition(&ThemeDefinition::default_dark())`
Replace `Theme::claude_light()` → `Theme::from_definition(&ThemeDefinition::default_light())`

- [ ] **Step 3: Fix `resolve_*` tests — they need registry + pair now**

```rust
#[test]
fn resolve_system_follows_winit_dark() {
    let registry = ThemeRegistry::new();
    let pair = ActiveThemePair::default();
    let t = Theme::resolve(
        crate::settings::ThemeMode::System,
        winit::window::Theme::Dark,
        &pair,
        &registry,
    );
    assert!(t.is_dark);
}
```

Update all 7 tests similarly.

- [ ] **Step 4: Run `cargo test -p edit-plus-ui` — all theme.rs tests pass**

---

### Task 6: Fix all ThemeMode match arms across codebase

**Files (all that reference ThemeMode variants):**
- `crates/ui/src/widgets/sidebar/types.rs`
- `crates/ui/src/widgets/sidebar/state.rs`
- `crates/ui/src/widgets/sidebar/menu.rs`
- `crates/ui/src/widgets/popup_menu/types.rs`
- `crates/app/src/actions.rs`
- `crates/app/src/events.rs`
- `crates/app/src/app_dispatch.rs`
- `crates/app/src/settings_io.rs`
- `crates/app/src/app_lifecycle.rs`
- `crates/app/src/menu_handler.rs`
- `crates/app/src/native_menu.rs`

- [ ] **Step 1: Find all match arms on ThemeMode**

Run: `grep -rn 'ClaudeLight\|ClaudeDark' crates/ --include='*.rs'`

- [ ] **Step 2: For each match arm, remove ClaudeLight/ClaudeDark variants**

Most cases follow the pattern:
```rust
match mode {
    ThemeMode::System => ...,
    ThemeMode::Dark => ...,
    ThemeMode::Light => ...,
    ThemeMode::ClaudeLight => ...,  // remove
    ThemeMode::ClaudeDark => ...,   // remove
}
```

If the old code used `ClaudeLight`/`ClaudeDark` to apply a specific theme, replace with the new pattern using `ActiveThemePair` + `ThemeRegistry`.

- [ ] **Step 3: Update `app_dispatch.rs` — `apply_theme_mode()` and `rebuild_theme()`**

`apply_theme_mode()`: persists `ThemeMode` to settings. Handle `ActiveThemePair` separately — add `apply_active_theme(mode: ActiveThemePair)` for theme switching.

`rebuild_theme()`: calls `Theme::resolve(mode, system_theme, &pair, &registry)` with the app-owned registry + pair from settings.

- [ ] **Step 4: Update `app_lifecycle.rs` — startup to create registry**

Add `let theme_registry = ThemeRegistry::new();` to app state. Pass to `rebuild_theme()`.

- [ ] **Step 5: Run `cargo check` from workspace root**

---

### Task 7: Fix all widget test helpers that used removed constructors

**Files:**
- `crates/ui/src/widgets/scrollbar.rs`
- `crates/ui/src/widgets/status_bar.rs`
- `crates/ui/src/widgets/title_bar.rs`
- `crates/ui/src/widgets/sidebar/widget_tests.rs`
- `crates/ui/src/widgets/tab_bar/widget.rs`
- `crates/ui/src/widgets/button.rs`
- `crates/ui/src/widgets/popup_menu/mod.rs`
- `crates/ui/src/widgets/toc.rs`
- `crates/ui/src/widgets/search_bar.rs`
- `crates/ui/src/widgets/text_box.rs`
- `crates/app/src/ui_shell.rs`

- [ ] **Step 1: In each file, find local `fn test_theme()` that mutates specific fields**

Most tests start from `crate::theme::test_theme()` and override specific fields. Since `test_theme()` already returns a `Theme` via `from_definition()`, the local overrides still work — the Theme struct fields are unchanged. These should compile as-is after plan 1+2.

- [ ] **Step 2: Verify — run `cargo test` from workspace root**

---

### Task 8: Full workspace build and test

- [ ] **Step 1: Run `cargo build`**

Expected: no errors.

- [ ] **Step 2: Run `cargo test`**

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: theme registry + simplified ThemeMode with ActiveThemePair

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```
