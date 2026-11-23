use super::ApplicationTheme;

/// Settings 界面的视觉语义令牌。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsTheme {
    pub application: ApplicationTheme,
    pub modal_surface: [f32; 4],
    pub sidebar_surface: [f32; 4],
    pub section_surface: [f32; 4],
    pub section_border: [f32; 4],
    pub separator: [f32; 4],
}

impl SettingsTheme {
    pub(crate) fn from_application(application: &ApplicationTheme) -> Self {
        Self {
            application: *application,
            modal_surface: application.overlay_surface,
            sidebar_surface: application.navigation_surface,
            section_surface: application.overlay_surface,
            section_border: application.divider,
            separator: application.divider,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::theme::{Theme, ThemeDefinition};

    #[test]
    fn settings_inherit_readability_roles_without_reusing_accent_or_inverse_text() {
        let light = Theme::from_definition(&ThemeDefinition::default_light());
        let dark = Theme::from_definition(&ThemeDefinition::default_dark());
        assert_ne!(light.application_theme().focus_ring, light.application_theme().accent);
        assert_ne!(light.application_theme().on_accent, light.application_theme().text_inverse);

        for theme in [light, dark] {
            let settings = theme.settings_theme();
            let shared = settings.application;
            assert_eq!(shared.focus_ring, theme.palette.focus_ring);
            assert_eq!(shared.on_accent, theme.palette.on_accent);
            assert_eq!(shared.text_inverse, theme.palette.text_inverse);
            assert_eq!(shared.accent_text, theme.palette.accent_text);
            assert_eq!(shared.danger_text, theme.palette.danger_text);
            assert_eq!(shared.warning_text, theme.palette.warning_text);
        }
    }
}
