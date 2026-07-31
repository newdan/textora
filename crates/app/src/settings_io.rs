//! Persistence for product settings.toml.

use serde::{Deserialize, Serialize};
use std::path::Path;
use ui::settings::ThemeMode;
use ui::view_mode::ViewMode;

/// Default sidebar width in logical pixels (before DPI scaling).
fn default_sidebar_width() -> f32 {
    220.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedSettings {
    pub view_mode: ViewMode,
    pub theme_mode: ThemeMode,
    #[serde(default = "default_true")]
    pub show_line_numbers: bool,
    #[serde(default = "default_true")]
    pub word_wrap: bool,
    #[serde(default = "default_false")]
    pub show_status_bar: bool,
    /// Font family name (platform-dependent default, e.g. "Menlo" on macOS).
    #[serde(default = "default_font_family")]
    pub font_family: String,
    /// Logical font size in points (before DPI scaling).
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// Line height multiplier relative to font_size.
    #[serde(default = "default_line_height_ratio")]
    pub line_height_ratio: f32,
    /// Tab width in spaces.
    #[serde(default = "default_tab_width")]
    pub tab_width: usize,
    /// Sidebar width in logical pixels (before DPI scaling).
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// Window geometry in logical pixels: None means not persisted or first run.
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    /// Whether `window_width` and `window_height` are stored in logical pixels.
    #[serde(default)]
    pub window_geometry_is_logical: bool,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::default(),
            theme_mode: ThemeMode::default(),
            show_line_numbers: true,
            word_wrap: true,
            show_status_bar: false,
            font_family: default_font_family(),
            font_size: default_font_size(),
            line_height_ratio: default_line_height_ratio(),
            tab_width: default_tab_width(),
            sidebar_width: default_sidebar_width(),
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
            window_geometry_is_logical: false,
        }
    }
}

impl PersistedSettings {
    pub(crate) fn apply_editor_settings(&mut self, settings: &ui::settings::Settings) {
        self.view_mode = settings.view_mode;
        self.theme_mode = settings.theme_mode;
        self.show_line_numbers = settings.show_line_numbers;
        self.word_wrap = settings.word_wrap;
        self.show_status_bar = settings.show_status_bar;
        self.font_family = settings.font_family.clone();
        self.font_size = settings.font_size;
        self.line_height_ratio = settings.line_height_ratio;
        self.tab_width = settings.tab_width;
    }
}

fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_font_family() -> String {
    ui::settings::platform_default_font_family().to_string()
}
fn default_font_size() -> f32 {
    15.0
}
fn default_line_height_ratio() -> f32 {
    1.618
}
fn default_tab_width() -> usize {
    4
}

pub(crate) fn load(path: &Path) -> std::io::Result<PersistedSettings> {
    let toml_str = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedSettings::default());
        }
        Err(e) => return Err(e),
    };
    toml::from_str(&toml_str).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to parse settings at {}: {}", path.display(), e),
        )
    })
}

pub(crate) fn save(path: &Path, settings: &PersistedSettings) -> std::io::Result<()> {
    let toml_str = toml::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    crate::persistence::atomic_write(path, toml_str.as_bytes())
}

pub(crate) fn save_editor_settings(
    path: &Path,
    settings: &ui::settings::Settings,
) -> std::io::Result<()> {
    let mut persisted = load(path)?;
    persisted.apply_editor_settings(settings);
    save(path, &persisted)
}

pub(crate) fn ensure_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() { Ok(()) } else { save(path, &PersistedSettings::default()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_mode_is_sidebar() {
        assert_eq!(PersistedSettings::default().view_mode, ViewMode::Sidebar);
    }

    #[test]
    fn persisted_settings_roundtrip() {
        let s = PersistedSettings { view_mode: ViewMode::Sidebar, ..Default::default() };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let parsed: PersistedSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.view_mode, ViewMode::Sidebar);
    }

    #[test]
    fn settings_save_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        // Passing a directory instead of a file should return an error
        let result = save(dir.path(), &PersistedSettings::default());
        assert!(result.is_err());
    }

    #[test]
    fn settings_load_propagates_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.toml");
        std::fs::write(&path, b"invalid = toml = format").unwrap();

        let result = load(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn settings_load_uses_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.view_mode, ViewMode::Sidebar);
    }

    #[test]
    fn settings_save_and_load_roundtrip_with_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let expected = PersistedSettings {
            view_mode: ViewMode::Tabs,
            theme_mode: ThemeMode::Dark,
            show_line_numbers: false,
            word_wrap: false,
            show_status_bar: true,
            font_family: "Test Font".to_owned(),
            font_size: 17.0,
            line_height_ratio: 1.5,
            tab_width: 2,
            sidebar_width: 250.0,
            window_x: Some(10),
            window_y: Some(20),
            window_width: Some(1000),
            window_height: Some(700),
            window_geometry_is_logical: true,
        };
        save(&path, &expected).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.view_mode, expected.view_mode);
        assert_eq!(loaded.theme_mode, expected.theme_mode);
        assert_eq!(loaded.show_line_numbers, expected.show_line_numbers);
        assert_eq!(loaded.word_wrap, expected.word_wrap);
        assert_eq!(loaded.show_status_bar, expected.show_status_bar);
        assert_eq!(loaded.font_family, expected.font_family);
        assert_eq!(loaded.font_size, expected.font_size);
        assert_eq!(loaded.line_height_ratio, expected.line_height_ratio);
        assert_eq!(loaded.tab_width, expected.tab_width);
        assert_eq!(loaded.sidebar_width, expected.sidebar_width);
        assert_eq!(loaded.window_x, expected.window_x);
        assert_eq!(loaded.window_y, expected.window_y);
        assert_eq!(loaded.window_width, expected.window_width);
        assert_eq!(loaded.window_height, expected.window_height);
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let toml_str = "";
        let parsed: PersistedSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.view_mode, ViewMode::Sidebar);
        assert_eq!(parsed.window_x, None);
        assert_eq!(parsed.window_y, None);
        assert_eq!(parsed.window_width, None);
        assert_eq!(parsed.window_height, None);
    }

    #[test]
    fn window_geometry_roundtrip() {
        let s = PersistedSettings {
            view_mode: ViewMode::Sidebar,
            window_x: Some(100),
            window_y: Some(200),
            window_width: Some(1200),
            window_height: Some(800),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let parsed: PersistedSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.window_x, Some(100));
        assert_eq!(parsed.window_y, Some(200));
        assert_eq!(parsed.window_width, Some(1200));
        assert_eq!(parsed.window_height, Some(800));
    }

    #[test]
    fn theme_mode_roundtrip() {
        let s = PersistedSettings { theme_mode: ThemeMode::Dark, ..Default::default() };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let parsed: PersistedSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.theme_mode, ThemeMode::Dark);
    }

    #[test]
    fn theme_mode_default_is_system() {
        let s = PersistedSettings::default();
        assert_eq!(s.theme_mode, ThemeMode::System);
    }

    #[test]
    fn theme_mode_legacy_claude_dark_deserializes_as_dark() {
        // Old config files may contain "ClaudeDark" — should deserialize as Dark
        let toml_str = "theme_mode = \"ClaudeDark\"\n";
        let parsed: PersistedSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.theme_mode, ThemeMode::Dark);
    }

    #[test]
    fn theme_mode_legacy_claude_light_deserializes_as_light() {
        // Old config files may contain "ClaudeLight" — should deserialize as Light
        let toml_str = "theme_mode = \"ClaudeLight\"\n";
        let parsed: PersistedSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.theme_mode, ThemeMode::Light);
    }

    #[test]
    fn show_line_numbers_default_true() {
        let s = PersistedSettings::default();
        assert!(s.show_line_numbers);
    }

    #[test]
    fn word_wrap_default_true() {
        let s = PersistedSettings::default();
        assert!(s.word_wrap);
    }

    #[test]
    fn toggle_settings_roundtrip() {
        let s = PersistedSettings {
            show_line_numbers: false,
            word_wrap: false,
            show_status_bar: true,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let parsed: PersistedSettings = toml::from_str(&toml_str).unwrap();
        assert!(!parsed.show_line_numbers);
        assert!(!parsed.word_wrap);
        assert!(parsed.show_status_bar);
    }

    #[test]
    fn show_status_bar_default_false() {
        let s = PersistedSettings::default();
        assert!(!s.show_status_bar);
    }

    #[test]
    fn show_status_bar_missing_field_defaults_false() {
        let toml_str = "view_mode = \"sidebar\"\n";
        let parsed: PersistedSettings = toml::from_str(toml_str).unwrap();
        assert!(!parsed.show_status_bar);
    }

    #[test]
    fn window_geometry_all_none_default() {
        let s = PersistedSettings::default();
        assert_eq!(s.window_x, None);
        assert_eq!(s.window_y, None);
        assert_eq!(s.window_width, None);
        assert_eq!(s.window_height, None);
    }

    #[test]
    fn font_settings_defaults() {
        let s = PersistedSettings::default();
        assert_eq!(s.font_family, ui::settings::platform_default_font_family());
        assert_eq!(s.font_size, 15.0);
        assert_eq!(s.line_height_ratio, 1.618);
        assert_eq!(s.tab_width, 4);
    }

    #[test]
    fn font_settings_roundtrip() {
        let s = PersistedSettings {
            font_family: "Menlo".to_string(),
            font_size: 18.0,
            line_height_ratio: 1.5,
            tab_width: 2,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let parsed: PersistedSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.font_family, "Menlo");
        assert_eq!(parsed.font_size, 18.0);
        assert_eq!(parsed.line_height_ratio, 1.5);
        assert_eq!(parsed.tab_width, 2);
    }

    #[test]
    fn missing_font_fields_use_defaults() {
        let toml_str = "view_mode = \"sidebar\"\n";
        let parsed: PersistedSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.font_family, ui::settings::platform_default_font_family());
        assert_eq!(parsed.font_size, 15.0);
        assert_eq!(parsed.line_height_ratio, 1.618);
        assert_eq!(parsed.tab_width, 4);
    }

    #[test]
    fn sidebar_width_default() {
        let s = PersistedSettings::default();
        assert_eq!(s.sidebar_width, 220.0);
    }

    #[test]
    fn sidebar_width_roundtrip() {
        let s = PersistedSettings { sidebar_width: 300.0, ..Default::default() };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let parsed: PersistedSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.sidebar_width, 300.0);
    }

    #[test]
    fn missing_sidebar_width_uses_default() {
        let toml_str = "view_mode = \"sidebar\"\n";
        let parsed: PersistedSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.sidebar_width, 220.0);
    }

    #[test]
    fn physical_sidebar_width_roundtrips_as_logical_value() {
        let mut app = crate::App::new(None);
        app.update_scale_factor(2.0);
        app.ui_shell.sidebar_cfg_mut().width = 440.0;
        let persisted = PersistedSettings {
            sidebar_width: app.sidebar_width_for_persistence(),
            ..PersistedSettings::default()
        };

        let encoded = toml::to_string(&persisted).unwrap();
        let decoded: PersistedSettings = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.sidebar_width, 220.0);
    }

    #[test]
    fn apply_editor_settings_updates_editor_fields_only() {
        let mut persisted = PersistedSettings {
            sidebar_width: 333.0,
            window_x: Some(10),
            window_y: Some(20),
            window_width: Some(900),
            window_height: Some(700),
            ..PersistedSettings::default()
        };
        let mut settings = ui::settings::Settings::new();
        settings.view_mode = ViewMode::Tabs;
        settings.theme_mode = ThemeMode::Dark;
        settings.show_line_numbers = false;
        settings.word_wrap = false;
        settings.show_status_bar = true;
        settings.font_family = "Test Mono".into();
        settings.font_size = 19.0;
        settings.line_height_ratio = 1.5;
        settings.line_height = 28.5;
        settings.tab_width = 8;

        persisted.apply_editor_settings(&settings);

        assert_eq!(persisted.view_mode, ViewMode::Tabs);
        assert_eq!(persisted.theme_mode, ThemeMode::Dark);
        assert!(!persisted.show_line_numbers);
        assert!(!persisted.word_wrap);
        assert!(persisted.show_status_bar);
        assert_eq!(persisted.font_family, "Test Mono");
        assert_eq!(persisted.font_size, 19.0);
        assert_eq!(persisted.line_height_ratio, 1.5);
        assert_eq!(persisted.tab_width, 8);
        assert_eq!(persisted.sidebar_width, 333.0);
        assert_eq!(persisted.window_x, Some(10));
        assert_eq!(persisted.window_y, Some(20));
        assert_eq!(persisted.window_width, Some(900));
        assert_eq!(persisted.window_height, Some(700));
    }
}
