use crate::core::{PaintCtx, Rect};
use crate::theme::{ControlMetrics, SettingsTheme, Theme};

const BUTTON_DISABLED_ALPHA: f32 = 0.45;
const DESTRUCTIVE_SURFACE_BLEND: f32 = 0.08;
const DESTRUCTIVE_HOVER_BLEND: f32 = 0.16;
const DESTRUCTIVE_PRESSED_BLEND: f32 = 0.24;
const DESTRUCTIVE_BORDER_BLEND: f32 = 0.5;
const TRANSPARENT: [f32; 4] = [0.0; 4];
const CATEGORY_HOVER_ACCENT_BLEND: f32 = 0.05;
const CATEGORY_PRESSED_ACCENT_BLEND: f32 = 0.09;
const CATEGORY_SELECTED_ACCENT_BLEND: f32 = 0.14;
const SEGMENT_HOVER_ACCENT_BLEND: f32 = 0.16;
const SEGMENT_PRESSED_ACCENT_BLEND: f32 = 0.14;

/// 互斥的按钮视觉状态；选中状态始终保持前景与背景成对使用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVisualState {
    #[default]
    Normal,
    Hovered,
    Pressed,
    Selected,
    Disabled,
}

/// Visual style for a Button.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonStyle {
    pub font_size_logical: f32,
    pub pad_x_logical: f32,
    pub foreground: [f32; 4],
    pub selected_foreground: [f32; 4],
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub hover_background: [f32; 4],
    pub pressed_background: [f32; 4],
    pub selected_background: [f32; 4],
    pub disabled_foreground: [f32; 4],
    pub disabled_background: [f32; 4],
    pub corner_radius_logical: f32,
}

impl ButtonStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        Self::action(theme.settings_theme())
    }

    pub fn action(settings: SettingsTheme) -> Self {
        let metrics = ControlMetrics::default();
        Self {
            font_size_logical: metrics.font_size_logical,
            pad_x_logical: metrics.horizontal_padding_logical,
            foreground: settings.text_primary,
            selected_foreground: settings.text_primary,
            background: settings.button_surface,
            border: settings.button_border,
            hover_background: settings.button_hover_surface,
            pressed_background: settings.button_pressed_surface,
            selected_background: settings.button_pressed_surface,
            disabled_foreground: with_alpha(settings.text_primary, BUTTON_DISABLED_ALPHA),
            disabled_background: with_alpha(settings.button_surface, BUTTON_DISABLED_ALPHA),
            corner_radius_logical: metrics.corner_radius_logical,
        }
    }

    /// 不可逆操作沿用普通按钮尺寸，以危险色区分文字、图标和轮廓。
    pub fn destructive(settings: SettingsTheme) -> Self {
        let background = blend(settings.button_surface, settings.danger, DESTRUCTIVE_SURFACE_BLEND);
        let pressed_background =
            blend(settings.button_surface, settings.danger, DESTRUCTIVE_PRESSED_BLEND);
        Self {
            foreground: settings.button_danger_foreground,
            selected_foreground: settings.button_danger_foreground,
            background,
            border: blend(settings.button_border, settings.danger, DESTRUCTIVE_BORDER_BLEND),
            hover_background: blend(
                settings.button_surface,
                settings.danger,
                DESTRUCTIVE_HOVER_BLEND,
            ),
            pressed_background,
            selected_background: pressed_background,
            disabled_foreground: with_alpha(
                settings.button_danger_foreground,
                BUTTON_DISABLED_ALPHA,
            ),
            disabled_background: with_alpha(background, BUTTON_DISABLED_ALPHA),
            ..Self::action(settings)
        }
    }

    pub fn category(settings: SettingsTheme) -> Self {
        Self {
            selected_foreground: settings.accent,
            background: TRANSPARENT,
            border: TRANSPARENT,
            hover_background: blend(
                settings.sidebar_surface,
                settings.accent,
                CATEGORY_HOVER_ACCENT_BLEND,
            ),
            pressed_background: blend(
                settings.sidebar_surface,
                settings.accent,
                CATEGORY_PRESSED_ACCENT_BLEND,
            ),
            selected_background: blend(
                settings.sidebar_surface,
                settings.accent,
                CATEGORY_SELECTED_ACCENT_BLEND,
            ),
            disabled_background: TRANSPARENT,
            ..Self::action(settings)
        }
    }

    pub fn segmented(settings: SettingsTheme) -> Self {
        Self {
            selected_foreground: settings.text_inverse,
            background: TRANSPARENT,
            border: TRANSPARENT,
            hover_background: blend(
                settings.modal_surface,
                settings.accent,
                SEGMENT_HOVER_ACCENT_BLEND,
            ),
            pressed_background: blend(
                settings.modal_surface,
                settings.accent,
                SEGMENT_PRESSED_ACCENT_BLEND,
            ),
            selected_background: settings.accent,
            disabled_background: TRANSPARENT,
            ..Self::action(settings)
        }
    }

    /// 无底色的内嵌图标操作，不绘制普通操作按钮边框。
    pub fn ghost(settings: SettingsTheme) -> Self {
        Self {
            foreground: settings.text_secondary,
            selected_foreground: settings.accent,
            background: TRANSPARENT,
            border: TRANSPARENT,
            hover_background: settings.hover_surface,
            pressed_background: settings.selected_surface,
            selected_background: settings.selected_surface,
            disabled_foreground: with_alpha(settings.text_secondary, BUTTON_DISABLED_ALPHA),
            disabled_background: TRANSPARENT,
            ..Self::action(settings)
        }
    }

    pub fn foreground_color(&self, state: ButtonVisualState, alpha: f32) -> [f32; 4] {
        let color = match state {
            ButtonVisualState::Disabled => self.disabled_foreground,
            ButtonVisualState::Selected => self.selected_foreground,
            _ => self.foreground,
        };
        with_alpha(color, alpha)
    }

    pub fn background_color(&self, state: ButtonVisualState, alpha: f32) -> [f32; 4] {
        let color = match state {
            ButtonVisualState::Normal => self.background,
            ButtonVisualState::Hovered => self.hover_background,
            ButtonVisualState::Pressed => self.pressed_background,
            ButtonVisualState::Selected => self.selected_background,
            ButtonVisualState::Disabled => self.disabled_background,
        };
        with_alpha(color, alpha)
    }

    pub fn border_color(&self, state: ButtonVisualState, alpha: f32) -> [f32; 4] {
        let state_alpha =
            if state == ButtonVisualState::Disabled { BUTTON_DISABLED_ALPHA } else { 1.0 };
        with_alpha(self.border, state_alpha * alpha)
    }

    pub fn paint_background(
        &self,
        context: &mut PaintCtx<'_>,
        rect: Rect,
        state: ButtonVisualState,
    ) {
        let color = self.background_color(state, context.global_alpha);
        if color[3] > 0.0 {
            context.list.fill_rounded(rect, color, self.corner_radius_logical * context.dpi);
        }
    }

    pub fn paint_outline(&self, context: &mut PaintCtx<'_>, rect: Rect, state: ButtonVisualState) {
        let color = self.border_color(state, context.global_alpha);
        if color[3] > 0.0 {
            context.list.stroke_rounded(
                rect,
                color,
                self.corner_radius_logical * context.dpi,
                context.dpi,
            );
        }
    }

    /// 绘制背景和边框，并返回已应用全局透明度的文字/图标颜色。
    pub fn paint(
        &self,
        context: &mut PaintCtx<'_>,
        rect: Rect,
        state: ButtonVisualState,
    ) -> [f32; 4] {
        self.paint_background(context, rect, state);
        self.paint_outline(context, rect, state);
        self.foreground_color(state, context.global_alpha)
    }
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

fn blend(base: [f32; 4], accent: [f32; 4], accent_factor: f32) -> [f32; 4] {
    std::array::from_fn(|index| base[index] * (1.0 - accent_factor) + accent[index] * accent_factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin_themes() -> [Theme; 2] {
        [
            Theme::resolve_builtin(crate::ThemeMode::Light, winit::window::Theme::Light),
            Theme::resolve_builtin(crate::ThemeMode::Dark, winit::window::Theme::Dark),
        ]
    }

    #[test]
    fn destructive_labels_remain_readable_in_every_enabled_state() {
        const MINIMUM_TEXT_CONTRAST: f32 = 4.5;
        let luminance = |color: [f32; 4]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        for theme in builtin_themes() {
            let style = ButtonStyle::destructive(theme.settings_theme());
            for state in [
                ButtonVisualState::Normal,
                ButtonVisualState::Hovered,
                ButtonVisualState::Pressed,
                ButtonVisualState::Selected,
            ] {
                let foreground = luminance(style.foreground_color(state, 1.0));
                let background = luminance(style.background_color(state, 1.0));
                let contrast =
                    (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
                assert!(
                    contrast >= MINIMUM_TEXT_CONTRAST,
                    "{state:?}: destructive label contrast is {contrast}"
                );
            }
        }
    }

    #[test]
    fn destructive_actions_preserve_metrics_and_keep_state_feedback_distinct() {
        for theme in builtin_themes() {
            let standard = ButtonStyle::from_theme(&theme);
            let destructive = ButtonStyle::destructive(theme.settings_theme());
            assert_eq!(destructive.font_size_logical, standard.font_size_logical);
            assert_eq!(destructive.pad_x_logical, standard.pad_x_logical);
            assert_eq!(destructive.corner_radius_logical, standard.corner_radius_logical);
            assert_ne!(destructive.background, destructive.hover_background);
            assert_ne!(destructive.hover_background, destructive.pressed_background);
            for alpha in [1.0, 0.5] {
                for state in [
                    ButtonVisualState::Normal,
                    ButtonVisualState::Hovered,
                    ButtonVisualState::Pressed,
                ] {
                    assert_eq!(
                        destructive.foreground_color(state, alpha),
                        with_alpha(theme.application_theme().button_danger_foreground, alpha)
                    );
                    assert_eq!(destructive.background_color(state, alpha)[3], alpha);
                    assert_eq!(destructive.border_color(state, alpha)[3], alpha);
                }
                assert_eq!(
                    destructive.foreground_color(ButtonVisualState::Disabled, alpha)[3],
                    BUTTON_DISABLED_ALPHA * alpha
                );
                assert_eq!(
                    destructive.background_color(ButtonVisualState::Disabled, alpha)[3],
                    BUTTON_DISABLED_ALPHA * alpha
                );
                assert_eq!(
                    destructive.border_color(ButtonVisualState::Disabled, alpha)[3],
                    BUTTON_DISABLED_ALPHA * alpha
                );
            }
        }
    }

    #[test]
    fn enabled_actions_have_a_distinct_surface_and_remain_opaque_during_interaction() {
        for theme in builtin_themes() {
            let style = ButtonStyle::from_theme(&theme);
            assert_ne!(
                style.background,
                theme.application_theme().control_surface,
                "可用操作按钮不能复用低层级的输入框底色"
            );
            for state in
                [ButtonVisualState::Normal, ButtonVisualState::Hovered, ButtonVisualState::Pressed]
            {
                let mut draw_list = crate::core::paint::DrawList::new();
                let mut context = PaintCtx::new(&mut draw_list, &theme, 1.0);
                let foreground = style.paint(&mut context, Rect::new(0.0, 0.0, 96.0, 28.0), state);
                assert_eq!(foreground, theme.application_theme().text_primary);
                assert!(
                    draw_list.cmds.iter().any(|command| matches!(command,
                        crate::core::paint::DrawCmd::FillRect { color, .. } if color[3] == 1.0
                    )),
                    "可用按钮的 {state:?} 状态必须保留完整底色"
                );
            }
        }
    }

    #[test]
    fn enabled_actions_have_a_visible_outline_distinct_from_disabled_and_input_controls() {
        for theme in builtin_themes() {
            let style = ButtonStyle::from_theme(&theme);
            let enabled = style.border_color(ButtonVisualState::Normal, 1.0);
            let disabled = style.border_color(ButtonVisualState::Disabled, 1.0);
            assert_ne!(
                enabled,
                theme.application_theme().control_border,
                "可用操作按钮应有独立于输入框的清晰轮廓"
            );
            assert_ne!(enabled, style.background);
            assert_eq!(enabled[3], 1.0);
            assert!(disabled[3] < enabled[3]);
        }
    }
}
