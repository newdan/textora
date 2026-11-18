use super::ApplicationTheme;

/// Settings 界面的视觉语义令牌。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsTheme {
    pub modal_surface: [f32; 4],
    pub sidebar_surface: [f32; 4],
    pub section_surface: [f32; 4],
    pub section_border: [f32; 4],
    pub separator: [f32; 4],
    pub control_surface: [f32; 4],
    pub control_border: [f32; 4],
    pub button_surface: [f32; 4],
    pub button_border: [f32; 4],
    pub button_hover_surface: [f32; 4],
    pub button_pressed_surface: [f32; 4],
    pub button_danger_foreground: [f32; 4],
    pub hover_surface: [f32; 4],
    pub selected_surface: [f32; 4],
    pub focus_ring: [f32; 4],
    pub accent: [f32; 4],
    pub on_accent: [f32; 4],
    pub accent_text: [f32; 4],
    pub danger_text: [f32; 4],
    pub warning_text: [f32; 4],
    pub danger: [f32; 4],
    pub text_primary: [f32; 4],
    pub text_secondary: [f32; 4],
    pub text_inverse: [f32; 4],
}

impl SettingsTheme {
    pub(crate) fn from_application(application: &ApplicationTheme) -> Self {
        Self {
            modal_surface: application.overlay_surface,
            sidebar_surface: application.navigation_surface,
            section_surface: application.overlay_surface,
            section_border: application.divider,
            separator: application.divider,
            control_surface: application.control_surface,
            control_border: application.control_border,
            button_surface: application.button_surface,
            button_border: application.button_border,
            button_hover_surface: application.button_hover_surface,
            button_pressed_surface: application.button_pressed_surface,
            button_danger_foreground: application.button_danger_foreground,
            hover_surface: application.hover_surface,
            selected_surface: application.selected_surface,
            focus_ring: application.focus_ring,
            accent: application.accent,
            on_accent: application.on_accent,
            accent_text: application.accent_text,
            danger_text: application.danger_text,
            warning_text: application.warning_text,
            danger: application.danger,
            text_primary: application.text_primary,
            text_secondary: application.text_secondary,
            text_inverse: application.text_inverse,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::theme::{Theme, ThemeDefinition};

    #[test]
    fn settings_inherit_readability_roles_without_reusing_accent_or_inverse_text() {
        let theme = Theme::from_definition(&ThemeDefinition::default_light());
        let application = theme.application_theme();
        let settings = theme.settings_theme();
        assert_ne!(settings.focus_ring, settings.accent);
        assert_ne!(settings.on_accent, settings.text_inverse);
        assert_eq!(settings.focus_ring, application.focus_ring);
        assert_eq!(settings.on_accent, application.on_accent);
        assert_eq!(settings.accent_text, application.accent_text);
        assert_eq!(settings.danger_text, application.danger_text);
        assert_eq!(settings.warning_text, application.warning_text);
    }
}
