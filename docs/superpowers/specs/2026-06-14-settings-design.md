# Settings Popup Menu — Simplified Design Spec

## Overview

Fix the settings button (gear icon in sidebar) and enhance its popup menu with toggle/radio items for quick settings changes. No modal overlay, no custom controls, no Done/Cancel — all changes apply instantly.

## Menu Layout

```
   ✓ 显示行号              ← toggle (show_line_numbers)
   ✓ 自动换行              ← toggle (word_wrap)
   ─────────
     跟随系统              ← radio (theme_mode: System)
   ✓ 深色模式              ← radio (theme_mode: Dark)
     浅色模式              ← radio (theme_mode: Light)
   ─────────
     打开 settings.yaml    ← action
   ─────────
   ✓ Sidebar 模式          ← radio (view_mode: Sidebar)
     Tabs 模式             ← radio (view_mode: Tabs)
```

Active items get a checkmark. Toggles flip immediately. Radio items apply the selected value and close the menu. "打开 settings.yaml" opens the file in the editor.

## Data Changes

```rust
// New enum in settings.rs
pub enum ThemeMode {
    System,  // follow winit window theme
    Dark,
    Light,
}

// New fields in Settings struct:
// - theme_mode: ThemeMode (default System)
// - line_height_ratio: f32 (default 1.618)

// Remove hardcoded 1.618 from set_font_size — derive line_height from font_size * line_height_ratio
```

## Theme Resolution

```rust
fn resolve_theme(mode: ThemeMode, system_theme: winit::window::Theme) -> Theme {
    match mode {
        ThemeMode::System => Theme::from_winit(system_theme),
        ThemeMode::Dark  => Theme::dark(),
        ThemeMode::Light => Theme::light(),
    }
}
```

When `theme_mode` changes, immediately rebuild Theme and trigger full redraw.

## Bug Fix

`SidebarState::paint()` does not render `self.open_menu` — the menu is created and hit-tested but never drawn. Add menu rendering at the end of `paint()` using `PopupMenu::paint()`.

## New SidebarActions

- `ToggleLineNumbers` — flip `show_line_numbers`
- `ToggleWordWrap` — flip `word_wrap`
- `SetThemeMode(ThemeMode)` — set theme_mode, rebuild theme

## App Layer Handling

- `ToggleLineNumbers` / `ToggleWordWrap`: `Settings::with_mut` flip the bool, request redraw
- `SetThemeMode(mode)`: update settings, call `resolve_theme()`, trigger full redraw
- Existing `SetViewMode` / `OpenSettingsFile` unchanged

## Persistence

All settings already persist to `~/.edit+/settings.yaml` on app exit. New fields (`theme_mode`, `line_height_ratio`) are included automatically via serde.

## Scope Boundaries

**In scope:**
- Fix settings button rendering bug
- 9 menu items (4 toggles/radios + 1 action + 2 separators)
- Theme de/serialization to settings.yaml
- Instant apply, no cancel needed

**Out of scope:**
- Modal overlay, custom controls, tab focus, keyboard navigation
- Font family/size pickers
- Tab width setting
