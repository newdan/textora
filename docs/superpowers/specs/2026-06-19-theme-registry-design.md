# Theme Registry + ThemeMode Simplification

## Summary

Introduce `ThemeDefinition` (pure data), `ThemeRegistry` (key → definition map), and `ActiveThemePair` (selected dark/light identifiers). Simplify `ThemeMode` to System/Dark/Light. Remove per-theme enum variants.

## Motivation

- `ThemeMode` currently hardcodes theme names (ClaudeLight/ClaudeDark) — can't add new themes
- No concept of "which dark theme" vs "which light theme" — they're always paired by name convention
- Plan 4 (config files) needs a registry to load into
- Construction logic mixed with data — can't serialize or introspect

## Design

### ThemeDefinition

Pure data — the serializable form of a theme. All field values explicit (no inheritance, no mutation).

**Color space:** Values in `ThemeDefinition` are in **sRGB** space (standard hex colors). `Theme::from_definition()` applies `gamma_correct()` to convert from sRGB to linear for GPU rendering. Theme authors write normal hex values; the engine handles the conversion.

All nested types (`ColorPalette`, `EditorTheme`, `MarkdownTheme`, `MarkdownSpacing`) must also derive `Serialize`/`Deserialize` for Plan 4 config file support.

```rust
/// Pure-data theme definition — values only, no logic.
/// Stored in sRGB space. from_definition() converts to linear.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    /// Display name shown in UI (e.g. "默认黑")
    pub display_name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    /// Scope → color map. BTreeMap for deterministic ordering in config files.
    pub scopes: BTreeMap<String, [f32; 4]>,
}
```

`Theme::from_definition(def: &ThemeDefinition) -> Theme`:
- Copies all fields into a `Theme` (converting `BTreeMap` → `HashMap` for runtime)
- Applies `gamma_correct()` — sRGB → linear conversion
- Returns the ready-to-use `Theme`

### ThemeRegistry

**Ownership:** Held by the `App` struct alongside `Settings`. Not a global singleton — this allows Plan 4 hot-reload via `registry.reload()` triggered by filesystem events.

```rust
/// Maps theme identifier → definition.
/// Identifiers are stable keys (e.g. "default-dark"), never shown to users.
/// Built-in defaults are unremovable and always available as fallback.
pub struct ThemeRegistry {
    themes: HashMap<String, ThemeDefinition>,
    /// Unremovable built-in default for fallback.
    default_dark: ThemeDefinition,
    default_light: ThemeDefinition,
}

impl ThemeRegistry {
    /// Create registry with built-in themes.
    pub fn new() -> Self

    /// Register a theme (from config file). Fails if id clashes with built-in.
    pub fn register(&mut self, id: String, def: ThemeDefinition)

    /// Remove a user-registered theme. Built-ins cannot be removed.
    pub fn unregister(&mut self, id: &str) -> bool

    /// Look up a theme, falling back to the built-in default for the given is_dark.
    /// This NEVER returns None — built-in defaults are always present.
    pub fn get_or_default(&self, id: &str, prefer_dark: bool) -> &ThemeDefinition

    /// Raw lookup without fallback (for listing).
    pub fn get(&self, id: &str) -> Option<&ThemeDefinition>

    /// List all registered theme identifiers.
    pub fn ids(&self) -> impl Iterator<Item = &str>

    /// (Plan 4) Reload themes from filesystem, preserving built-ins.
    pub fn reload(&mut self) -> Result<(), Error>
}
```

Built-in defaults (stored in dedicated fields, never in the removable `themes` map):

| Identifier | Display Name | is_dark | Values |
|---|---|---|---|
| `default-dark` | 默认黑 | true | Claude Dark |
| `default-light` | 默认亮 | false | Claude Light |

### ActiveThemePair

```rust
/// Which specific themes are currently active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveThemePair {
    /// Registry identifier for the dark theme.
    pub dark: String,   // e.g. "default-dark"
    /// Registry identifier for the light theme.
    pub light: String,  // e.g. "default-light"
}
```

### ThemeMode simplified

```rust
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}
```

Remove: `ClaudeLight`, `ClaudeDark` variants.

### Theme resolution flow

```rust
fn resolve(
    mode: ThemeMode,
    system_theme: winit::Theme,
    pair: &ActiveThemePair,
    registry: &ThemeRegistry,
) -> Theme {
    let want_dark = match mode {
        System => system_theme == winit::Theme::Dark,
        Dark => true,
        Light => false,
    };
    let id = if want_dark { &pair.dark } else { &pair.light };
    let def = registry.get_or_default(id, want_dark);
    Theme::from_definition(def)
}
```

`get_or_default()` ensures: if the user's chosen theme is missing (e.g., config file deleted), the built-in default of matching `is_dark` is returned — **never panics**.

### Settings integration

Add `active_themes` to `Settings`:

```rust
pub struct Settings {
    pub theme_mode: ThemeMode,        // System / Dark / Light
    pub active_themes: ActiveThemePair, // which dark/light themes
    // ... other settings unchanged
}
```

Default: `ActiveThemePair { dark: "default-dark".into(), light: "default-light".into() }`.

- `apply_theme_mode()` persists `ThemeMode` to settings as before
- Theme switching in sidebar menu: lists `registry.ids()`, lets user pick which dark/light theme
- Changing active theme updates `settings.active_themes` → persists → calls `rebuild_theme()`

### App lifecycle

```
startup:
  1. ThemeRegistry::new()            → built-in 2 themes
  2. (Plan 4: scan themes/ dir)      → load file-based themes into registry
  3. Load settings                    → get ThemeMode + ActiveThemePair
  4. resolve()                        → build active Theme
  5. On settings change: rebuild      → re-resolve from registry

runtime theme switch (sidebar):
  1. User picks "默认黑" from dark theme list
  2. Update settings.active_themes.dark → persist
  3. Rebuild theme via resolve()
```

### Removal of old constructors

- `Theme::dark()` — remove (edit+ Dark is not a built-in)
- `Theme::light()` — remove (edit+ Light is not a built-in)
- `Theme::claude_dark()` — values become `ThemeDefinition` for `default-dark`
- `Theme::claude_light()` — values become `ThemeDefinition` for `default-light`
- Old `test_theme()` — keep, updated to use new structure (already done in Plan 1/2)
- Tests that used `Theme::dark()` / `Theme::light()` for quick test themes: update to use `Theme::from_definition()` with a minimal `ThemeDefinition`

### Fallback behavior

Built-in defaults are stored in dedicated fields (`default_dark`/`default_light`), separate from the user-registered `themes` map. They cannot be removed or overridden.

`get_or_default(id, prefer_dark)`:
1. Look up `id` in the user-registered `themes` map
2. If found → return it
3. If not found → return `default_dark` or `default_light` based on `prefer_dark`

This guarantees `resolve()` never panics on missing theme references.

## Implementation phases

### Phase 1: ThemeDefinition + Theme::from_definition
- Define `ThemeDefinition` struct in `theme.rs`
- Implement `Theme::from_definition()`
- Convert `claude_dark()` / `claude_light()` values into `const`/`fn` `ThemeDefinition` instances
- Keep old constructors temporarily for compilation

### Phase 2: ThemeRegistry
- New file `theme_registry.rs`
- `ThemeRegistry` with `new()`, `register()`, `get()`, `ids()`
- Built-in registration of `default-dark` and `default-light`
- Tests

### Phase 3: ThemeMode simplification + ActiveThemePair
- Strip `ThemeMode` to System/Dark/Light
- Add `ActiveThemePair` to `Settings` with defaults
- Update `resolve()` to take registry + pair
- Update `from_winit()` similarly
- Update all `ThemeMode` match arms (remove ClaudeLight/ClaudeDark)
- Update app code: sidebar menu, settings IO, native menu, actions, events

### Phase 4: Remove old constructors, fix tests
- Remove `Theme::dark()`, `Theme::light()`, `Theme::claude_dark()`, `Theme::claude_light()`
- Update all tests that used removed constructors
- Full workspace build + test

## Decisions

| Decision | Rationale |
|---|---|
| `ThemeDefinition` as pure data (not factory) | Required for Plan 4 serialization |
| `Serialize`/`Deserialize` on ThemeDefinition + all nested types | ColorPalette, EditorTheme, MarkdownTheme, MarkdownSpacing all derive serde |
| `scopes` as `BTreeMap` (not `Vec`) | Natural TOML `[scopes]` section; deterministic ordering |
| Color space: ThemeDefinition = sRGB, Theme = linear | Theme authors write hex; from_definition applies gamma |
| Registry owned by `App`, not global singleton | Enables Plan 4 hot-reload via `reload()` |
| Built-in defaults in dedicated fields, never in removable map | `get_or_default()` never returns None — no panic path |
| Built-in themes = 2 (default-dark, default-light) | Minimal set; config files provide the rest |
| Display name separate from identifier | Identifier is stable for settings; display name can change |
| Registry key = file name without extension | Plan 4 — file `claude-dark.toml` → key `claude-dark` |
| `ThemeMode` 3 variants only | User only cares about light vs dark; specific theme is a separate choice |
| `active_themes` as a single Settings field | One concept, one field — not two separate settings |
