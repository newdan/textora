use crate::core::{PaintCtx, Rect};
use crate::theme::{ControlMetrics, SettingsTheme, Theme};

const BUTTON_DISABLED_ALPHA: f32 = 0.45;
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
            background: settings.control_surface,
            border: settings.control_border,
            hover_background: settings.hover_surface,
            pressed_background: settings.selected_surface,
            selected_background: settings.selected_surface,
            disabled_foreground: with_alpha(settings.text_primary, BUTTON_DISABLED_ALPHA),
            disabled_background: with_alpha(settings.control_surface, BUTTON_DISABLED_ALPHA),
            corner_radius_logical: metrics.corner_radius_logical,
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
