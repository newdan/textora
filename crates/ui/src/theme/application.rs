use super::{ColorPalette, EditorTheme};

const MODAL_SCRIM_MINIMUM_ALPHA: f32 = 0.45;

/// 应用外壳的通用视觉语义令牌。
///
/// 产品层只选择视觉角色，不直接解释底层色板。所有颜色均来自已经完成 gamma
/// correction 的 [`super::Theme`]。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ApplicationTheme {
    pub window_surface: [f32; 4],
    pub navigation_surface: [f32; 4],
    pub content_surface: [f32; 4],
    pub editor_surface: [f32; 4],
    pub overlay_surface: [f32; 4],
    pub hover_surface: [f32; 4],
    pub selected_surface: [f32; 4],
    pub navigation_hover_surface: [f32; 4],
    pub navigation_selected_surface: [f32; 4],
    pub navigation_selected_text: [f32; 4],
    pub text_primary: [f32; 4],
    pub text_secondary: [f32; 4],
    pub text_inverse: [f32; 4],
    pub divider: [f32; 4],
    pub strong_border: [f32; 4],
    pub control_surface: [f32; 4],
    pub control_border: [f32; 4],
    pub accent: [f32; 4],
    pub danger: [f32; 4],
    pub warning: [f32; 4],
    pub modal_scrim: [f32; 4],
}

impl ApplicationTheme {
    pub(crate) fn from_theme(palette: &ColorPalette, editor: &EditorTheme) -> Self {
        let mut modal_scrim = palette.shadow;
        modal_scrim[3] = modal_scrim[3].max(MODAL_SCRIM_MINIMUM_ALPHA);

        Self {
            window_surface: palette.bg_base,
            navigation_surface: palette.bg_surface,
            content_surface: palette.bg_base,
            editor_surface: editor.background,
            overlay_surface: palette.bg_elevated,
            hover_surface: palette.bg_hover,
            selected_surface: palette.bg_active,
            navigation_hover_surface: palette.sidebar_hover_bg,
            navigation_selected_surface: palette.sidebar_active_bg,
            navigation_selected_text: palette.sidebar_active_fg,
            text_primary: palette.text_main,
            text_secondary: palette.text_muted,
            text_inverse: palette.text_inverse,
            divider: palette.border_subtle,
            strong_border: palette.border_strong,
            control_surface: palette.input_bg,
            control_border: palette.input_border,
            accent: palette.accent,
            danger: palette.danger,
            warning: palette.warning,
            modal_scrim,
        }
    }
}
