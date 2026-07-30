# Theme Config File — TOML Format

## Summary

User themes as TOML files in `~/.config/edit+/themes/*.toml`. Colors written as hex strings (`"#74ADE8"`), converted to `[f32; 4]` via a custom serde module. Files auto-load at startup into `ThemeRegistry`.

## Motivation

- All 4 themes currently hardcoded in Rust — users can't add or tweak themes
- After Plan 1/2, `ThemeDefinition` + `ThemeRegistry` provide the target structure
- TOML chosen for readability (comments, no quote noise for keys, native hex readability)

## Design

### File location

```
~/.config/edit+/themes/
├── claude-dark.toml      # filename = registry key "claude-dark"
├── claude-light.toml     # registry key "claude-light"
└── my-custom.toml        # registry key "my-custom"
```

Registry key = filename without extension.

### Hex color format

`serde` module `hex_color` handles deserialization of `[f32; 4]` from hex strings:

| Input | Output | Notes |
|---|---|---|
| `"#74ADE8"` | `[0.4549, 0.6784, 0.9098, 1.0]` | 6-char RGB, alpha=1 |
| `"#74ADE83D"` | `[0.4549, 0.6784, 0.9098, 0.2392]` | 8-char RGBA |
| `"74ADE8"` | same as above | `#` optional |

All `[f32; 4]` fields in `ThemeDefinition` annotated with `#[serde(with = "hex_color")]`.

### Inheritance & partial override

To solve forward compatibility (new fields added to `ColorPalette` later, old user TOML files don't have them), user TOML files are **partial overlays** on a base theme.

```toml
# my-custom.toml — only override what you want to change
extends = "default-dark"
display_name = "My Custom Dark"
is_dark = true

[palette]
accent = "#FF6B6B"
bg_surface = "#1E1E24"

[editor]
cursor = "#FF6B6B"
```

Any field not specified inherits from the base. Adding `bg_sidebar` to `ColorPalette` later doesn't break existing user files — they simply inherit the base value.

**Deserialization structs:**

```rust
/// What the user writes in a TOML file — all fields optional.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    extends: Option<String>,
    display_name: String,
    is_dark: bool,
    #[serde(default)]
    palette: PartialPalette,
    #[serde(default)]
    editor: PartialEditorTheme,
    #[serde(default)]
    markdown: PartialMarkdownTheme,
    #[serde(default)]
    scopes: BTreeMap<String, String>,  // hex strings
}

/// Every color field is Option — None = inherit from base.
#[derive(Deserialize, Default)]
#[serde(default)]
struct PartialPalette {
    bg_base: Option<String>,
    bg_surface: Option<String>,
    // ... all 18 fields as Option<String>
}

// PartialEditorTheme, PartialMarkdownTheme, PartialMarkdownSpacing — same pattern
```

**Resolution:** `ThemeFile::resolve(base: &ThemeDefinition) -> ThemeDefinition`:
1. Start with `base.clone()`
2. For each partial sub-struct, override only the `Some` fields (parsing hex → `[f32; 4]`)
3. Merge `scopes`: user entries override base, non-overridden base entries kept
4. `extends` defaults to built-in default matching `is_dark`

Built-in themes are full `ThemeDefinition` instances (not fragments), registered as unremovable defaults. They serve as both fallback targets and `extends` bases.

### Serde module

```rust
// crates/ui/src/hex_color.rs
mod hex_color {
    pub fn serialize<S>(color: &[f32; 4], serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[f32; 4], D::Error>
    where D: serde::Deserializer<'de>;
}
```

Deserialize validates:
- Length: 6 or 8 hex chars (after stripping `#`)
- Valid hex digits only
- Returns error with context on failure (filename + field path)

**Float precision in serialize:** Colors are stored as `[f32; 4]` (0.0–1.0). To convert back to hex:
- `(c[0] * 255.0).round() as u8` — **round() not trunc()**, otherwise `0.45490196 * 255 = 115.999...` truncates to `115` (#73) instead of `116` (#74)
- 6-char output if alpha ≥ 0.999, 8-char otherwise

Round-trip test: serialize built-in → TOML → deserialize → compare `[f32; 4]` values within epsilon. Hex strings must match exactly.

### TOML file schema

User files are **partial overlays** — only override what you want to change. Full ThemeDefinition files also work (omit `extends` and write all fields).

```toml
# my-custom.toml — partial overlay example
extends = "default-dark"
display_name = "My Custom Dark"
is_dark = true

[palette]
# Only the fields you want to override from the base
accent = "#FF6B6B"
bg_surface = "#1E1E24"

[editor]
cursor = "#FF6B6B"

[scopes]
# Override specific scopes; rest inherit from base
string = "#6BFF6B"
```

Full definition example (no `extends`, all fields explicit):

```toml
display_name = "Claude Dark"
is_dark = true

[palette]
bg_base = "#1A1C21"
# ... all 18 palette fields ...

[editor]
# ... all 8 editor fields ...

[markdown]
# ... all 9 markdown color fields ...

[markdown.spacing]
# ... all 15 spacing fields ...

[scopes]
# ... all scope entries ...
```

### Loading flow

```
startup:
  1. Register built-in defaults ("default-dark", "default-light") as full ThemeDefinitions
  2. List ~/.config/edit+/themes/*.toml (sorted alphabetically)
  3. For each file:
     a. Read file content
     b. toml::from_str::<ThemeFile>(&content)
     c. Resolve extends base (defaults to built-in of matching is_dark)
     d. Overlay partial fields onto base → ThemeDefinition
     e. On success: registry.register(filename, def)
     f. On error: log warning with file path + error, skip this file
  4. Active theme pair resolved from registry (Plan 3)
```

### Validation

At load time:
- TOML parse errors → logged, file skipped
- Hex parse errors → logged with field path, file skipped
- Missing `display_name` or `is_dark` → logged, file skipped
- `extends` references unknown theme ID → logged, file skipped
- Unknown fields → error (`#[serde(deny_unknown_fields)]` on ThemeFile) — catch typos early

No validation that every scope key exists or that color values look "right" — the hex parser is the only gate.

### Error messages

```
WARN theme: failed to load ~/.config/edit+/themes/broken.toml: TOML parse error at line 5, column 12
WARN theme: invalid hex color in my-theme.toml `palette.bg_base`: expected 6 or 8 hex chars, got "#XYZ"
WARN theme: missing required field `display_name` in bad-theme.toml
```

### Integration with ThemeRegistry (from Plan 3)

```rust
impl ThemeRegistry {
    /// Scan themes/ directory and register all valid .toml files.
    /// Existing user-registered themes are cleared first.
    /// Built-in defaults are preserved.
    pub fn load_user_themes(&mut self, themes_dir: &Path) -> Vec<LoadError>;

    /// (Hot-reload) Clear user themes and re-scan.
    pub fn reload(&mut self, themes_dir: &Path) -> Result<(), Vec<LoadError>>;
}
```

### Hot-reload (Plan 4 extension, not blocking)

Watch `~/.config/edit+/themes/` with `notify` crate. On file change → call `registry.reload()` → if active theme was affected, trigger `rebuild_theme()`.

### serde dependencies

```toml
# Cargo.toml additions
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

`serde` is already likely a transitive dependency somewhere. `toml` is new.

## Implementation phases

### Phase 1: hex_color serde module
- New file `crates/ui/src/hex_color.rs`
- `serialize`: `[f32; 4]` → hex string (using `.round() as u8`)
- `deserialize`: hex string → `[f32; 4]`
- Unit tests: valid 6-char, valid 8-char, no-prefix, invalid chars, wrong length, **round-trip precision test**
- Add `serde` dependency with `derive` feature to `edit-plus-ui`

### Phase 2: Partial override structs + ThemeFile
- New file `crates/ui/src/theme_file.rs`
- Define `PartialPalette`, `PartialEditorTheme`, `PartialMarkdownTheme`, `PartialMarkdownSpacing` — all fields `Option<String>` with `#[serde(default)]`
- Define `ThemeFile` with `extends`, `display_name`, `is_dark`, partial sub-structs, `scopes: BTreeMap<String, String>`
- Implement `ThemeFile::resolve(&self, base: &ThemeDefinition) -> Result<ThemeDefinition, Error>`
- Serde derives on the partial structs (`Deserialize` only)
- Unit tests: full override, partial override, scope merge, missing extends defaults to built-in

### Phase 3: Serde on ThemeDefinition + nested types
- Add `#[derive(Serialize, Deserialize)]` to: `ColorPalette`, `EditorTheme`, `MarkdownTheme`, `MarkdownSpacing`, `ThemeDefinition`
- Add `#[serde(with = "hex_color")]` to every `[f32; 4]` field
- Round-trip test: ThemeDefinition → serialize → TOML → deserialize → fields match within epsilon; hex values match exactly

### Phase 4: ThemeRegistry::load_user_themes
- Create themes directory on first run: `dirs::config_dir()/edit+/themes/`
- Implement `load_user_themes()` + `reload()`
- Load flow: parse `ThemeFile` → resolve against base → register as `ThemeDefinition`
- Wire into app startup (after registry creation, before theme resolution)
- Logging for load errors with file path + field context
- Tests: valid partial file, invalid TOML, bad hex, unknown extends target, missing required fields

### Phase 5: Hot-reload (optional, can defer)
- `notify` crate for filesystem watching
- On change → reload → rebuild if active theme changed

## Decisions

| Decision | Rationale |
|---|---|
| TOML over JSON | Comments, no quote noise for simple keys, standard for Rust config |
| Hex strings over float arrays | Human-writable; `#DE7356` is what people expect |
| Inheritance: `extends` + partial overlay | Forward compat — new palette fields don't break old user files |
| User files are `ThemeFile` (partial), not `ThemeDefinition` (full) | `Option` fields = inherit from base; only override what's needed |
| `#[serde(deny_unknown_fields)]` on ThemeFile | Catch typos at load time; no silent ignore of misspelled keys |
| `serde(with = "hex_color")` on full ThemeDefinition only | Partial structs use raw `Option<String>` — resolve() does hex parsing |
| Serialize: `.round() as u8` (not `.trunc()`) | 0.45490196 * 255 = 115.999... → round to 116, not trunc to 115 |
| Scopes as TOML `[scopes]` table (BTreeMap) | Natural map syntax; dots in keys need quoting but it's standard TOML |
| File skip on error (not panic) | One broken theme shouldn't crash the app; log + continue |
| `~/.config/edit+/themes/` | XDG convention; `dirs` crate resolves platform path |
| Built-in defaults as full ThemeDefinitions (not TOML strings) | Simpler — no TOML parsing for built-ins; partial files still extend them |
| Notify hot-reload deferred | Adds complexity; core loading flow works without it |
