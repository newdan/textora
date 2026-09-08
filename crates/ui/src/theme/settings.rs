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
            focus_ring: application.accent,
            accent: application.accent,
            danger: application.danger,
            text_primary: application.text_primary,
            text_secondary: application.text_secondary,
            text_inverse: application.text_inverse,
        }
    }
}
