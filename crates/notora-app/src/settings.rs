use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

const SETTINGS_SCHEMA_VERSION: u32 = 1;
const MINIMUM_FONT_SIZE: f32 = 6.0;
const MAXIMUM_FONT_SIZE: f32 = 72.0;
const MINIMUM_LINE_HEIGHT_RATIO: f32 = 1.0;
const MAXIMUM_LINE_HEIGHT_RATIO: f32 = 3.0;
const MINIMUM_TAB_WIDTH: usize = 1;
const MAXIMUM_TAB_WIDTH: usize = 16;
const MINIMUM_RUNTIME_TAB_LIMIT: usize = 1;
const MAXIMUM_RUNTIME_TAB_LIMIT: usize = 128;
const MINIMUM_AUTO_SAVE_DELAY_MILLIS: u64 = 100;
const MAXIMUM_AUTO_SAVE_DELAY_MILLIS: u64 = 60_000;
const MINIMUM_CATALOG_BACKUP_RETENTION: usize = 1;
const MAXIMUM_CATALOG_BACKUP_RETENTION: usize = 100;
static SETTINGS_TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// notora 的持久化设置；不包含其他 textora 产品的兼容状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProductSettings {
    pub schema_version: u32,
    pub appearance: AppearanceSettings,
    pub editor: EditorSettings,
    pub interface: InterfaceSettings,
    pub workspace: WorkspaceSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceSettings {
    pub theme_mode: ui::ThemeMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorSettings {
    pub font_family: String,
    pub font_size: f32,
    pub line_height_ratio: f32,
    pub tab_width: usize,
    pub word_wrap: bool,
    pub markdown_first_line_indent: bool,
    pub show_line_numbers: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InterfaceSettings {
    pub show_status_bar: bool,
    pub runtime_tab_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceSettings {
    pub auto_save_delay_millis: u64,
    pub catalog_backup_retention: usize,
}

impl Default for ProductSettings {
    fn default() -> Self {
        let ui_settings = ui::Settings::new();
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            appearance: AppearanceSettings { theme_mode: ui_settings.theme_mode },
            editor: EditorSettings {
                font_family: ui_settings.font_family,
                font_size: ui_settings.font_size,
                line_height_ratio: ui_settings.line_height_ratio,
                tab_width: ui_settings.tab_width,
                word_wrap: ui_settings.word_wrap,
                markdown_first_line_indent: ui_settings.markdown_first_line_indent,
                show_line_numbers: ui_settings.show_line_numbers,
            },
            interface: InterfaceSettings {
                show_status_bar: ui_settings.show_status_bar,
                runtime_tab_limit: 12,
            },
            workspace: WorkspaceSettings {
                auto_save_delay_millis: 800,
                catalog_backup_retention: 8,
            },
        }
    }
}

impl Default for EditorSettings {
    fn default() -> Self {
        ProductSettings::default().editor
    }
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        ProductSettings::default().interface
    }
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        ProductSettings::default().workspace
    }
}

impl ProductSettings {
    pub fn apply_to_ui(&self, ui_settings: &mut ui::Settings) {
        ui_settings.set_theme_mode(self.appearance.theme_mode);
        ui_settings.set_font_family(self.editor.font_family.clone());
        ui_settings.set_font_size(self.editor.font_size);
        ui_settings.set_line_height_ratio(self.editor.line_height_ratio);
        ui_settings.set_tab_width(self.editor.tab_width);
        ui_settings.set_word_wrap(self.editor.word_wrap);
        ui_settings.set_markdown_first_line_indent(self.editor.markdown_first_line_indent);
        ui_settings.set_show_line_numbers(self.editor.show_line_numbers);
        ui_settings.set_show_status_bar(self.interface.show_status_bar);
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.editor.font_family.trim().is_empty() {
            return Err("font family must not be empty");
        }
        if !float_is_in_range(self.editor.font_size, MINIMUM_FONT_SIZE, MAXIMUM_FONT_SIZE) {
            return Err("font size must be between 6 and 72");
        }
        if !float_is_in_range(
            self.editor.line_height_ratio,
            MINIMUM_LINE_HEIGHT_RATIO,
            MAXIMUM_LINE_HEIGHT_RATIO,
        ) {
            return Err("line height ratio must be between 1 and 3");
        }
        if !(MINIMUM_TAB_WIDTH..=MAXIMUM_TAB_WIDTH).contains(&self.editor.tab_width) {
            return Err("tab width must be between 1 and 16");
        }
        if !(MINIMUM_RUNTIME_TAB_LIMIT..=MAXIMUM_RUNTIME_TAB_LIMIT)
            .contains(&self.interface.runtime_tab_limit)
        {
            return Err("runtime tab limit must be between 1 and 128");
        }
        if !(MINIMUM_AUTO_SAVE_DELAY_MILLIS..=MAXIMUM_AUTO_SAVE_DELAY_MILLIS)
            .contains(&self.workspace.auto_save_delay_millis)
        {
            return Err("auto-save delay must be between 100 and 60000 milliseconds");
        }
        if !(MINIMUM_CATALOG_BACKUP_RETENTION..=MAXIMUM_CATALOG_BACKUP_RETENTION)
            .contains(&self.workspace.catalog_backup_retention)
        {
            return Err("catalog backup retention must be between 1 and 100");
        }
        Ok(())
    }
}

fn float_is_in_range(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
}

/// 配置读取失败会安全回退，但保留可展示的诊断文本。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedProductSettings {
    pub settings: ProductSettings,
    pub diagnostic: Option<String>,
}

pub fn load_product_settings(path: &Path) -> LoadedProductSettings {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedProductSettings {
                settings: ProductSettings::default(),
                diagnostic: None,
            };
        }
        Err(error) => {
            return LoadedProductSettings {
                settings: ProductSettings::default(),
                diagnostic: Some(format!("could not read settings: {error}")),
            };
        }
    };
    match toml::from_str::<ProductSettings>(&contents) {
        Ok(settings) if settings.schema_version == SETTINGS_SCHEMA_VERSION => {
            match settings.validate() {
                Ok(()) => LoadedProductSettings { settings, diagnostic: None },
                Err(message) => LoadedProductSettings {
                    settings: ProductSettings::default(),
                    diagnostic: Some(format!("invalid product settings: {message}")),
                },
            }
        }
        Ok(settings) => LoadedProductSettings {
            settings: ProductSettings::default(),
            diagnostic: Some(format!(
                "unsupported settings schema version: {}",
                settings.schema_version
            )),
        },
        Err(error) => LoadedProductSettings {
            settings: ProductSettings::default(),
            diagnostic: Some(format!("could not parse settings: {error}")),
        },
    }
}

pub fn save_product_settings(path: &Path, settings: &ProductSettings) -> Result<(), SettingsError> {
    let contents = toml::to_string_pretty(settings).map_err(SettingsError::Serialize)?;
    let parent =
        path.parent().ok_or_else(|| SettingsError::MissingParent { path: path.to_path_buf() })?;
    fs::create_dir_all(parent)
        .map_err(|source| SettingsError::Io { path: parent.to_path_buf(), source })?;
    let temporary_path = parent.join(format!(
        ".settings.{}.{}.tmp",
        std::process::id(),
        SETTINGS_TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut temporary_guard = TemporarySettingsPath::new(temporary_path.clone());
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|source| SettingsError::Io { path: temporary_path.clone(), source })?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| SettingsError::Io { path: temporary_path.clone(), source })?;
    fs::rename(&temporary_path, path)
        .map_err(|source| SettingsError::Io { path: path.to_path_buf(), source })?;
    temporary_guard.keep();
    Ok(())
}

#[derive(Debug)]
pub enum SettingsError {
    MissingParent { path: PathBuf },
    Serialize(toml::ser::Error),
    Io { path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingParent { path } => {
                write!(formatter, "settings path has no parent: {}", path.display())
            }
            Self::Serialize(source) => write!(formatter, "could not serialize settings: {source}"),
            Self::Io { path, source } => {
                write!(formatter, "settings I/O failed for {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::MissingParent { .. } => None,
        }
    }
}

struct TemporarySettingsPath {
    path: PathBuf,
    should_remove: bool,
}

impl TemporarySettingsPath {
    fn new(path: PathBuf) -> Self {
        Self { path, should_remove: true }
    }

    fn keep(&mut self) {
        self.should_remove = false;
    }
}

impl Drop for TemporarySettingsPath {
    fn drop(&mut self) {
        if self.should_remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProductSettings, load_product_settings, save_product_settings};

    #[test]
    fn settings_round_trip_and_map_to_ui_settings() {
        let directory = tempfile::tempdir().expect("settings test directory should exist");
        let path = directory.path().join("settings.toml");
        let mut settings = ProductSettings::default();
        settings.editor.font_size = 19.0;
        settings.interface.runtime_tab_limit = 6;
        save_product_settings(&path, &settings).expect("settings should save atomically");

        let loaded = load_product_settings(&path);
        assert_eq!(loaded.diagnostic, None);
        assert_eq!(loaded.settings, settings);
        let mut ui_settings = ui::Settings::new();
        loaded.settings.apply_to_ui(&mut ui_settings);
        assert_eq!(ui_settings.font_size, 19.0);
    }

    #[test]
    fn malformed_or_unknown_settings_fall_back_with_a_diagnostic() {
        let directory = tempfile::tempdir().expect("settings test directory should exist");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "unknown = true").expect("fixture should write");

        let loaded = load_product_settings(&path);
        assert_eq!(loaded.settings, ProductSettings::default());
        assert!(loaded.diagnostic.is_some());
    }

    #[test]
    fn invalid_operational_settings_fall_back_to_safe_defaults() {
        let directory = tempfile::tempdir().expect("settings test directory should exist");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "schema_version = 1\n[interface]\nruntime_tab_limit = 0\n")
            .expect("fixture should write");

        let loaded = load_product_settings(&path);
        assert_eq!(loaded.settings, ProductSettings::default());
        assert!(loaded.diagnostic.as_deref().is_some_and(|message| message.contains("tab limit")));
    }

    #[test]
    fn invalid_editor_settings_fall_back_before_reaching_the_ui_runtime() {
        let directory = tempfile::tempdir().expect("settings test directory should exist");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "schema_version = 1\n[editor]\nfont_size = 100\n")
            .expect("fixture should write");

        let loaded = load_product_settings(&path);
        assert_eq!(loaded.settings, ProductSettings::default());
        assert!(loaded.diagnostic.as_deref().is_some_and(|message| message.contains("font size")));
    }
}
