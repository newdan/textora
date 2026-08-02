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
    pub focus_ring: [f32; 4],
    pub accent: [f32; 4],
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
            focus_ring: application.accent,
            accent: application.accent,
            text_primary: application.text_primary,
            text_secondary: application.text_secondary,
            text_inverse: application.text_inverse,
        }
    }
}
