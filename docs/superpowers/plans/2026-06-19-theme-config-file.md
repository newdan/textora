# Theme Config File (TOML) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load user themes from `~/.config/edit+/themes/*.toml` files with hex color support and inheritance-based partial overrides.

**Architecture:** `hex_color` serde module converts hex strings ↔ `[f32; 4]`. `ThemeFile` is the deserialized partial form with all-Option fields; `ThemeFile::resolve(base)` overlays onto a base `ThemeDefinition`. `ThemeRegistry::load_user_themes()` scans the directory and registers resolved definitions.

**Tech Stack:** Rust, new dependencies: `serde` (derive), `toml` (0.8), `dirs`.

**Prerequisite:** Plans 1+2+3 must be complete (ThemeRegistry exists, ThemeDefinition exists, ThemeMode simplified).

---

### Task 1: Add dependencies

**Files:**
- Modify: `crates/ui/Cargo.toml`

- [ ] **Step 1: Add serde and toml dependencies**

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`** to verify dependency resolution

---

### Task 2: Create hex_color serde module

**Files:**
- Create: `crates/ui/src/hex_color.rs`
- Modify: `crates/ui/src/lib.rs` (add `pub mod hex_color;`)

- [ ] **Step 1: Write hex_color module**

```rust
//! serde module for [f32; 4] ↔ hex string (#RRGGBB or #RRGGBBAA).

pub fn serialize<S>(color: &[f32; 4], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let r = (color[0] * 255.0).round() as u8;
    let g = (color[1] * 255.0).round() as u8;
    let b = (color[2] * 255.0).round() as u8;
    let a = (color[3] * 255.0).round() as u8;
    if a == 255 {
        serializer.serialize_str(&format!("#{:02X}{:02X}{:02X}", r, g, b))
    } else {
        serializer.serialize_str(&format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a))
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<[f32; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    let hex = s.strip_prefix('#').unwrap_or(&s);
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| invalid_hex(&s))?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| invalid_hex(&s))?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| invalid_hex(&s))?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| invalid_hex(&s))?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| invalid_hex(&s))?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| invalid_hex(&s))?;
            let a = u8::from_str_radix(&hex[6..8], 16).map_err(|_| invalid_hex(&s))?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0])
        }
        _ => Err(invalid_hex(&s)),
    }
}

fn invalid_hex(s: &str) -> serde::de::Error {
    // Use a custom error type or use serde::de::value::Error
    // This will be wrapped by TOML parsing with field context
    serde::de::Error::custom(format_args!("invalid hex color: expected 6 or 8 hex chars, got \"{}\"", s))
}
```

- [ ] **Step 2: Write tests in the same file under `#[cfg(test)]`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_6_char_rgb() {
        let json = "\"#74ADE8\"";
        let color: [f32; 4] = serde_json::from_str(json).unwrap();
        assert!((color[0] - 0.4549).abs() < 0.01);
        assert!((color[1] - 0.6784).abs() < 0.01);
        assert!((color[2] - 0.9098).abs() < 0.01);
        assert_eq!(color[3], 1.0);
    }

    #[test]
    fn deserialize_8_char_rgba() {
        let json = "\"#74ADE83D\"";
        let color: [f32; 4] = serde_json::from_str(json).unwrap();
        assert!((color[3] - 0.2392).abs() < 0.01);
    }

    #[test]
    fn deserialize_no_prefix() {
        let json = "\"74ADE8\"";
        let color: [f32; 4] = serde_json::from_str(json).unwrap();
        assert!((color[0] - 0.4549).abs() < 0.01);
    }

    #[test]
    fn deserialize_invalid_length() {
        let json = "\"#74ADE\"";
        let err = serde_json::from_str::<[f32; 4]>(json).unwrap_err();
        assert!(err.to_string().contains("hex"));
    }

    #[test]
    fn round_trip_preserves_hex() {
        let original = "#74ADE8";
        let color: [f32; 4] = serde_json::from_str(&format!("\"{}\"", original)).unwrap();
        let serialized = serde_json::to_string(&color).unwrap();
        assert_eq!(serialized, format!("\"{}\"", original));
    }

    #[test]
    fn round_trip_preserves_hex_with_alpha() {
        let original = "#74ADE83D";
        let color: [f32; 4] = serde_json::from_str(&format!("\"{}\"", original)).unwrap();
        let serialized = serde_json::to_string(&color).unwrap();
        assert_eq!(serialized, format!("\"{}\"", original));
    }

    #[test]
    fn serialize_skips_alpha_when_opaque() {
        let color = [0.4549, 0.6784, 0.9098, 1.0];
        let serialized = serde_json::to_string(&color).unwrap();
        // Should be 6 chars (no AA suffix)
        assert!(!serialized.contains("\"#74ADE8FF\""));
    }

    #[test]
    fn round_trip_precision_no_truncation() {
        // The value 116/255 = 0.45490196... must round-trip correctly
        let color = [116.0 / 255.0, 173.0 / 255.0, 232.0 / 255.0, 1.0];
        let serialized = serde_json::to_string(&color).unwrap();
        assert_eq!(serialized, "\"#74ADE8\"");
    }
}
```

Requires `serde_json` as dev-dependency for testing:
```toml
[dev-dependencies]
serde_json = "1"
```

- [ ] **Step 3: Run `cargo test -p edit-plus-ui -- hex_color`** — all 8 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/hex_color.rs crates/ui/src/lib.rs crates/ui/Cargo.toml
git commit -m "feat(ui): hex_color serde module for theme config files

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: Add serde derives to theme types

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Add serde derives and hex_color annotations to all nested types**

Update struct definitions:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorPalette {
    #[serde(with = "crate::hex_color")]
    pub bg_base: [f32; 4],
    #[serde(with = "crate::hex_color")]
    pub bg_surface: [f32; 4],
    // ... all 18 fields with #[serde(with = "crate::hex_color")]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorTheme {
    #[serde(with = "crate::hex_color")]
    pub background: [f32; 4],
    // ... all 8 fields
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownTheme {
    #[serde(with = "crate::hex_color")]
    pub heading: [f32; 4],
    // ... all 9 color fields

    pub spacing: MarkdownSpacing,  // no hex_color on f32 fields
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownSpacing {
    // all f32 fields — serde handles natively
    pub paragraph_spacing: f32,
    // ...
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub display_name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub scopes: BTreeMap<String, [f32; 4]>,
}
```

For `BTreeMap<String, [f32; 4]>`, serde can't handle `[f32; 4]` as a map value natively. Need a custom serializer or use `HashMap<String, [f32; 4]>` with `#[serde(with = "crate::hex_color")]` on values. Since BTreeMap with custom value serde is complex, use this approach:

```rust
// In hex_color.rs, add a module for scope map serialization
pub mod scope_map {
    // serialize: BTreeMap<String, [f32; 4]> → TOML [scopes] table
    // deserialize: TOML [scopes] table → BTreeMap<String, [f32; 4]>
}
```

Then annotate: `#[serde(with = "crate::hex_color::scope_map")]` on the scopes field.

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`**

---

### Task 4: Create Partial* override structs and ThemeFile

**Files:**
- Create: `crates/ui/src/theme_file.rs`
- Modify: `crates/ui/src/lib.rs` (add `pub mod theme_file;`)

- [ ] **Step 1: Define partial override structs**

```rust
use serde::Deserialize;
use std::collections::BTreeMap;

/// Partial ColorPalette — every field optional.
/// None = inherit from base theme.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PartialPalette {
    pub bg_base: Option<String>,
    pub bg_surface: Option<String>,
    pub bg_elevated: Option<String>,
    pub bg_hover: Option<String>,
    pub bg_active: Option<String>,
    pub text_main: Option<String>,
    pub text_muted: Option<String>,
    pub text_inverse: Option<String>,
    pub border_subtle: Option<String>,
    pub border_strong: Option<String>,
    pub shadow: Option<String>,
    pub accent: Option<String>,
    pub highlight: Option<String>,
    pub danger: Option<String>,
    pub warning: Option<String>,
    pub input_bg: Option<String>,
    pub input_border: Option<String>,
    pub input_fg: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PartialEditorTheme {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub gutter_bg: Option<String>,
    pub line_number: Option<String>,
    pub selection: Option<String>,
    pub cursor: Option<String>,
    pub scrollbar_track: Option<String>,
    pub scrollbar_thumb: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PartialMarkdownTheme {
    pub heading: Option<String>,
    pub link: Option<String>,
    pub inline_code: Option<String>,
    pub code_bg: Option<String>,
    pub code_block_bg: Option<String>,
    pub toc_background: Option<String>,
    pub toc_active_background: Option<String>,
    pub toc_text: Option<String>,
    pub toc_level_indicator: Option<String>,
    pub spacing: Option<PartialMarkdownSpacing>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PartialMarkdownSpacing {
    pub paragraph_spacing: Option<f32>,
    pub heading_spacing_top: Option<f32>,
    pub heading_spacing_bottom: Option<f32>,
    pub list_item_spacing: Option<f32>,
    pub list_group_spacing: Option<f32>,
    pub list_indent: Option<f32>,
    pub code_block_padding: Option<f32>,
    pub blockquote_padding: Option<f32>,
    pub table_cell_padding: Option<f32>,
    pub rule_spacing: Option<f32>,
    pub rule_thickness: Option<f32>,
    pub rule_width_ratio: Option<f32>,
    pub border_radius_base: Option<f32>,
    pub border_radius_small: Option<f32>,
    pub code_line_height: Option<f32>,
}

/// What the user writes in a .toml file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    #[serde(default)]
    pub extends: Option<String>,
    pub display_name: String,
    pub is_dark: bool,
    #[serde(default)]
    pub palette: PartialPalette,
    #[serde(default)]
    pub editor: PartialEditorTheme,
    #[serde(default)]
    pub markdown: PartialMarkdownTheme,
    #[serde(default)]
    pub scopes: BTreeMap<String, String>,
}
```

- [ ] **Step 2: Implement ThemeFile::resolve()**

```rust
impl ThemeFile {
    /// Resolve this partial file against a base ThemeDefinition.
    /// Overlays non-None fields; merges scopes (user wins).
    pub fn resolve(&self, base: &ThemeDefinition) -> Result<ThemeDefinition, ResolveError> {
        let mut def = base.clone();
        def.display_name = self.display_name.clone();
        def.is_dark = self.is_dark;

        // Overlay palette colors
        overlay_color(&mut def.palette.bg_base, &self.palette.bg_base)?;
        overlay_color(&mut def.palette.bg_surface, &self.palette.bg_surface)?;
        // ... all 18 palette fields

        // Overlay editor colors
        overlay_color(&mut def.editor.background, &self.editor.background)?;
        // ... all 8 editor fields

        // Overlay markdown colors
        overlay_color(&mut def.markdown.heading, &self.markdown.heading)?;
        // ... all 9 markdown color fields

        // Overlay markdown spacing
        if let Some(ref sp) = self.markdown.spacing {
            overlay_f32(&mut def.markdown.spacing.paragraph_spacing, sp.paragraph_spacing);
            // ... all 15 spacing fields
        }

        // Merge scopes: user values override base
        for (key, hex) in &self.scopes {
            let color = parse_hex(hex)?;
            def.scopes.insert(key.clone(), color);
        }

        Ok(def)
    }
}

fn overlay_color(target: &mut [f32; 4], source: &Option<String>) -> Result<(), ResolveError> {
    if let Some(ref hex) = source {
        *target = parse_hex(hex)?;
    }
    Ok(())
}

fn overlay_f32(target: &mut f32, source: Option<f32>) {
    if let Some(v) = source { *target = v; }
}

fn parse_hex(hex: &str) -> Result<[f32; 4], ResolveError> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    let len = s.len();
    if len != 6 && len != 8 {
        return Err(ResolveError::InvalidHex(hex.into()));
    }
    let parse = |i: usize| u8::from_str_radix(&s[i..i+2], 16)
        .map_err(|_| ResolveError::InvalidHex(hex.into()));
    let r = parse(0)? as f32 / 255.0;
    let g = parse(2)? as f32 / 255.0;
    let b = parse(4)? as f32 / 255.0;
    let a = if len == 8 { parse(6)? as f32 / 255.0 } else { 1.0 };
    Ok([r, g, b, a])
}

#[derive(Debug)]
pub enum ResolveError {
    InvalidHex(String),
}
```

- [ ] **Step 3: Write unit tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn base_theme() -> ThemeDefinition {
        ThemeDefinition::default_dark()
    }

    #[test]
    fn resolve_overrides_palette_field() {
        let file = ThemeFile {
            extends: None,
            display_name: "Test".into(),
            is_dark: true,
            palette: {
                let mut p = PartialPalette::default();
                p.accent = Some("#FF0000".into());
                p
            },
            ..Default::default()  // needs Default impl
        };
        let resolved = file.resolve(&base_theme()).unwrap();
        assert!((resolved.palette.accent[0] - 1.0).abs() < 0.01); // red
    }

    #[test]
    fn resolve_inherits_unset_fields() {
        let file = ThemeFile {
            display_name: "Test".into(),
            is_dark: true,
            ..Default::default()
        };
        let resolved = file.resolve(&base_theme()).unwrap();
        assert_eq!(resolved.palette.bg_base, base_theme().palette.bg_base);
    }

    #[test]
    fn resolve_merges_scopes() {
        let mut file = ThemeFile::default();
        file.display_name = "Test".into();
        file.is_dark = true;
        file.scopes.insert("comment".into(), "#FF0000".into());
        let resolved = file.resolve(&base_theme()).unwrap();
        assert!((resolved.scopes["comment"][0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn resolve_invalid_hex_errors() {
        let mut file = ThemeFile::default();
        file.display_name = "Test".into();
        file.is_dark = true;
        file.palette.accent = Some("#GGGGGG".into());
        assert!(file.resolve(&base_theme()).is_err());
    }
}
```

Add `#[derive(Default)]` to `ThemeFile` — requires that `display_name` and `is_dark` have defaults. Remove them from being required in tests. Actually, keep them required in real use but add a helper for tests:

```rust
impl ThemeFile {
    #[cfg(test)]
    fn test_default() -> Self {
        Self {
            extends: None,
            display_name: "test".into(),
            is_dark: true,
            palette: PartialPalette::default(),
            editor: PartialEditorTheme::default(),
            markdown: PartialMarkdownTheme::default(),
            scopes: BTreeMap::new(),
        }
    }
}
```

- [ ] **Step 4: Run `cargo test -p edit-plus-ui -- theme_file`** — all resolve tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/theme_file.rs crates/ui/src/lib.rs
git commit -m "feat(ui): ThemeFile with partial override + resolve against base

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: Implement ThemeRegistry::load_user_themes()

**Files:**
- Modify: `crates/ui/src/theme_registry.rs`
- Modify: `crates/ui/Cargo.toml` (add `dirs` dependency if not present)

- [ ] **Step 1: Add `dirs` to Cargo.toml**

```toml
[dependencies]
dirs = "5"
```

- [ ] **Step 2: Implement `load_user_themes()`**

```rust
use std::path::{Path, PathBuf};
use crate::theme_file::ThemeFile;

impl ThemeRegistry {
    /// Scan `themes_dir` for *.toml files, parse, resolve, and register.
    /// Returns errors for files that failed to load (does not stop on first error).
    pub fn load_user_themes(&mut self, themes_dir: &Path) -> Vec<LoadError> {
        let mut errors = Vec::new();

        let entries = match std::fs::read_dir(themes_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // First run — create the directory
                let _ = std::fs::create_dir_all(themes_dir);
                return Vec::new();
            }
            Err(e) => {
                errors.push(LoadError::Io(themes_dir.to_owned(), e));
                return errors;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "toml") {
                continue;
            }

            let id = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            match self.load_one(&path, &id) {
                Ok(()) => {}
                Err(e) => errors.push(e),
            }
        }

        errors
    }

    fn load_one(&mut self, path: &Path, id: &str) -> Result<(), LoadError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| LoadError::Io(path.to_owned(), e))?;

        let file: ThemeFile = toml::from_str(&content)
            .map_err(|e| LoadError::TomlParse(path.to_owned(), e))?;

        let base_id = file.extends.as_deref().unwrap_or_else(|| {
            if file.is_dark { "default-dark" } else { "default-light" }
        });

        let base = self.get(base_id)
            .ok_or_else(|| LoadError::UnknownExtends(path.to_owned(), base_id.into()))?;

        let def = file.resolve(base)
            .map_err(|e| LoadError::Resolve(path.to_owned(), format!("{:?}", e)))?;

        self.register(id.to_string(), def)
            .map_err(|e| LoadError::Register(path.to_owned(), format!("{:?}", e)))?;

        Ok(())
    }

    /// Clear user themes and re-scan. Built-in defaults are preserved.
    pub fn reload(&mut self, themes_dir: &Path) -> Vec<LoadError> {
        self.themes.clear();
        self.load_user_themes(themes_dir)
    }
}

#[derive(Debug)]
pub enum LoadError {
    Io(PathBuf, std::io::Error),
    TomlParse(PathBuf, toml::de::Error),
    UnknownExtends(PathBuf, String),
    Resolve(PathBuf, String),
    Register(PathBuf, String),
}
```

- [ ] **Step 3: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn load_valid_theme_file() {
        let dir = temp_dir();
        let theme_path = dir.path().join("test.toml");
        std::fs::write(&theme_path, r#"
display_name = "Test Theme"
is_dark = true

[palette]
accent = "#FF6B6B"
"#).unwrap();

        let mut registry = ThemeRegistry::new();
        let errors = registry.load_user_themes(dir.path());
        assert!(errors.is_empty());
        assert_eq!(registry.len(), 1);
        let def = registry.get("test").unwrap();
        assert_eq!(def.display_name, "Test Theme");
    }

    #[test]
    fn load_invalid_toml_skips_file() {
        let dir = temp_dir();
        std::fs::write(dir.path().join("bad.toml"), "not valid toml [[[").unwrap();

        let mut registry = ThemeRegistry::new();
        let errors = registry.load_user_themes(dir.path());
        assert_eq!(errors.len(), 1);
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn extends_missing_base_errors() {
        let dir = temp_dir();
        std::fs::write(dir.path().join("orphan.toml"), r#"
extends = "nonexistent"
display_name = "Orphan"
is_dark = true
"#).unwrap();

        let mut registry = ThemeRegistry::new();
        let errors = registry.load_user_themes(dir.path());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn missing_directory_on_first_run() {
        let nonexistent = std::path::Path::new("/tmp/definitely_not_exists_theme_test");
        let mut registry = ThemeRegistry::new();
        let errors = registry.load_user_themes(nonexistent);
        assert!(errors.is_empty()); // first run creates dir, no error
    }
}
```

Requires `tempfile` as dev-dependency.

- [ ] **Step 4: Run `cargo test -p edit-plus-ui -- theme_registry`** — all load tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/theme_registry.rs crates/ui/Cargo.toml
git commit -m "feat(ui): ThemeRegistry::load_user_themes from TOML directory

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: Wire into app startup

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/Cargo.toml` (add `dirs` if needed)

- [ ] **Step 1: Compute themes directory path**

```rust
fn themes_dir() -> std::path::PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("edit+");
    path.push("themes");
    path
}
```

- [ ] **Step 2: In app startup, after ThemeRegistry::new():**

```rust
let mut registry = ThemeRegistry::new();
let dir = themes_dir();
let load_errors = registry.load_user_themes(&dir);
for err in &load_errors {
    log::warn!("theme: {}", err);  // or eprintln! if no log crate
}
```

- [ ] **Step 3: Pass registry to rebuild_theme() — it already takes registry from Plan 3**

- [ ] **Step 4: Ensure app state holds `ThemeRegistry` (add to App struct if not already done in Plan 3)**

- [ ] **Step 5: Run `cargo check -p edit-plus-app`**

---

### Task 7: Full workspace build and test

- [ ] **Step 1: Run `cargo build`**

Expected: clean build.

- [ ] **Step 2: Run `cargo test`**

Expected: all existing tests pass + new hex_color, theme_file, theme_registry tests pass.

- [ ] **Step 3: Manual smoke test — create a test TOML file**

```bash
mkdir -p ~/.config/edit+/themes/
cat > ~/.config/edit+/themes/smoke.toml << 'EOF'
display_name = "Smoke Test"
is_dark = true

[palette]
accent = "#FF0000"

[editor]
cursor = "#00FF00"
EOF
```

Launch app — should load with accent = red, cursor = green, everything else from default-dark.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: load user themes from ~/.config/edit+/themes/*.toml

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```
