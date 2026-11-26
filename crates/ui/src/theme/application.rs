use super::color::SelectedControlColors;
use super::{ColorPalette, EditorTheme};

const MODAL_SCRIM_MINIMUM_ALPHA: f32 = 0.45;
const BUTTON_SURFACE_TEXT_BLEND: f32 = 0.035;
const DARK_BUTTON_SURFACE_TEXT_BLEND: f32 = 0.015;
const BUTTON_BORDER_TEXT_BLEND: f32 = 0.12;
const BUTTON_DANGER_TEXT_BLEND: f32 = 0.6;
const BUTTON_PRESSED_TEXT_BLEND: f32 = 0.06;
const DARK_BUTTON_PRESSED_TEXT_BLEND: f32 = 0.015;

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
    pub on_accent: [f32; 4],
    pub(crate) selected_control: SelectedControlColors,
    pub accent_text: [f32; 4],
    pub danger: [f32; 4],
    pub danger_text: [f32; 4],
    pub warning: [f32; 4],
    pub warning_text: [f32; 4],
    pub focus_ring: [f32; 4],
    pub modal_scrim: [f32; 4],
}

impl ApplicationTheme {
    pub(crate) fn from_theme(palette: &ColorPalette, editor: &EditorTheme, is_dark: bool) -> Self {
        let mut modal_scrim = palette.shadow;
        modal_scrim[3] = modal_scrim[3].max(MODAL_SCRIM_MINIMUM_ALPHA);
        let (button_base, surface_text_blend, pressed_text_blend) = if is_dark {
            (palette.input_bg, DARK_BUTTON_SURFACE_TEXT_BLEND, DARK_BUTTON_PRESSED_TEXT_BLEND)
        } else {
            (palette.bg_elevated, BUTTON_SURFACE_TEXT_BLEND, BUTTON_PRESSED_TEXT_BLEND)
        };
        let button_surface = blend_surface(button_base, palette.text_main, surface_text_blend);
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
                pressed_text_blend,
            ),
            button_danger_foreground: blend_surface(
                palette.danger,
                palette.text_main,
                BUTTON_DANGER_TEXT_BLEND,
            ),
            accent: palette.accent,
            on_accent: palette.on_accent,
            selected_control: palette.selected_control.unwrap_or(SelectedControlColors {
                background: palette.accent,
                foreground: palette.on_accent,
            }),
            accent_text: palette.accent_text,
            danger: palette.danger,
            danger_text: palette.danger_text,
            warning: palette.warning,
            warning_text: palette.warning_text,
            focus_ring: palette.focus_ring,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Theme, ThemeDefinition};

    const MINIMUM_TEXT_CONTRAST: f64 = 4.5;
    const MINIMUM_CONTROL_CONTRAST: f64 = 3.0;

    fn srgb_relative_luminance(color: [f32; 4]) -> f64 {
        const SRGB_LINEAR_THRESHOLD: f64 = 0.04045;
        const SRGB_LINEAR_DIVISOR: f64 = 12.92;
        const SRGB_OFFSET: f64 = 0.055;
        const SRGB_SCALE: f64 = 1.055;
        const SRGB_EXPONENT: f64 = 2.4;
        const LUMINANCE_WEIGHTS: [f64; 3] = [0.2126, 0.7152, 0.0722];
        color[..3]
            .iter()
            .zip(LUMINANCE_WEIGHTS)
            .map(|(&channel, weight)| {
                let channel = f64::from(channel);
                let linear = if channel <= SRGB_LINEAR_THRESHOLD {
                    channel / SRGB_LINEAR_DIVISOR
                } else {
                    ((channel + SRGB_OFFSET) / SRGB_SCALE).powf(SRGB_EXPONENT)
                };
                linear * weight
            })
            .sum()
    }

    fn contrast_ratio(foreground: [f32; 4], background: [f32; 4]) -> f64 {
        const CONTRAST_LUMINANCE_OFFSET: f64 = 0.05;
        let foreground = srgb_relative_luminance(foreground);
        let background = srgb_relative_luminance(background);
        (foreground.max(background) + CONTRAST_LUMINANCE_OFFSET)
            / (foreground.min(background) + CONTRAST_LUMINANCE_OFFSET)
    }

    fn paper_surfaces(palette: &ColorPalette) -> [(&'static str, [f32; 4]); 4] {
        [
            ("window", palette.bg_base),
            ("navigation", palette.bg_surface),
            ("overlay", palette.bg_elevated),
            ("input", palette.input_bg),
        ]
    }

    #[test]
    fn light_input_boundary_is_visible_against_every_paper_surface() {
        let definition = ThemeDefinition::default_light();
        let insufficient_contrasts: Vec<_> = paper_surfaces(&definition.palette)
            .map(|(role, surface)| (role, contrast_ratio(definition.palette.input_border, surface)))
            .into_iter()
            .filter(|(_, contrast)| *contrast < MINIMUM_CONTROL_CONTRAST)
            .collect();
        assert!(
            insufficient_contrasts.is_empty(),
            "input boundary needs at least {MINIMUM_CONTROL_CONTRAST}:1: {insufficient_contrasts:?}"
        );
    }

    #[test]
    fn light_line_numbers_remain_readable_on_the_editor_canvas() {
        let definition = ThemeDefinition::default_light();
        for canvas in [definition.editor.background, definition.editor.gutter_bg] {
            let contrast = contrast_ratio(definition.editor.line_number, canvas);
            assert!(
                contrast >= MINIMUM_TEXT_CONTRAST,
                "line numbers need at least {MINIMUM_TEXT_CONTRAST}:1, got {contrast:.9}:1"
            );
        }
    }

    #[test]
    fn semantic_foregrounds_keep_their_srgb_definition_values() {
        for (definition, expected_colors) in [
            (
                ThemeDefinition::default_light(),
                ["#2a1808", "#9a4815", "#b3261e", "#8a5a00", "#c2410c"],
            ),
            (
                ThemeDefinition::default_dark(),
                ["#2a1504", "#ffc080", "#f08a7f", "#e5ca9a", "#f59a4a"],
            ),
        ] {
            for (actual, expected_hex) in
                palette_foregrounds(&definition.palette).into_iter().zip(expected_colors)
            {
                let expected = crate::hex_color::parse_hex(expected_hex)
                    .expect("semantic foreground test colors are valid six-digit sRGB hex");
                assert_eq!(actual, expected, "{}: {expected_hex}", definition.display_name);
            }
        }
    }

    fn palette_foregrounds(palette: &ColorPalette) -> [[f32; 4]; 5] {
        [
            palette.on_accent,
            palette.accent_text,
            palette.danger_text,
            palette.warning_text,
            palette.focus_ring,
        ]
    }

    #[test]
    fn application_foregrounds_preserve_gamma_corrected_roles_and_alpha() {
        const DISPLAY_GAMMA: f32 = 2.2;
        let mut definition = ThemeDefinition::default_light();
        definition.palette.on_accent = [0.1, 0.2, 0.3, 0.4];
        definition.palette.accent_text = [0.2, 0.3, 0.4, 0.5];
        definition.palette.danger_text = [0.3, 0.4, 0.5, 0.6];
        definition.palette.warning_text = [0.4, 0.5, 0.6, 0.7];
        definition.palette.focus_ring = [0.5, 0.6, 0.7, 0.8];
        let source_colors = palette_foregrounds(&definition.palette);
        let theme = Theme::from_definition(&definition);
        let application = theme.application_theme();
        let application_colors = [
            application.on_accent,
            application.accent_text,
            application.danger_text,
            application.warning_text,
            application.focus_ring,
        ];
        for ((source, palette), application) in source_colors
            .into_iter()
            .zip(palette_foregrounds(&theme.palette))
            .zip(application_colors)
        {
            let expected = [
                source[0].powf(DISPLAY_GAMMA),
                source[1].powf(DISPLAY_GAMMA),
                source[2].powf(DISPLAY_GAMMA),
                source[3],
            ];
            assert_eq!(palette, expected);
            assert_eq!(application, expected);
        }
        assert_eq!(palette_foregrounds(&definition.palette), source_colors);
    }

    #[test]
    fn builtin_semantic_foregrounds_are_readable_in_their_intended_states() {
        for definition in [ThemeDefinition::default_light(), ThemeDefinition::default_dark()] {
            let palette = &definition.palette;
            assert_minimum_contrast(
                "selected control label",
                palette.on_accent,
                palette.accent,
                MINIMUM_TEXT_CONTRAST,
            );
            for (surface_role, surface) in paper_surfaces(palette).into_iter().chain([
                ("navigation hover", palette.sidebar_hover_bg),
                ("navigation selection", palette.sidebar_active_bg),
                ("editor", definition.editor.background),
            ]) {
                for foreground in [palette.accent_text, palette.danger_text, palette.warning_text] {
                    assert_minimum_contrast(
                        surface_role,
                        foreground,
                        surface,
                        MINIMUM_TEXT_CONTRAST,
                    );
                }
                assert_minimum_contrast(
                    surface_role,
                    palette.focus_ring,
                    surface,
                    MINIMUM_CONTROL_CONTRAST,
                );
            }
            assert_minimum_contrast(
                "line number",
                definition.editor.line_number,
                definition.editor.gutter_bg,
                MINIMUM_TEXT_CONTRAST,
            );
        }
    }

    fn assert_minimum_contrast(
        role: &str,
        foreground: [f32; 4],
        background: [f32; 4],
        minimum: f64,
    ) {
        let contrast = contrast_ratio(foreground, background);
        assert!(contrast >= minimum, "{role} needs {minimum}:1, got {contrast:.9}:1");
    }
}
