use ui::ThemeMode;
use ui::theme::{ActiveThemePair, Theme, ThemeRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsPersistenceState {
    Saved,
    SaveFailed { message: String },
}

pub(crate) struct SettingsUiState {
    theme_registry: ThemeRegistry,
    active_theme_pair: ActiveThemePair,
    persistence: SettingsPersistenceState,
}

impl SettingsUiState {
    pub(crate) fn new(theme_registry: ThemeRegistry) -> Self {
        Self {
            theme_registry,
            active_theme_pair: ActiveThemePair::default(),
            persistence: SettingsPersistenceState::Saved,
        }
    }

    pub(crate) fn resolve_theme(&self, mode: ThemeMode, system: winit::window::Theme) -> Theme {
        Theme::resolve(mode, system, &self.active_theme_pair, &self.theme_registry)
    }

    pub(crate) fn persistence(&self) -> &SettingsPersistenceState {
        &self.persistence
    }

    pub(crate) fn record_persistence(&mut self, result: std::io::Result<()>) {
        self.persistence = match result {
            Ok(()) => SettingsPersistenceState::Saved,
            Err(error) => SettingsPersistenceState::SaveFailed { message: error.to_string() },
        };
    }
}
