use super::ColorPalette;

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
    pub(crate) fn from_palette(palette: &ColorPalette) -> Self {
        Self {
            modal_surface: palette.bg_elevated,
            sidebar_surface: palette.bg_surface,
            section_surface: palette.bg_elevated,
            section_border: palette.border_subtle,
            separator: palette.border_subtle,
            control_surface: palette.input_bg,
            control_border: palette.input_border,
            focus_ring: palette.accent,
            accent: palette.accent,
            text_primary: palette.text_main,
            text_secondary: palette.text_muted,
            text_inverse: palette.text_inverse,
        }
    }
}
