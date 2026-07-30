//! Color theme system — centralized color management with light/dark support.
//!
//! Follows system appearance automatically via winit Theme events.
//! Syntax colors are inspired by Zed's One Dark / One Light palettes.

mod color;
mod editor;
mod markdown;
mod mindmap;
mod novel;
mod settings;

pub use color::ColorPalette;
pub use editor::EditorTheme;
pub use markdown::{MarkdownSpacing, MarkdownTheme};
pub use mindmap::*;
pub use novel::NovelTheme;
pub use settings::SettingsTheme;

pub use crate::theme_registry::{
    BUILTIN_DARK_ID, BUILTIN_LIGHT_ID, RegisterError, ThemeLoadError, ThemeRegistrationReport,
    ThemeRegistry, ThemeSource,
};

use std::collections::{BTreeMap, HashMap};

/// A modular color theme for the editor.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub novel: NovelTheme,
    pub mindmap: MindmapTheme,
    pub scopes: HashMap<String, [f32; 4]>,
}

/// Pure-data theme definition in sRGB space.
/// `Theme::from_definition()` converts to linear-space Theme via gamma correction.
#[derive(Debug, Clone)]
pub struct ThemeDefinition {
    pub display_name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub novel: NovelTheme,
    pub mindmap: MindmapTheme,
    pub scopes: BTreeMap<String, [f32; 4]>,
}

/// Stores the active light/dark theme identifiers for quick switching.
/// The actual definitions are resolved via `ThemeRegistry` at theme-build time.
#[derive(Debug, Clone)]
pub struct ActiveThemePair {
    /// Theme id for light mode (e.g. "default-light").
    pub light: String,
    /// Theme id for dark mode (e.g. "default-dark").
    pub dark: String,
}

impl Default for ActiveThemePair {
    fn default() -> Self {
        Self { light: "default-light".into(), dark: "default-dark".into() }
    }
}

impl Theme {
    /// Build a gamma-corrected Theme from a pure-data definition.
    /// Definition values are sRGB; gamma correction converts to linear.
    pub fn from_definition(def: &ThemeDefinition) -> Self {
        let mut theme = Self {
            name: def.display_name.clone(),
            is_dark: def.is_dark,
            palette: def.palette.clone(),
            editor: def.editor.clone(),
            markdown: def.markdown.clone(),
            novel: def.novel.clone(),
            mindmap: def.mindmap.clone(),
            scopes: def.scopes.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        };
        theme.gamma_correct();
        theme
    }

    /// Resolve theme from ThemeMode + system theme using registry and pair.
    /// Pair stores theme identifiers; registry resolves them to definitions.
    /// If a user-registered theme is missing (e.g. file deleted), falls back
    /// gracefully to the built-in default for the requested appearance.
    pub fn resolve(
        mode: crate::settings::ThemeMode,
        system_theme: winit::window::Theme,
        pair: &ActiveThemePair,
        registry: &ThemeRegistry,
    ) -> Self {
        let is_dark = match mode {
            crate::settings::ThemeMode::System => system_theme == winit::window::Theme::Dark,
            crate::settings::ThemeMode::Dark => true,
            crate::settings::ThemeMode::Light => false,
        };
        let id = if is_dark { &pair.dark } else { &pair.light };
        Theme::from_definition(registry.get_or_default(id, is_dark))
    }

    /// Apply display gamma correction (~2.2) so sRGB-intended colors appear
    /// correctly on non-sRGB framebuffers (e.g. macOS Metal surfaces).
    fn gamma_correct(&mut self) {
        self.palette.gamma_correct();
        self.editor.gamma_correct();
        self.markdown.gamma_correct();
        self.novel.gamma_correct();
        self.mindmap.gamma_correct();
        let gamma = 2.2;
        for color in self.scopes.values_mut() {
            for ch in color[..3].iter_mut() {
                *ch = ch.powf(gamma);
            }
        }
    }

    /// Look up a color by scope name, falling back to editor foreground.
    pub fn scope_color(&self, name: &str) -> [f32; 4] {
        self.scopes.get(name).copied().unwrap_or(self.editor.foreground)
    }

    pub fn settings_theme(&self) -> SettingsTheme {
        SettingsTheme::from_palette(&self.palette)
    }
}

impl ThemeDefinition {
    /// Built-in default dark theme (Claude Dark values, sRGB).
    pub fn default_dark() -> Self {
        Self {
            display_name: "默认黑".into(),
            is_dark: true,
            palette: ColorPalette {
                bg_base: [0.0667, 0.0745, 0.0941, 1.0000],
                bg_surface: [0.0902, 0.0980, 0.1216, 1.0000],
                bg_elevated: [0.0941, 0.1059, 0.1333, 1.0000],
                bg_hover: [1.0, 1.0, 1.0, 0.08],
                bg_active: [0.9608, 0.6039, 0.2902, 0.1412],
                text_main: [0.9490, 0.9569, 0.9725, 1.0000],
                text_muted: [0.5569, 0.5922, 0.6588, 1.0000],
                text_inverse: [0.0667, 0.0745, 0.0941, 1.0000],
                border_subtle: [0.1373, 0.1569, 0.1961, 1.0000],
                border_strong: [0.1647, 0.1843, 0.2275, 1.0000],
                shadow: [0.0000, 0.0000, 0.0000, 0.3216],
                accent: [0.9608, 0.6039, 0.2902, 1.0000],
                highlight: [0.9608, 0.6039, 0.2902, 1.0000],
                inactive_highlight: [0.80, 0.60, 0.25, 0.70],
                danger: [0.8941, 0.3373, 0.2863, 1.0],
                warning: [0.8745, 0.7569, 0.5176, 1.0],
                input_bg: [0.055, 0.053, 0.051, 1.0],
                input_border: [0.08, 0.078, 0.076, 1.0],
                input_fg: [0.9608, 0.9529, 0.9412, 1.0],
                sidebar_hover_bg: [0.085, 0.083, 0.081, 1.0],
                sidebar_active_bg: [0.13, 0.10, 0.08, 1.0],
                sidebar_active_fg: [0.95, 0.65, 0.35, 1.0],
            },
            editor: EditorTheme {
                background: [0.0941, 0.1059, 0.1333, 1.0000],
                foreground: [0.7686, 0.7922, 0.8353, 1.0000],
                gutter_bg: [0.0941, 0.1059, 0.1333, 1.0000],
                line_number: [0.4078, 0.4471, 0.5137, 1.0000],
                selection: [0.3569, 0.6118, 1.0000, 0.2510],
                cursor: [0.3569, 0.6118, 1.0000, 1.0000],
                scrollbar_track: [0.1800, 0.2000, 0.2350, 0.30],
                scrollbar_thumb: [0.6000, 0.6000, 0.6500, 1.0],
            },
            markdown: MarkdownTheme {
                heading: [0.75, 0.75, 0.75, 1.0],
                link: [0.4510, 0.6784, 0.9137, 1.0],
                inline_code: [0.6745, 0.6980, 0.7451, 1.0],
                code_bg: [0.122, 0.122, 0.122, 1.0],
                code_block_bg: [0.122, 0.122, 0.122, 1.0],
                toc_background: [0.08, 0.078, 0.076, 1.0],
                toc_active_background: [0.8706, 0.4510, 0.3373, 0.12],
                toc_hover_background: [0.8706, 0.4510, 0.3373, 0.08],
                toc_text: [0.9608, 0.9529, 0.9412, 1.0],
                toc_hover_text: [0.9608, 0.9529, 0.9412, 1.0],
                toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
                spacing: MarkdownSpacing {
                    paragraph_spacing_ratio: 0.5,
                    heading_spacing_top_ratio: 1.0,
                    heading_spacing_bottom_ratio: 0.45,
                    list_item_spacing_ratio: 0.15,
                    list_group_spacing_ratio: 0.5,
                    list_indent_ratio: 2.0,
                    code_block_padding_ratio: 0.8,
                    code_line_height_ratio: 1.5,
                    blockquote_padding_ratio: 0.65,
                    table_cell_padding_ratio: 0.5,
                    rule_spacing: 12.0,
                    rule_thickness: 2.0,
                    rule_width_ratio: 1.0,
                    border_radius_base: 8.0,
                    border_radius_small: 4.0,
                },
            },
            novel: NovelTheme {
                heading: [0.80, 0.78, 0.72, 1.0],
                quote: [0.9608, 0.6039, 0.2902, 1.0000],
                toc_text: [0.9608, 0.9529, 0.9412, 1.0],
                toc_hover_text: [0.9608, 0.9529, 0.9412, 1.0],
                toc_background: [0.08, 0.078, 0.076, 1.0],
                toc_active_background: [0.8706, 0.4510, 0.3373, 0.12],
                spacing: MarkdownSpacing {
                    paragraph_spacing_ratio: 0.5,
                    heading_spacing_top_ratio: 1.3,
                    heading_spacing_bottom_ratio: 0.55,
                    list_item_spacing_ratio: 0.15,
                    list_group_spacing_ratio: 0.5,
                    list_indent_ratio: 2.0,
                    code_block_padding_ratio: 0.8,
                    code_line_height_ratio: 1.5,
                    blockquote_padding_ratio: 0.65,
                    table_cell_padding_ratio: 0.5,
                    rule_spacing: 16.0,
                    rule_thickness: 2.0,
                    rule_width_ratio: 1.0,
                    border_radius_base: 8.0,
                    border_radius_small: 4.0,
                },
            },
            mindmap: MindmapTheme::default_dark(),
            scopes: BTreeMap::from([
                ("comment".into(), [0.3647, 0.3882, 0.4353, 1.0]),
                ("string".into(), [0.6314, 0.7569, 0.5059, 1.0]),
                ("keyword.control".into(), [0.7059, 0.4667, 0.8118, 1.0]),
                ("keyword.other".into(), [0.7059, 0.4667, 0.8118, 1.0]),
                ("constant.numeric".into(), [0.7490, 0.5843, 0.4157, 1.0]),
                ("constant.language".into(), [0.8745, 0.7569, 0.5176, 1.0]),
                ("variable".into(), [0.6745, 0.6980, 0.7451, 1.0]),
                ("variable.special".into(), [0.7490, 0.5843, 0.4157, 1.0]),
                ("keyword.import".into(), [0.7059, 0.4667, 0.8118, 1.0]),
                ("keyword.declaration".into(), [0.7059, 0.4667, 0.8118, 1.0]),
                ("boolean".into(), [0.7490, 0.5843, 0.4157, 1.0]),
                ("method".into(), [0.4510, 0.6784, 0.9137, 1.0]),
                ("meta.header".into(), [0.4549, 0.6784, 0.9098, 1.0]),
                ("markup.heading".into(), [0.8157, 0.4471, 0.4667, 1.0]),
                ("markup.bold".into(), [0.7490, 0.5843, 0.4157, 1.0]),
                ("markup.italic".into(), [0.4549, 0.6784, 0.9098, 1.0]),
                ("markup.list".into(), [0.8157, 0.4471, 0.4667, 1.0]),
                ("markup.link".into(), [0.4510, 0.6784, 0.9137, 1.0]),
                ("markup.strikethrough".into(), [0.6745, 0.6980, 0.7451, 1.0]),
                ("markup.changed".into(), [0.8745, 0.7569, 0.5176, 1.0]),
                ("markup.deleted".into(), [0.8784, 0.4235, 0.4588, 1.0]),
                ("markup.inserted".into(), [0.5961, 0.7647, 0.4745, 1.0]),
                ("property".into(), [0.8157, 0.4471, 0.4667, 1.0]),
            ]),
        }
    }

    /// Built-in default light theme (Claude Light values, sRGB).
    pub fn default_light() -> Self {
        Self {
            display_name: "默认白".into(),
            is_dark: false,
            palette: ColorPalette {
                bg_base: [0.9686, 0.9647, 0.9529, 1.0000],
                bg_surface: [0.9804, 0.9765, 0.9686, 1.0000],
                bg_elevated: [1.0000, 1.0000, 1.0000, 1.0000],
                bg_hover: [0.0, 0.0, 0.0, 0.06],
                bg_active: [1.0000, 0.9529, 0.9098, 1.0000],
                text_main: [0.0667, 0.0941, 0.1529, 1.0000],
                text_muted: [0.4196, 0.4471, 0.5020, 1.0000],
                text_inverse: [1.0000, 1.0000, 1.0000, 1.0000],
                border_subtle: [0.9373, 0.9176, 0.8902, 1.0000],
                border_strong: [0.9020, 0.8824, 0.8471, 1.0000],
                shadow: [0.0000, 0.0000, 0.0000, 0.0588],
                accent: [0.9137, 0.4745, 0.1412, 1.0000],
                highlight: [0.9137, 0.4745, 0.1412, 1.0000],
                inactive_highlight: [0.7, 0.6, 0.2, 0.5],
                danger: [0.8941, 0.3373, 0.2863, 1.0],
                warning: [0.7569, 0.5176, 0.0039, 1.0],
                input_bg: [0.9529, 0.9490, 0.9333, 1.0],
                input_border: [0.9137, 0.9020, 0.8824, 1.0],
                input_fg: [0.1, 0.1, 0.08, 1.0],
                sidebar_hover_bg: [1.0, 0.9529, 0.9137, 1.0],
                sidebar_active_bg: [0.9569, 0.9137, 0.8745, 1.0],
                sidebar_active_fg: [0.6039, 0.3020, 0.0706, 1.0],
            },
            editor: EditorTheme {
                background: [1.0000, 1.0000, 1.0000, 1.0000],
                foreground: [0.2157, 0.2549, 0.3176, 1.0000],
                gutter_bg: [1.0000, 1.0000, 1.0000, 1.0000],
                line_number: [0.6039, 0.6392, 0.6863, 1.0000],
                selection: [0.9137, 0.4745, 0.1412, 0.1490],
                cursor: [0.9137, 0.4745, 0.1412, 1.0000],
                scrollbar_track: [0.7800, 0.7800, 0.7800, 0.30],
                scrollbar_thumb: [0.4000, 0.4000, 0.4500, 1.0],
            },
            markdown: MarkdownTheme {
                heading: [0.15, 0.15, 0.15, 1.0],
                link: [0.3569, 0.4745, 0.8902, 1.0],
                inline_code: [0.1412, 0.1451, 0.1608, 1.0],
                code_bg: [0.9725, 0.9686, 0.9608, 1.0],
                code_block_bg: [0.9725, 0.9686, 0.9608, 1.0],
                toc_background: [0.98, 0.96, 0.93, 1.0],
                toc_active_background: [0.8706, 0.4510, 0.3373, 0.12],
                toc_hover_background: [0.8706, 0.4510, 0.3373, 0.08],
                toc_text: [0.1, 0.1, 0.08, 1.0],
                toc_hover_text: [0.1, 0.1, 0.08, 1.0],
                toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
                spacing: MarkdownSpacing {
                    paragraph_spacing_ratio: 0.5,
                    heading_spacing_top_ratio: 1.0,
                    heading_spacing_bottom_ratio: 0.45,
                    list_item_spacing_ratio: 0.15,
                    list_group_spacing_ratio: 0.5,
                    list_indent_ratio: 2.0,
                    code_block_padding_ratio: 0.8,
                    code_line_height_ratio: 1.5,
                    blockquote_padding_ratio: 0.65,
                    table_cell_padding_ratio: 0.5,
                    rule_spacing: 12.0,
                    rule_thickness: 2.0,
                    rule_width_ratio: 1.0,
                    border_radius_base: 8.0,
                    border_radius_small: 4.0,
                },
            },
            novel: NovelTheme {
                heading: [0.15, 0.15, 0.12, 1.0],
                quote: [0.9137, 0.4745, 0.1412, 1.0000],
                toc_text: [0.1, 0.1, 0.08, 1.0],
                toc_hover_text: [0.1, 0.1, 0.08, 1.0],
                toc_background: [0.96, 0.95, 0.93, 1.0],
                toc_active_background: [0.8706, 0.4510, 0.3373, 0.08],
                spacing: MarkdownSpacing {
                    paragraph_spacing_ratio: 0.5,
                    heading_spacing_top_ratio: 1.3,
                    heading_spacing_bottom_ratio: 0.55,
                    list_item_spacing_ratio: 0.15,
                    list_group_spacing_ratio: 0.5,
                    list_indent_ratio: 2.0,
                    code_block_padding_ratio: 0.8,
                    code_line_height_ratio: 1.5,
                    blockquote_padding_ratio: 0.65,
                    table_cell_padding_ratio: 0.5,
                    rule_spacing: 16.0,
                    rule_thickness: 2.0,
                    rule_width_ratio: 1.0,
                    border_radius_base: 8.0,
                    border_radius_small: 4.0,
                },
            },
            mindmap: MindmapTheme::default_light(),
            scopes: BTreeMap::from([
                ("comment".into(), [0.6353, 0.6392, 0.6549, 1.0]),
                ("string".into(), [0.3922, 0.6235, 0.3412, 1.0]),
                ("keyword.control".into(), [0.6431, 0.2863, 0.6706, 1.0]),
                ("keyword.other".into(), [0.6431, 0.2863, 0.6706, 1.0]),
                ("constant.numeric".into(), [0.6784, 0.4314, 0.1451, 1.0]),
                ("constant.language".into(), [0.7569, 0.5176, 0.0039, 1.0]),
                ("variable".into(), [0.1412, 0.1451, 0.1608, 1.0]),
                ("variable.special".into(), [0.6784, 0.4314, 0.1451, 1.0]),
                ("keyword.import".into(), [0.6431, 0.2863, 0.6706, 1.0]),
                ("keyword.declaration".into(), [0.6431, 0.2863, 0.6706, 1.0]),
                ("boolean".into(), [0.6784, 0.4314, 0.1451, 1.0]),
                ("method".into(), [0.3569, 0.4745, 0.8902, 1.0]),
                ("meta.header".into(), [0.3608, 0.4706, 0.8863, 1.0]),
                ("markup.heading".into(), [0.8275, 0.3765, 0.3098, 1.0]),
                ("markup.bold".into(), [0.6784, 0.4314, 0.1451, 1.0]),
                ("markup.italic".into(), [0.3608, 0.4706, 0.8863, 1.0]),
                ("markup.list".into(), [0.8275, 0.3765, 0.3098, 1.0]),
                ("markup.link".into(), [0.3569, 0.4745, 0.8902, 1.0]),
                ("markup.strikethrough".into(), [0.1412, 0.1451, 0.1608, 1.0]),
                ("markup.changed".into(), [0.7569, 0.5176, 0.0039, 1.0]),
                ("markup.deleted".into(), [0.8941, 0.3373, 0.2863, 1.0]),
                ("markup.inserted".into(), [0.3137, 0.6314, 0.3098, 1.0]),
                ("property".into(), [0.8275, 0.3765, 0.3098, 1.0]),
            ]),
        }
    }
}

/// Default theme for tests (One Dark values, gamma-corrected).
/// One Dark values are intentionally hardcoded as stable test fixtures —
/// distinct from the built-in Claude Dark defaults used in production.
pub fn test_theme() -> Theme {
    let def = ThemeDefinition {
        display_name: "Test Dark".into(),
        is_dark: true,
        palette: ColorPalette {
            bg_base: [0.1569, 0.1725, 0.2000, 1.0],
            bg_surface: [0.188, 0.204, 0.224, 1.0],
            bg_elevated: [0.211, 0.227, 0.247, 1.0],
            bg_hover: [0.2118, 0.2353, 0.2745, 1.0],
            bg_active: [0.2706, 0.2902, 0.3373, 1.0],
            text_main: [0.8627, 0.8784, 0.8980, 1.0],
            text_muted: [0.65, 0.65, 0.65, 1.0],
            text_inverse: [0.9, 0.9, 0.9, 1.0],
            border_subtle: [0.133, 0.145, 0.161, 1.0],
            border_strong: [0.133, 0.145, 0.161, 1.0],
            shadow: [0.0, 0.0, 0.0, 0.30],
            accent: [0.4549, 0.6784, 0.9098, 1.0],
            highlight: [1.0, 0.65, 0.2, 0.90],
            inactive_highlight: [0.80, 0.60, 0.25, 0.70],
            danger: [0.8941, 0.3373, 0.2863, 1.0],
            warning: [0.8745, 0.7569, 0.5176, 1.0],
            input_bg: [0.188, 0.204, 0.224, 1.0],
            input_border: [0.133, 0.145, 0.161, 1.0],
            input_fg: [0.8627, 0.8784, 0.8980, 1.0],
            sidebar_hover_bg: [0.22, 0.23, 0.25, 1.0],
            sidebar_active_bg: [0.25, 0.22, 0.18, 1.0],
            sidebar_active_fg: [0.95, 0.65, 0.35, 1.0],
        },
        editor: EditorTheme {
            background: [0.1569, 0.1725, 0.2000, 1.0],
            foreground: [0.6745, 0.6980, 0.7451, 1.0],
            gutter_bg: [0.1569, 0.1725, 0.2000, 1.0],
            line_number: [0.3059, 0.3529, 0.3725, 1.0],
            selection: [0.4549, 0.6784, 0.9098, 0.2392],
            cursor: [0.4549, 0.6784, 0.9098, 1.0],
            scrollbar_track: [0.1800, 0.2000, 0.2350, 0.30],
            scrollbar_thumb: [0.6000, 0.6000, 0.6500, 1.0],
        },
        markdown: MarkdownTheme {
            heading: [0.75, 0.75, 0.75, 1.0],
            link: [0.4510, 0.6784, 0.9137, 1.0],
            inline_code: [0.6745, 0.6980, 0.7451, 1.0],
            code_bg: [0.122, 0.122, 0.122, 1.0],
            code_block_bg: [0.122, 0.122, 0.122, 1.0],
            toc_background: [0.28, 0.26, 0.23, 1.0],
            toc_active_background: [0.33, 0.29, 0.24, 0.5],
            toc_hover_background: [0.40, 0.36, 0.32, 0.5],
            toc_text: [0.6745, 0.6980, 0.7451, 1.0],
            toc_hover_text: [0.80, 0.82, 0.86, 1.0],
            toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
            spacing: MarkdownSpacing {
                paragraph_spacing_ratio: 0.5,
                heading_spacing_top_ratio: 1.0,
                heading_spacing_bottom_ratio: 0.45,
                list_item_spacing_ratio: 0.15,
                list_group_spacing_ratio: 0.5,
                list_indent_ratio: 2.0,
                code_block_padding_ratio: 0.8,
                code_line_height_ratio: 1.5,
                blockquote_padding_ratio: 0.65,
                table_cell_padding_ratio: 0.5,
                rule_spacing: 12.0,
                rule_thickness: 2.0,
                rule_width_ratio: 1.0,
                border_radius_base: 8.0,
                border_radius_small: 4.0,
            },
        },
        novel: NovelTheme {
            heading: [0.75, 0.75, 0.75, 1.0],
            quote: [0.4510, 0.6784, 0.9137, 1.0],
            toc_text: [0.8627, 0.8784, 0.8980, 1.0],
            toc_hover_text: [0.8627, 0.8784, 0.8980, 1.0],
            toc_background: [0.28, 0.26, 0.23, 1.0],
            toc_active_background: [0.33, 0.29, 0.24, 0.5],
            spacing: MarkdownSpacing {
                paragraph_spacing_ratio: 0.5,
                heading_spacing_top_ratio: 1.3,
                heading_spacing_bottom_ratio: 0.55,
                list_item_spacing_ratio: 0.15,
                list_group_spacing_ratio: 0.5,
                list_indent_ratio: 2.0,
                code_block_padding_ratio: 0.8,
                code_line_height_ratio: 1.5,
                blockquote_padding_ratio: 0.65,
                table_cell_padding_ratio: 0.5,
                rule_spacing: 16.0,
                rule_thickness: 2.0,
                rule_width_ratio: 1.0,
                border_radius_base: 8.0,
                border_radius_small: 4.0,
            },
        },
        mindmap: MindmapTheme::default_dark(),
        scopes: BTreeMap::from([
            ("comment".into(), [0.6353, 0.6392, 0.6549, 1.0]),
            ("string".into(), [0.3922, 0.6235, 0.3412, 1.0]),
            ("keyword.control".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("keyword.other".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("constant.numeric".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("constant.language".into(), [0.7569, 0.5176, 0.0039, 1.0]),
            ("variable".into(), [0.1412, 0.1451, 0.1608, 1.0]),
            ("variable.special".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("keyword.import".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("keyword.declaration".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("boolean".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("method".into(), [0.3569, 0.4745, 0.8902, 1.0]),
            ("meta.header".into(), [0.3608, 0.4706, 0.8863, 1.0]),
            ("markup.heading".into(), [0.8275, 0.3765, 0.3098, 1.0]),
            ("markup.bold".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("markup.italic".into(), [0.3608, 0.4706, 0.8863, 1.0]),
            ("markup.list".into(), [0.8275, 0.3765, 0.3098, 1.0]),
            ("markup.link".into(), [0.3569, 0.4745, 0.8902, 1.0]),
            ("markup.strikethrough".into(), [0.1412, 0.1451, 0.1608, 1.0]),
            ("markup.changed".into(), [0.7569, 0.5176, 0.0039, 1.0]),
            ("markup.deleted".into(), [0.8941, 0.3373, 0.2863, 1.0]),
            ("markup.inserted".into(), [0.3137, 0.6314, 0.3098, 1.0]),
            ("property".into(), [0.8275, 0.3765, 0.3098, 1.0]),
        ]),
    };
    Theme::from_definition(&def)
}

/// Default light theme for tests (One Light values, gamma-corrected).
pub fn test_light_theme() -> Theme {
    Theme::from_definition(&ThemeDefinition {
        display_name: "Test Light".into(),
        is_dark: false,
        palette: ColorPalette {
            bg_base: [0.9804, 0.9804, 0.9804, 1.0],
            bg_surface: [0.906, 0.906, 0.914, 1.0],
            bg_elevated: [0.935, 0.935, 0.941, 1.0],
            bg_hover: [0.8745, 0.8745, 0.8784, 1.0],
            bg_active: [0.7922, 0.7922, 0.7922, 1.0],
            text_main: [0.1412, 0.1451, 0.1608, 1.0],
            text_muted: [0.4196, 0.4392, 0.5020, 1.0],
            text_inverse: [0.15, 0.15, 0.15, 1.0],
            border_subtle: [0.9020, 0.8824, 0.8471, 1.0],
            border_strong: [0.9020, 0.8824, 0.8471, 1.0],
            shadow: [0.0, 0.0, 0.0, 0.08],
            accent: [0.4549, 0.6784, 0.9098, 1.0],
            highlight: [1.0, 0.55, 0.15, 0.7],
            inactive_highlight: [0.7, 0.6, 0.2, 0.5],
            danger: [0.8941, 0.3373, 0.2863, 1.0],
            warning: [0.7569, 0.5176, 0.0039, 1.0],
            input_bg: [0.906, 0.906, 0.914, 1.0],
            input_border: [0.816, 0.816, 0.824, 1.0],
            input_fg: [0.1412, 0.1451, 0.1608, 1.0],
            sidebar_hover_bg: [1.0, 0.9529, 0.9137, 1.0],
            sidebar_active_bg: [0.9569, 0.9137, 0.8745, 1.0],
            sidebar_active_fg: [0.6039, 0.3020, 0.0706, 1.0],
        },
        editor: EditorTheme {
            background: [0.9804, 0.9804, 0.9804, 1.0],
            foreground: [0.1412, 0.1451, 0.1608, 1.0],
            gutter_bg: [0.9804, 0.9804, 0.9804, 1.0],
            line_number: [0.7059, 0.7059, 0.7333, 1.0],
            selection: [0.4549, 0.6784, 0.9098, 0.2392],
            cursor: [0.4549, 0.6784, 0.9098, 1.0],
            scrollbar_track: [0.7800, 0.7800, 0.7800, 0.30],
            scrollbar_thumb: [0.4000, 0.4000, 0.4500, 1.0],
        },
        markdown: MarkdownTheme {
            heading: [0.15, 0.15, 0.15, 1.0],
            link: [0.3569, 0.4745, 0.8902, 1.0],
            inline_code: [0.1412, 0.1451, 0.1608, 1.0],
            code_bg: [0.9725, 0.9686, 0.9608, 1.0],
            code_block_bg: [0.9725, 0.9686, 0.9608, 1.0],
            toc_background: [0.98, 0.96, 0.93, 1.0],
            toc_active_background: [0.90, 0.85, 0.75, 0.5],
            toc_hover_background: [0.92, 0.90, 0.85, 0.5],
            toc_text: [0.1412, 0.1451, 0.1608, 1.0],
            toc_hover_text: [0.10, 0.10, 0.08, 1.0],
            toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
            spacing: MarkdownSpacing {
                paragraph_spacing_ratio: 0.5,
                heading_spacing_top_ratio: 1.0,
                heading_spacing_bottom_ratio: 0.45,
                list_item_spacing_ratio: 0.15,
                list_group_spacing_ratio: 0.5,
                list_indent_ratio: 2.0,
                code_block_padding_ratio: 0.8,
                code_line_height_ratio: 1.5,
                blockquote_padding_ratio: 0.65,
                table_cell_padding_ratio: 0.5,
                rule_spacing: 12.0,
                rule_thickness: 2.0,
                rule_width_ratio: 1.0,
                border_radius_base: 8.0,
                border_radius_small: 4.0,
            },
        },
        novel: NovelTheme {
            heading: [0.15, 0.15, 0.12, 1.0],
            quote: [0.8706, 0.4510, 0.3373, 1.0],
            toc_text: [0.1, 0.1, 0.08, 1.0],
            toc_hover_text: [0.1, 0.1, 0.08, 1.0],
            toc_background: [0.96, 0.95, 0.93, 1.0],
            toc_active_background: [0.8706, 0.4510, 0.3373, 0.08],
            spacing: MarkdownSpacing {
                paragraph_spacing_ratio: 0.5,
                heading_spacing_top_ratio: 1.3,
                heading_spacing_bottom_ratio: 0.55,
                list_item_spacing_ratio: 0.15,
                list_group_spacing_ratio: 0.5,
                list_indent_ratio: 2.0,
                code_block_padding_ratio: 0.8,
                code_line_height_ratio: 1.5,
                blockquote_padding_ratio: 0.65,
                table_cell_padding_ratio: 0.5,
                rule_spacing: 16.0,
                rule_thickness: 2.0,
                rule_width_ratio: 1.0,
                border_radius_base: 8.0,
                border_radius_small: 4.0,
            },
        },
        mindmap: MindmapTheme::default_light(),
        scopes: BTreeMap::from([
            ("comment".into(), [0.6353, 0.6392, 0.6549, 1.0]),
            ("string".into(), [0.3922, 0.6235, 0.3412, 1.0]),
            ("keyword.control".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("keyword.other".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("constant.numeric".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("constant.language".into(), [0.7569, 0.5176, 0.0039, 1.0]),
            ("variable".into(), [0.1412, 0.1451, 0.1608, 1.0]),
            ("variable.special".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("keyword.import".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("keyword.declaration".into(), [0.6431, 0.2863, 0.6706, 1.0]),
            ("boolean".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("method".into(), [0.3569, 0.4745, 0.8902, 1.0]),
            ("meta.header".into(), [0.3608, 0.4706, 0.8863, 1.0]),
            ("markup.heading".into(), [0.8275, 0.3765, 0.3098, 1.0]),
            ("markup.bold".into(), [0.6784, 0.4314, 0.1451, 1.0]),
            ("markup.italic".into(), [0.3608, 0.4706, 0.8863, 1.0]),
            ("markup.list".into(), [0.8275, 0.3765, 0.3098, 1.0]),
            ("markup.link".into(), [0.3569, 0.4745, 0.8902, 1.0]),
            ("markup.strikethrough".into(), [0.1412, 0.1451, 0.1608, 1.0]),
            ("markup.changed".into(), [0.7569, 0.5176, 0.0039, 1.0]),
            ("markup.deleted".into(), [0.8941, 0.3373, 0.2863, 1.0]),
            ("markup.inserted".into(), [0.3137, 0.6314, 0.3098, 1.0]),
            ("property".into(), [0.8275, 0.3765, 0.3098, 1.0]),
        ]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_accepts_immutable_registry() {
        let registry = ThemeRegistry::new();
        let pair = ActiveThemePair::default();
        let theme = Theme::resolve(
            crate::settings::ThemeMode::Dark,
            winit::window::Theme::Light,
            &pair,
            &registry,
        );
        assert!(theme.is_dark);
    }

    #[test]
    fn settings_tokens_are_derived_from_palette() {
        let theme = crate::theme::test_theme();
        let tokens = theme.settings_theme();

        assert_eq!(tokens.modal_surface, theme.palette.bg_elevated);
        assert_eq!(tokens.sidebar_surface, theme.palette.bg_surface);
        assert_eq!(tokens.section_surface, theme.palette.bg_elevated);
        assert_eq!(tokens.focus_ring, theme.palette.accent);
        assert_eq!(tokens.text_primary, theme.palette.text_main);
        assert_eq!(tokens.text_secondary, theme.palette.text_muted);
    }
}
