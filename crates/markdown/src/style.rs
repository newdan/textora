//! MarkdownStyle — pure data configuration for markdown rendering.

const DARK_INLINE_CODE_BG_BLEND_RATIO: f32 = 0.84;
const LIGHT_INLINE_CODE_BG_BLEND_RATIO: f32 = 0.90;

/// Styling configuration for markdown rendering.
/// Pure data, no logic. Derived from Theme + Settings at construction time.
#[derive(Clone, Debug)]
pub struct MarkdownStyle {
    // -- Fonts --
    pub body_font_size: f32,
    pub code_font_size: f32,
    pub heading_font_sizes: [f32; 6], // H1..H6
    pub body_font_family: Vec<String>,
    pub code_font_family: Option<String>,

    // -- Colors (RGBA) --
    pub text_color: [f32; 4],
    pub code_color: [f32; 4],
    pub code_bg: [f32; 4],
    pub inline_code_bg: [f32; 4],
    pub heading_color: [f32; 4],
    pub link_color: [f32; 4],
    pub rule_color: [f32; 4],
    pub blockquote_bg: [f32; 4],
    pub blockquote_border: [f32; 4],
    pub table_border: [f32; 4],
    pub table_header_bg: [f32; 4],
    pub table_stripe_bg: [f32; 4],

    // -- Spacing (pixels) --
    pub border_radius_base: f32,
    pub border_radius_small: f32,
    pub code_block_border: [f32; 4],
    pub list_item_spacing: f32,
    pub list_group_spacing: f32,
    pub rule_spacing: f32,
    pub paragraph_spacing: f32,
    pub paragraph_first_line_indent: f32,
    pub heading_spacing_top: f32,
    pub heading_spacing_bottom: f32,
    pub code_block_padding: f32,
    pub blockquote_padding: f32,
    pub list_indent: f32,
    pub table_cell_padding: f32,
    pub line_height: f32,
    pub code_line_height: f32,

    pub background_color: [f32; 4],

    // -- Misc --
    pub rule_thickness: f32,
    pub rule_width_ratio: f32, // fraction of viewport width
}

impl MarkdownStyle {
    /// Build a style from the current theme and editor settings.
    pub fn from_theme(theme: &ui::Theme, base_font_size: f32, line_height: f32) -> Self {
        let body_font_size = base_font_size;
        let code_font_size = base_font_size * 0.9;
        let heading_scale = [1.8, 1.3, 1.15, 1.2, 1.1, 0.95];
        let heading_font_sizes = heading_scale.map(|s| base_font_size * s);
        let body_font_family = vec!["PingFang SC".to_string()];
        let code_font_family = Some("monospace".to_string());

        let bg = theme.editor.background;
        let fg = theme.editor.foreground;
        let accent = theme.palette.accent;
        let is_dark = theme.is_dark;

        let code_bg = theme.markdown.code_bg;
        let _code_block_bg = theme.markdown.code_block_bg;
        let inline_code_bg_blend_ratio = if is_dark {
            DARK_INLINE_CODE_BG_BLEND_RATIO
        } else {
            LIGHT_INLINE_CODE_BG_BLEND_RATIO
        };
        let inline_code_bg =
            blend_toward_bg(theme.markdown.inline_code, bg, inline_code_bg_blend_ratio);

        let blockquote_border = if is_dark { blend_toward_bg(accent, bg, 0.75) } else { accent };
        let blockquote_bg = if is_dark {
            [accent[0], accent[1], accent[2], 0.08]
        } else {
            [accent[0], accent[1], accent[2], 0.05]
        };

        let table_border = theme.palette.border_subtle;
        let table_header_bg = theme.palette.bg_hover;
        let table_stripe_bg = [
            theme.palette.bg_hover[0],
            theme.palette.bg_hover[1],
            theme.palette.bg_hover[2],
            theme.palette.bg_hover[3] * 0.5,
        ];
        let code_block_border = theme.palette.border_subtle;
        let rule_color = theme.palette.border_subtle;

        let sp = &theme.markdown.spacing;

        Self {
            body_font_size,
            code_font_size,
            heading_font_sizes,
            body_font_family,
            code_font_family,
            text_color: fg,
            code_color: fg,
            code_bg,
            inline_code_bg,
            heading_color: theme.markdown.heading,
            link_color: theme.markdown.link,
            rule_color,
            blockquote_bg,
            blockquote_border,
            table_border,
            table_header_bg,
            table_stripe_bg,
            border_radius_base: sp.border_radius_base,
            border_radius_small: sp.border_radius_small,
            code_block_border,
            list_item_spacing: line_height * sp.list_item_spacing_ratio,
            list_group_spacing: line_height * sp.list_group_spacing_ratio,
            rule_spacing: sp.rule_spacing,
            paragraph_spacing: line_height * sp.paragraph_spacing_ratio,
            paragraph_first_line_indent: 0.0,
            heading_spacing_top: line_height * sp.heading_spacing_top_ratio,
            heading_spacing_bottom: line_height * sp.heading_spacing_bottom_ratio,
            code_block_padding: code_font_size * sp.code_block_padding_ratio,
            blockquote_padding: base_font_size * sp.blockquote_padding_ratio,
            list_indent: base_font_size * sp.list_indent_ratio,
            table_cell_padding: base_font_size * sp.table_cell_padding_ratio,
            line_height,
            code_line_height: code_font_size * sp.code_line_height_ratio,
            background_color: bg,
            rule_thickness: sp.rule_thickness,
            rule_width_ratio: sp.rule_width_ratio,
        }
    }

    /// Build a style for novel reading mode from the NovelTheme section.
    /// Uses the same MarkdownStyle structure but with novel-specific colors and spacing.
    pub fn novel(theme: &ui::Theme, base_font_size: f32, line_height: f32) -> Self {
        let body_font_size = base_font_size;
        let code_font_size = base_font_size * 0.9;
        let heading_scale = [1.8, 1.3, 1.15, 1.2, 1.1, 0.95];
        let heading_font_sizes = heading_scale.map(|s| base_font_size * s);
        let body_font_family = vec!["PingFang SC".to_string()];
        let code_font_family = Some("monospace".to_string());

        let nt = &theme.novel;
        let bg = theme.editor.background;
        let fg = theme.editor.foreground;
        let code_bg = bg; // novels don't use code blocks
        let inline_code_bg = bg;

        let sp = &nt.spacing;

        Self {
            body_font_size,
            code_font_size,
            heading_font_sizes,
            body_font_family,
            code_font_family,
            text_color: fg,
            code_color: fg,
            code_bg,
            inline_code_bg,
            heading_color: nt.heading,
            link_color: fg, // novels don't have links
            rule_color: theme.palette.border_subtle,
            blockquote_bg: [nt.quote[0], nt.quote[1], nt.quote[2], 0.05],
            blockquote_border: nt.quote,
            table_border: theme.palette.border_subtle,
            table_header_bg: theme.palette.bg_hover,
            table_stripe_bg: theme.palette.bg_hover,
            border_radius_base: sp.border_radius_base,
            border_radius_small: sp.border_radius_small,
            code_block_border: theme.palette.border_subtle,
            list_item_spacing: line_height * sp.list_item_spacing_ratio,
            list_group_spacing: line_height * sp.list_group_spacing_ratio,
            rule_spacing: sp.rule_spacing,
            paragraph_spacing: line_height * sp.paragraph_spacing_ratio,
            paragraph_first_line_indent: 0.0,
            heading_spacing_top: line_height * sp.heading_spacing_top_ratio,
            heading_spacing_bottom: line_height * sp.heading_spacing_bottom_ratio,
            code_block_padding: code_font_size * sp.code_block_padding_ratio,
            blockquote_padding: base_font_size * sp.blockquote_padding_ratio,
            list_indent: base_font_size * sp.list_indent_ratio,
            table_cell_padding: base_font_size * sp.table_cell_padding_ratio,
            line_height,
            code_line_height: code_font_size * sp.code_line_height_ratio,
            background_color: bg,
            rule_thickness: sp.rule_thickness,
            rule_width_ratio: sp.rule_width_ratio,
        }
    }
}

/// Blend a color toward a target color by a ratio, keeping alpha at 1.0.
/// This preserves subpixel anti-aliasing (unlike alpha fading).
/// ratio: 0.0 = keep original, 1.0 = fully target color.
pub fn blend_toward_bg(color: [f32; 4], target: [f32; 4], ratio: f32) -> [f32; 4] {
    let r = ratio.clamp(0.0, 1.0);
    [
        color[0] + (target[0] - color[0]) * r,
        color[1] + (target[1] - color[1]) * r,
        color[2] + (target[2] - color[2]) * r,
        1.0, // always fully opaque for crisp subpixel rendering
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_toward_bg_identity_at_zero() {
        let color = [0.2, 0.4, 0.6, 1.0];
        let bg = [1.0, 1.0, 1.0, 1.0];
        let result = blend_toward_bg(color, bg, 0.0);
        assert_eq!(result, color);
    }

    #[test]
    fn blend_toward_bg_fully_target_at_one() {
        let color = [0.2, 0.4, 0.6, 1.0];
        let bg = [1.0, 1.0, 1.0, 1.0];
        let result = blend_toward_bg(color, bg, 1.0);
        assert!((result[0] - 1.0).abs() < 1e-6);
        assert!((result[1] - 1.0).abs() < 1e-6);
        assert!((result[2] - 1.0).abs() < 1e-6);
        assert_eq!(result[3], 1.0); // alpha always 1.0
    }

    #[test]
    fn blend_toward_bg_alpha_always_one() {
        let color = [0.2, 0.4, 0.6, 0.5]; // input alpha ignored
        let bg = [1.0, 1.0, 1.0, 0.8];
        let result = blend_toward_bg(color, bg, 0.5);
        assert_eq!(result[3], 1.0, "alpha must always be 1.0 for subpixel rendering");
    }

    #[test]
    fn blend_toward_bg_clamps_ratio() {
        let color = [0.0, 0.0, 0.0, 1.0];
        let bg = [1.0, 1.0, 1.0, 1.0];
        let r1 = blend_toward_bg(color, bg, -0.5);
        let r2 = blend_toward_bg(color, bg, 1.5);
        assert_eq!(r1, color, "negative ratio clamps to 0");
        assert!((r2[0] - 1.0).abs() < 1e-6, "ratio > 1 clamps to 1");
    }

    #[test]
    fn from_theme_has_background_color() {
        let theme = ui::theme::test_theme();
        let style = MarkdownStyle::from_theme(&theme, 15.0, 24.0);
        assert_eq!(style.background_color, theme.editor.background);
    }

    fn assert_color_close(actual: [f32; 4], expected: [f32; 4]) {
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 1e-6,
                "channel {channel} differs: actual={actual:?}, expected={expected:?}"
            );
        }
    }

    #[test]
    fn from_theme_dark_inline_code_bg_uses_inline_code_token() {
        const EXPECTED_DARK_BLEND_RATIO: f32 = 0.84;

        let theme = ui::theme::test_theme();
        let style = MarkdownStyle::from_theme(&theme, 15.0, 24.0);
        let expected = blend_toward_bg(
            theme.markdown.inline_code,
            theme.editor.background,
            EXPECTED_DARK_BLEND_RATIO,
        );

        assert_color_close(style.inline_code_bg, expected);
        assert_ne!(
            style.inline_code_bg, style.code_bg,
            "inline code background should be distinct from code block background"
        );
    }

    #[test]
    fn from_theme_light_inline_code_bg_uses_inline_code_token() {
        const EXPECTED_LIGHT_BLEND_RATIO: f32 = 0.90;

        let theme = ui::theme::test_light_theme();
        let style = MarkdownStyle::from_theme(&theme, 15.0, 24.0);
        let expected = blend_toward_bg(
            theme.markdown.inline_code,
            theme.editor.background,
            EXPECTED_LIGHT_BLEND_RATIO,
        );

        assert_color_close(style.inline_code_bg, expected);
        assert_ne!(
            style.inline_code_bg, style.code_bg,
            "inline code background should be distinct from code block background"
        );
    }

    #[test]
    fn from_theme_spacing_scales_with_line_height() {
        let theme = ui::theme::test_theme();
        let s1 = MarkdownStyle::from_theme(&theme, 15.0, 24.0);
        let s2 = MarkdownStyle::from_theme(&theme, 15.0, 48.0);
        // Spacing proportional to line_height should double
        assert!((s2.paragraph_spacing - s1.paragraph_spacing * 2.0).abs() < 0.01);
        assert!((s2.heading_spacing_top - s1.heading_spacing_top * 2.0).abs() < 0.01);
        assert!((s2.heading_spacing_bottom - s1.heading_spacing_bottom * 2.0).abs() < 0.01);
        assert!((s2.list_item_spacing - s1.list_item_spacing * 2.0).abs() < 0.01);
    }

    #[test]
    fn from_theme_spacing_scales_with_font_size() {
        let theme = ui::theme::test_theme();
        let s1 = MarkdownStyle::from_theme(&theme, 15.0, 24.0);
        let s2 = MarkdownStyle::from_theme(&theme, 30.0, 24.0);
        // Spacing proportional to base_font_size should double
        assert!((s2.list_indent - s1.list_indent * 2.0).abs() < 0.01);
        assert!((s2.blockquote_padding - s1.blockquote_padding * 2.0).abs() < 0.01);
        assert!((s2.table_cell_padding - s1.table_cell_padding * 2.0).abs() < 0.01);
    }

    #[test]
    fn from_theme_default_values_match_old_formulas() {
        let theme = ui::theme::test_theme();
        let base = 15.0_f32;
        let lh = 24.0_f32;
        let style = MarkdownStyle::from_theme(&theme, base, lh);
        let code_fs = base * 0.9;
        // Verify against old formulas
        assert!((style.paragraph_spacing - lh * 0.5).abs() < 0.01);
        assert!((style.heading_spacing_top - lh * 1.0).abs() < 0.01);
        assert!((style.heading_spacing_bottom - lh * 0.45).abs() < 0.01);
        assert!((style.list_item_spacing - lh * 0.15).abs() < 0.01);
        assert!((style.list_group_spacing - lh * 0.5).abs() < 0.01);
        assert!((style.list_indent - base * 2.0).abs() < 0.01);
        assert!((style.code_block_padding - code_fs * 0.8).abs() < 0.01);
        assert!((style.blockquote_padding - base * 0.65).abs() < 0.01);
        assert!((style.table_cell_padding - base * 0.5).abs() < 0.01);
        assert!((style.code_line_height - code_fs * 1.5).abs() < 0.01);
    }
}
