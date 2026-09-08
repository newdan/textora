use super::{ColorPalette, EditorTheme};

const MODAL_SCRIM_MINIMUM_ALPHA: f32 = 0.45;
const BUTTON_SURFACE_TEXT_BLEND: f32 = 0.035;
const BUTTON_BORDER_TEXT_BLEND: f32 = 0.12;
const BUTTON_DANGER_TEXT_BLEND: f32 = 0.6;
const BUTTON_PRESSED_TEXT_BLEND: f32 = 0.06;

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
    pub button_surface: [f32; 4],
    pub button_border: [f32; 4],
    pub button_hover_surface: [f32; 4],
    pub button_pressed_surface: [f32; 4],
    pub button_danger_foreground: [f32; 4],
    pub accent: [f32; 4],
    pub danger: [f32; 4],
    pub warning: [f32; 4],
    pub modal_scrim: [f32; 4],
}

impl ApplicationTheme {
    pub(crate) fn from_theme(palette: &ColorPalette, editor: &EditorTheme) -> Self {
        let mut modal_scrim = palette.shadow;
        modal_scrim[3] = modal_scrim[3].max(MODAL_SCRIM_MINIMUM_ALPHA);
        let button_surface =
            blend_surface(palette.bg_elevated, palette.text_main, BUTTON_SURFACE_TEXT_BLEND);
        let button_hover_surface = overlay_surface(button_surface, palette.bg_hover);

        Self {
            window_surface: palette.bg_base,
            navigation_surface: palette.bg_surface,
            content_surface: palette.bg_base,
            editor_surface: editor.background,
            overlay_surface: palette.bg_elevated,
            hover_surface: button_hover_surface,
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
            button_surface,
            button_border: blend_surface(
                palette.border_strong,
                palette.text_main,
                BUTTON_BORDER_TEXT_BLEND,
            ),
            button_hover_surface,
            button_pressed_surface: blend_surface(
                button_hover_surface,
                palette.text_main,
                BUTTON_PRESSED_TEXT_BLEND,
            ),
            button_danger_foreground: blend_surface(
                palette.danger,
                palette.text_main,
                BUTTON_DANGER_TEXT_BLEND,
            ),
            accent: palette.accent,
            danger: palette.danger,
            warning: palette.warning,
            modal_scrim,
        }
    }
}

fn blend_surface(surface: [f32; 4], foreground: [f32; 4], amount: f32) -> [f32; 4] {
    std::array::from_fn(|index| surface[index] * (1.0 - amount) + foreground[index] * amount)
}

/// 状态色是覆盖层；先在主题的线性颜色空间合成，再统一应用控件透明度。
fn overlay_surface(background: [f32; 4], overlay: [f32; 4]) -> [f32; 4] {
    let background_alpha = background[3] * (1.0 - overlay[3]);
    let alpha = overlay[3] + background_alpha;
    if alpha <= 0.0 {
        return [0.0; 4];
    }
    let mut surface = [0.0; 4];
    for index in 0..3 {
        surface[index] =
            (overlay[index] * overlay[3] + background[index] * background_alpha) / alpha;
    }
    surface[3] = alpha;
    surface
}
