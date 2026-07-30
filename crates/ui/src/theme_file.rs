//! TOML-deserializable partial theme file.
//!
//! All fields are `Option` — only specified values override the base theme.
//! Use `ThemeFile::resolve(base)` to produce a concrete `ThemeDefinition`.

use crate::theme::{
    ColorPalette, EditorTheme, MarkdownSpacing, MarkdownTheme, MindmapNodeStyle, MindmapTheme,
    ThemeDefinition,
};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Top-level TOML structure for a theme file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFile {
    /// Base theme id to inherit from (e.g. "default-dark").
    /// If `None`, resolved at load time based on `is_dark`.
    pub extends: Option<String>,

    pub display_name: Option<String>,
    pub is_dark: Option<bool>,

    pub palette: Option<PaletteFile>,
    pub editor: Option<EditorFile>,
    pub markdown: Option<MarkdownFile>,
    pub mindmap: Option<MindmapFile>,

    /// Additional syntax/UI scopes (color overrides).
    #[serde(default)]
    pub scopes: BTreeMap<String, String>,
}

/// All-optional version of `ColorPalette`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PaletteFile {
    pub bg_base: Option<String>,
    pub bg_surface: Option<String>,
    pub bg_elevated: Option<String>,
    pub bg_hover: Option<String>,
    pub bg_active: Option<String>,
    pub text_main: Option<String>,
    pub text_muted: Option<String>,
    pub text_inverse: Option<String>,
    pub border_subtle: Option<String>,
    pub border_strong: Option<String>,
    pub shadow: Option<String>,
    pub accent: Option<String>,
    pub highlight: Option<String>,
    pub inactive_highlight: Option<String>,
    pub danger: Option<String>,
    pub warning: Option<String>,
    pub input_bg: Option<String>,
    pub input_border: Option<String>,
    pub input_fg: Option<String>,
    pub sidebar_hover_bg: Option<String>,
    pub sidebar_active_bg: Option<String>,
    pub sidebar_active_fg: Option<String>,
}

/// All-optional version of `EditorTheme`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorFile {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub gutter_bg: Option<String>,
    pub line_number: Option<String>,
    pub selection: Option<String>,
    pub cursor: Option<String>,
    pub scrollbar_track: Option<String>,
    pub scrollbar_thumb: Option<String>,
}

/// All-optional version of `MarkdownTheme`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarkdownFile {
    pub heading: Option<String>,
    pub link: Option<String>,
    pub inline_code: Option<String>,
    pub code_bg: Option<String>,
    pub code_block_bg: Option<String>,
    pub toc_background: Option<String>,
    pub toc_active_background: Option<String>,
    pub toc_hover_background: Option<String>,
    pub toc_text: Option<String>,
    pub toc_hover_text: Option<String>,
    pub toc_level_indicator: Option<String>,
    pub spacing: Option<MarkdownSpacingFile>,
}

/// All-optional version of `MarkdownSpacing`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarkdownSpacingFile {
    pub paragraph_spacing_ratio: Option<f32>,
    pub heading_spacing_top_ratio: Option<f32>,
    pub heading_spacing_bottom_ratio: Option<f32>,
    pub list_item_spacing_ratio: Option<f32>,
    pub list_group_spacing_ratio: Option<f32>,
    pub list_indent_ratio: Option<f32>,
    pub code_block_padding_ratio: Option<f32>,
    pub code_line_height_ratio: Option<f32>,
    pub blockquote_padding_ratio: Option<f32>,
    pub table_cell_padding_ratio: Option<f32>,
    pub rule_spacing: Option<f32>,
    pub rule_thickness: Option<f32>,
    pub rule_width_ratio: Option<f32>,
    pub border_radius_base: Option<f32>,
    pub border_radius_small: Option<f32>,
}

/// All-optional version of `MindmapTheme`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapFile {
    pub canvas: Option<MindmapCanvasFile>,
    pub node: Option<MindmapNodeThemeFile>,
    pub semantic: Option<MindmapSemanticFile>,
    pub geometry: Option<MindmapGeometryFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapCanvasFile {
    pub background: Option<String>,
    pub connector: Option<String>,
    pub connector_hover: Option<String>,
    pub selection: Option<String>,
    pub focus_ring: Option<String>,
    pub drag_invalid: Option<String>,
    /// Hex colors cycling over root child subtrees by branch index.
    pub branch_palette: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapNodeStyleFile {
    pub fill: Option<String>,
    pub border: Option<String>,
    pub text: Option<String>,
    pub accent: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapNodeThemeFile {
    pub default: Option<MindmapNodeStyleFile>,
    pub root: Option<MindmapNodeStyleFile>,
    pub depth: Option<Vec<MindmapNodeStyleFile>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapSemanticFile {
    pub status: Option<MindmapStatusFile>,
    pub priority: Option<MindmapPriorityFile>,
    #[serde(default)]
    pub named: BTreeMap<String, MindmapNodeStyleFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapStatusFile {
    pub todo: Option<MindmapNodeStyleFile>,
    pub doing: Option<MindmapNodeStyleFile>,
    pub done: Option<MindmapNodeStyleFile>,
    pub blocked: Option<MindmapNodeStyleFile>,
    pub canceled: Option<MindmapNodeStyleFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapPriorityFile {
    pub p0: Option<MindmapNodeStyleFile>,
    pub p1: Option<MindmapNodeStyleFile>,
    pub p2: Option<MindmapNodeStyleFile>,
    pub p3: Option<MindmapNodeStyleFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MindmapGeometryFile {
    pub card_height: Option<f32>,
    pub card_padding_x: Option<f32>,
    pub card_padding_y: Option<f32>,
    pub root_child_gap: Option<f32>,
    pub nested_child_gap: Option<f32>,
    pub sibling_gap: Option<f32>,
    pub card_radius: Option<f32>,
    pub connector_width: Option<f32>,
    pub selection_outline_width: Option<f32>,
    pub selection_outline_gap: Option<f32>,
    pub drag_source_alpha: Option<f32>,
    pub drag_preview_alpha: Option<f32>,
    pub same_level_threshold_ratio: Option<f32>,
    /// Per-depth font scale factors, index = depth (0 = root).
    pub depth_font_scales: Option<Vec<f32>>,
}

/// Error type for hex color resolution within ThemeFile.
#[derive(Debug, Clone)]
pub enum ResolveError {
    InvalidHex { field: String, value: String },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHex { field, value } => {
                write!(f, "invalid hex color in field '{}': \"{}\"", field, value)
            }
        }
    }
}

impl ThemeFile {
    /// Resolve this partial file onto a base `ThemeDefinition`, producing a
    /// complete definition with all fields filled in.
    pub fn resolve(&self, base: &ThemeDefinition) -> Result<ThemeDefinition, ResolveError> {
        let mut def = base.clone();

        if let Some(ref name) = self.display_name {
            def.display_name = name.clone();
        }
        if let Some(is_dark) = self.is_dark {
            def.is_dark = is_dark;
        }

        if let Some(ref p) = self.palette {
            resolve_palette(&mut def.palette, p)?;
        }
        if let Some(ref e) = self.editor {
            resolve_editor(&mut def.editor, e)?;
        }
        if let Some(ref m) = self.markdown {
            resolve_markdown(&mut def.markdown, m)?;
        }

        for (key, hex) in &self.scopes {
            let color = crate::hex_color::parse_hex(hex).map_err(|_| ResolveError::InvalidHex {
                field: format!("scopes.{}", key),
                value: hex.clone(),
            })?;
            def.scopes.insert(key.clone(), color);

            if key.starts_with("mindmap.") {
                map_legacy_mindmap_scope(&mut def.mindmap, key, color);
            }
        }

        if let Some(ref m) = self.mindmap {
            resolve_mindmap(&mut def.mindmap, m)?;
        }

        Ok(def)
    }
}

/// Parse a single hex color, tagging errors with the theme-file field path.
fn parse_hex_field(hex: &str, field: &str) -> Result<[f32; 4], ResolveError> {
    crate::hex_color::parse_hex(hex)
        .map_err(|_| ResolveError::InvalidHex { field: field.to_string(), value: hex.to_string() })
}

/// Resolve a single optional hex string, applying to target if set.
fn apply_color(
    target: &mut [f32; 4],
    src: &Option<String>,
    field: &str,
) -> Result<(), ResolveError> {
    if let Some(hex) = src {
        *target = parse_hex_field(hex, field)?;
    }
    Ok(())
}

fn resolve_palette(target: &mut ColorPalette, src: &PaletteFile) -> Result<(), ResolveError> {
    apply_color(&mut target.bg_base, &src.bg_base, "palette.bg_base")?;
    apply_color(&mut target.bg_surface, &src.bg_surface, "palette.bg_surface")?;
    apply_color(&mut target.bg_elevated, &src.bg_elevated, "palette.bg_elevated")?;
    apply_color(&mut target.bg_hover, &src.bg_hover, "palette.bg_hover")?;
    apply_color(&mut target.bg_active, &src.bg_active, "palette.bg_active")?;
    apply_color(&mut target.text_main, &src.text_main, "palette.text_main")?;
    apply_color(&mut target.text_muted, &src.text_muted, "palette.text_muted")?;
    apply_color(&mut target.text_inverse, &src.text_inverse, "palette.text_inverse")?;
    apply_color(&mut target.border_subtle, &src.border_subtle, "palette.border_subtle")?;
    apply_color(&mut target.border_strong, &src.border_strong, "palette.border_strong")?;
    apply_color(&mut target.shadow, &src.shadow, "palette.shadow")?;
    apply_color(&mut target.accent, &src.accent, "palette.accent")?;
    apply_color(&mut target.highlight, &src.highlight, "palette.highlight")?;
    apply_color(
        &mut target.inactive_highlight,
        &src.inactive_highlight,
        "palette.inactive_highlight",
    )?;
    apply_color(&mut target.danger, &src.danger, "palette.danger")?;
    apply_color(&mut target.warning, &src.warning, "palette.warning")?;
    apply_color(&mut target.input_bg, &src.input_bg, "palette.input_bg")?;
    apply_color(&mut target.input_border, &src.input_border, "palette.input_border")?;
    apply_color(&mut target.input_fg, &src.input_fg, "palette.input_fg")?;
    apply_color(&mut target.sidebar_hover_bg, &src.sidebar_hover_bg, "palette.sidebar_hover_bg")?;
    apply_color(
        &mut target.sidebar_active_bg,
        &src.sidebar_active_bg,
        "palette.sidebar_active_bg",
    )?;
    apply_color(
        &mut target.sidebar_active_fg,
        &src.sidebar_active_fg,
        "palette.sidebar_active_fg",
    )?;
    Ok(())
}

fn resolve_editor(target: &mut EditorTheme, src: &EditorFile) -> Result<(), ResolveError> {
    apply_color(&mut target.background, &src.background, "editor.background")?;
    apply_color(&mut target.foreground, &src.foreground, "editor.foreground")?;
    apply_color(&mut target.gutter_bg, &src.gutter_bg, "editor.gutter_bg")?;
    apply_color(&mut target.line_number, &src.line_number, "editor.line_number")?;
    apply_color(&mut target.selection, &src.selection, "editor.selection")?;
    apply_color(&mut target.cursor, &src.cursor, "editor.cursor")?;
    apply_color(&mut target.scrollbar_track, &src.scrollbar_track, "editor.scrollbar_track")?;
    apply_color(&mut target.scrollbar_thumb, &src.scrollbar_thumb, "editor.scrollbar_thumb")?;
    Ok(())
}

fn resolve_markdown(target: &mut MarkdownTheme, src: &MarkdownFile) -> Result<(), ResolveError> {
    apply_color(&mut target.heading, &src.heading, "markdown.heading")?;
    apply_color(&mut target.link, &src.link, "markdown.link")?;
    apply_color(&mut target.inline_code, &src.inline_code, "markdown.inline_code")?;
    apply_color(&mut target.code_bg, &src.code_bg, "markdown.code_bg")?;
    apply_color(&mut target.code_block_bg, &src.code_block_bg, "markdown.code_block_bg")?;
    apply_color(&mut target.toc_background, &src.toc_background, "markdown.toc_background")?;
    apply_color(
        &mut target.toc_active_background,
        &src.toc_active_background,
        "markdown.toc_active_background",
    )?;
    apply_color(
        &mut target.toc_hover_background,
        &src.toc_hover_background,
        "markdown.toc_hover_background",
    )?;
    apply_color(&mut target.toc_text, &src.toc_text, "markdown.toc_text")?;
    apply_color(&mut target.toc_hover_text, &src.toc_hover_text, "markdown.toc_hover_text")?;
    apply_color(
        &mut target.toc_level_indicator,
        &src.toc_level_indicator,
        "markdown.toc_level_indicator",
    )?;

    if let Some(ref sp) = src.spacing {
        resolve_spacing(&mut target.spacing, sp);
    }
    Ok(())
}

fn resolve_spacing(target: &mut MarkdownSpacing, src: &MarkdownSpacingFile) {
    macro_rules! apply_f32 {
        ($field:ident) => {
            if let Some(v) = src.$field {
                target.$field = v;
            }
        };
    }
    apply_f32!(paragraph_spacing_ratio);
    apply_f32!(heading_spacing_top_ratio);
    apply_f32!(heading_spacing_bottom_ratio);
    apply_f32!(list_item_spacing_ratio);
    apply_f32!(list_group_spacing_ratio);
    apply_f32!(list_indent_ratio);
    apply_f32!(code_block_padding_ratio);
    apply_f32!(code_line_height_ratio);
    apply_f32!(blockquote_padding_ratio);
    apply_f32!(table_cell_padding_ratio);
    apply_f32!(rule_spacing);
    apply_f32!(rule_thickness);
    apply_f32!(rule_width_ratio);
    apply_f32!(border_radius_base);
    apply_f32!(border_radius_small);
}

fn resolve_mindmap(target: &mut MindmapTheme, src: &MindmapFile) -> Result<(), ResolveError> {
    if let Some(ref c) = src.canvas {
        apply_color(&mut target.canvas.background, &c.background, "mindmap.canvas.background")?;
        apply_color(&mut target.canvas.connector, &c.connector, "mindmap.canvas.connector")?;
        apply_color(
            &mut target.canvas.connector_hover,
            &c.connector_hover,
            "mindmap.canvas.connector_hover",
        )?;
        apply_color(&mut target.canvas.selection, &c.selection, "mindmap.canvas.selection")?;
        apply_color(&mut target.canvas.focus_ring, &c.focus_ring, "mindmap.canvas.focus_ring")?;
        apply_color(
            &mut target.canvas.drag_invalid,
            &c.drag_invalid,
            "mindmap.canvas.drag_invalid",
        )?;
        if let Some(ref palette) = c.branch_palette {
            let mut resolved = Vec::with_capacity(palette.len());
            for (i, hex) in palette.iter().enumerate() {
                resolved
                    .push(parse_hex_field(hex, &format!("mindmap.canvas.branch_palette[{}]", i))?);
            }
            target.canvas.branch_palette = resolved;
        }
    }
    if let Some(ref n) = src.node {
        if let Some(ref d) = n.default {
            resolve_node_style(&mut target.node.default, d, "mindmap.node.default")?;
        }
        if let Some(ref r) = n.root {
            resolve_node_style(&mut target.node.root, r, "mindmap.node.root")?;
        }
        if let Some(ref depth_list) = n.depth {
            let mut new_depth = Vec::new();
            for (i, style_file) in depth_list.iter().enumerate() {
                let mut style = target.node.default.clone();
                resolve_node_style(&mut style, style_file, &format!("mindmap.node.depth[{}]", i))?;
                new_depth.push(style);
            }
            if !new_depth.is_empty() {
                target.node.depth = new_depth;
            }
        }
    }
    if let Some(ref s) = src.semantic {
        if let Some(ref st) = s.status {
            if let Some(ref v) = st.todo {
                resolve_node_style(
                    &mut target.semantic.status.todo,
                    v,
                    "mindmap.semantic.status.todo",
                )?;
            }
            if let Some(ref v) = st.doing {
                resolve_node_style(
                    &mut target.semantic.status.doing,
                    v,
                    "mindmap.semantic.status.doing",
                )?;
            }
            if let Some(ref v) = st.done {
                resolve_node_style(
                    &mut target.semantic.status.done,
                    v,
                    "mindmap.semantic.status.done",
                )?;
            }
            if let Some(ref v) = st.blocked {
                resolve_node_style(
                    &mut target.semantic.status.blocked,
                    v,
                    "mindmap.semantic.status.blocked",
                )?;
            }
            if let Some(ref v) = st.canceled {
                resolve_node_style(
                    &mut target.semantic.status.canceled,
                    v,
                    "mindmap.semantic.status.canceled",
                )?;
            }
        }
        if let Some(ref p) = s.priority {
            if let Some(ref v) = p.p0 {
                resolve_node_style(
                    &mut target.semantic.priority.p0,
                    v,
                    "mindmap.semantic.priority.p0",
                )?;
            }
            if let Some(ref v) = p.p1 {
                resolve_node_style(
                    &mut target.semantic.priority.p1,
                    v,
                    "mindmap.semantic.priority.p1",
                )?;
            }
            if let Some(ref v) = p.p2 {
                resolve_node_style(
                    &mut target.semantic.priority.p2,
                    v,
                    "mindmap.semantic.priority.p2",
                )?;
            }
            if let Some(ref v) = p.p3 {
                resolve_node_style(
                    &mut target.semantic.priority.p3,
                    v,
                    "mindmap.semantic.priority.p3",
                )?;
            }
        }
        for (name, style_file) in &s.named {
            let mut style = target
                .semantic
                .named
                .get(name)
                .cloned()
                .unwrap_or_else(|| target.node.default.clone());
            resolve_node_style(
                &mut style,
                style_file,
                &format!("mindmap.semantic.named.{}", name),
            )?;
            target.semantic.named.insert(name.clone(), style);
        }
    }
    if let Some(ref g) = src.geometry {
        apply_geometry_f32(&mut target.geometry.card_height, g.card_height, 1.0);
        apply_geometry_f32(&mut target.geometry.card_padding_x, g.card_padding_x, 0.0);
        apply_geometry_f32(&mut target.geometry.card_padding_y, g.card_padding_y, 0.0);
        apply_geometry_f32(&mut target.geometry.root_child_gap, g.root_child_gap, 0.0);
        apply_geometry_f32(&mut target.geometry.nested_child_gap, g.nested_child_gap, 0.0);
        apply_geometry_f32(&mut target.geometry.sibling_gap, g.sibling_gap, 0.0);
        apply_geometry_f32(&mut target.geometry.card_radius, g.card_radius, 0.0);
        apply_geometry_f32(&mut target.geometry.connector_width, g.connector_width, 0.1);
        apply_geometry_f32(
            &mut target.geometry.selection_outline_width,
            g.selection_outline_width,
            0.1,
        );
        apply_geometry_f32(
            &mut target.geometry.selection_outline_gap,
            g.selection_outline_gap,
            0.0,
        );
        apply_geometry_f32(&mut target.geometry.drag_source_alpha, g.drag_source_alpha, 0.0);
        apply_geometry_f32(&mut target.geometry.drag_preview_alpha, g.drag_preview_alpha, 0.0);
        apply_geometry_f32(
            &mut target.geometry.same_level_threshold_ratio,
            g.same_level_threshold_ratio,
            0.0,
        );
        if let Some(ref scales) = g.depth_font_scales {
            target.geometry.depth_font_scales = scales.clone();
        }
    }
    Ok(())
}

fn apply_geometry_f32(target: &mut f32, src: Option<f32>, min: f32) {
    if let Some(v) = src.filter(|v| *v >= min && v.is_finite()) {
        *target = v;
    }
}

fn map_legacy_mindmap_scope(mt: &mut MindmapTheme, key: &str, color: [f32; 4]) {
    match key {
        "mindmap.node_bg" => mt.node.default.fill = color,
        "mindmap.node_border" => mt.node.default.border = color,
        "mindmap.root_bg" => mt.node.root.fill = color,
        "mindmap.root_border" => mt.node.root.border = color,
        "mindmap.connector" => mt.canvas.connector = color,
        "mindmap.text" => mt.node.default.text = color,
        "mindmap.root_text" => mt.node.root.text = color,
        _ => {}
    }
}

fn resolve_node_style(
    target: &mut MindmapNodeStyle,
    src: &MindmapNodeStyleFile,
    prefix: &str,
) -> Result<(), ResolveError> {
    apply_color(&mut target.fill, &src.fill, &format!("{}.fill", prefix))?;
    apply_color(&mut target.border, &src.border, &format!("{}.border", prefix))?;
    apply_color(&mut target.text, &src.text, &format!("{}.text", prefix))?;
    apply_color(&mut target.accent, &src.accent, &format!("{}.accent", prefix))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_toml() {
        let toml_str = r##"
display_name = "My Theme"
is_dark = true
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.display_name.as_deref(), Some("My Theme"));
        assert_eq!(file.is_dark, Some(true));
        assert!(file.palette.is_none());
    }

    #[test]
    fn parse_palette_overrides() {
        let toml_str = r##"
[palette]
accent = "#FF6B6B"
bg_base = "#1E1E2E"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let p = file.palette.unwrap();
        assert_eq!(p.accent.as_deref(), Some("#FF6B6B"));
        assert_eq!(p.bg_base.as_deref(), Some("#1E1E2E"));
        assert!(p.text_main.is_none());
    }

    #[test]
    fn parse_editor_overrides() {
        let toml_str = r##"
[editor]
cursor = "#FF0000"
selection = "#74ADE83D"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let e = file.editor.unwrap();
        assert_eq!(e.cursor.as_deref(), Some("#FF0000"));
        assert_eq!(e.selection.as_deref(), Some("#74ADE83D"));
    }

    #[test]
    fn parse_scopes() {
        let toml_str = r##"
[scopes]
keyword = "#C678DD"
string = "#98C379"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.scopes.len(), 2);
        assert_eq!(file.scopes["keyword"], "#C678DD");
    }

    #[test]
    fn parse_empty_toml() {
        let file: ThemeFile = toml::from_str("").unwrap();
        assert!(file.display_name.is_none());
        assert!(file.palette.is_none());
    }

    #[test]
    fn reject_unknown_fields() {
        let toml_str = r##"
dispaly_name = "Typo"
"##;
        let err = toml::from_str::<ThemeFile>(toml_str).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn reject_unknown_palette_field() {
        let toml_str = r##"
[palette]
accent = "#FF0000"
unknown_field = "#000000"
"##;
        let err = toml::from_str::<ThemeFile>(toml_str).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn parse_markdown_spacing() {
        let toml_str = r##"
[markdown.spacing]
paragraph_spacing_ratio = 0.8
code_line_height_ratio = 1.3
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let sp = file.markdown.unwrap().spacing.unwrap();
        assert_eq!(sp.paragraph_spacing_ratio, Some(0.8));
        assert_eq!(sp.code_line_height_ratio, Some(1.3));
    }

    #[test]
    fn resolve_overrides_base() {
        let base = ThemeDefinition::default_dark();
        let original_accent = base.palette.accent;

        let toml_str = r##"
display_name = "Override"
[palette]
accent = "#FF0000"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_eq!(resolved.display_name, "Override");
        // accent should be overridden
        assert_ne!(resolved.palette.accent, original_accent);
        assert_eq!(resolved.palette.accent, [1.0, 0.0, 0.0, 1.0]);
        // non-overridden fields should stay from base
        assert_eq!(resolved.palette.bg_base, base.palette.bg_base);
    }

    #[test]
    fn resolve_invalid_hex_errors() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[palette]
accent = "not-a-color"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let err = file.resolve(&base).unwrap_err();
        assert!(err.to_string().contains("accent"));
    }

    #[test]
    fn resolve_scopes() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[scopes]
keyword = "#C678DD"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();
        let keyword = resolved.scopes.get("keyword").unwrap();
        assert!((keyword[0] - 0.776).abs() < 0.01);
    }

    #[test]
    fn resolve_mindmap_drag_feedback_overrides() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[mindmap.canvas]
drag_invalid = "#D94A4AFF"

[mindmap.geometry]
selection_outline_width = 3.0
same_level_threshold_ratio = 0.35
root_child_gap = 42.0
nested_child_gap = 18.0
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_eq!(
            resolved.mindmap.canvas.drag_invalid,
            [217.0 / 255.0, 74.0 / 255.0, 74.0 / 255.0, 1.0]
        );
        assert_eq!(resolved.mindmap.geometry.selection_outline_width, 3.0);
        assert_eq!(resolved.mindmap.geometry.same_level_threshold_ratio, 0.35);
        assert_eq!(resolved.mindmap.geometry.root_child_gap, 42.0);
        assert_eq!(resolved.mindmap.geometry.nested_child_gap, 18.0);
    }
}

// --- Additional resolve coverage tests ---

#[cfg(test)]
mod resolve_extra_tests {
    use super::*;

    #[test]
    fn resolve_editor_cursor() {
        let base = ThemeDefinition::default_dark();
        let original_cursor = base.editor.cursor;

        let toml_str = r##"
[editor]
cursor = "#00FF00"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_ne!(resolved.editor.cursor, original_cursor);
        assert_eq!(resolved.editor.cursor, [0.0, 1.0, 0.0, 1.0]);
        // Other editor fields unchanged
        assert_eq!(resolved.editor.background, base.editor.background);
    }

    #[test]
    fn resolve_editor_selection() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[editor]
selection = "#74ADE83D"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert!((resolved.editor.selection[3] - 0.2392).abs() < 0.01);
    }

    #[test]
    fn resolve_markdown_heading_color() {
        let base = ThemeDefinition::default_dark();
        let original_heading = base.markdown.heading;

        let toml_str = r##"
[markdown]
heading = "#FF0000"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_ne!(resolved.markdown.heading, original_heading);
        assert_eq!(resolved.markdown.heading, [1.0, 0.0, 0.0, 1.0]);
        // Other markdown fields unchanged
        assert_eq!(resolved.markdown.link, base.markdown.link);
    }

    #[test]
    fn resolve_markdown_spacing_override() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[markdown.spacing]
paragraph_spacing_ratio = 0.9
code_line_height_ratio = 1.2
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_eq!(resolved.markdown.spacing.paragraph_spacing_ratio, 0.9);
        assert_eq!(resolved.markdown.spacing.code_line_height_ratio, 1.2);
        // Non-overridden spacing fields unchanged
        assert_eq!(
            resolved.markdown.spacing.heading_spacing_top_ratio,
            base.markdown.spacing.heading_spacing_top_ratio
        );
    }

    #[test]
    fn resolve_is_dark_override() {
        let base = ThemeDefinition::default_dark();
        assert!(base.is_dark);

        let toml_str = r##"
is_dark = false
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();
        assert!(!resolved.is_dark);
    }

    #[test]
    fn resolve_full_override() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
display_name = "Full Override"
is_dark = false

[palette]
accent = "#FF0000"
bg_base = "#FFFFFF"

[editor]
cursor = "#00FF00"
background = "#FFFFFF"

[markdown]
heading = "#0000FF"
link = "#FF00FF"

[scopes]
keyword = "#C678DD"
string = "#98C379"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_eq!(resolved.display_name, "Full Override");
        assert!(!resolved.is_dark);
        assert_eq!(resolved.palette.accent, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(resolved.palette.bg_base, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(resolved.editor.cursor, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(resolved.editor.background, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(resolved.markdown.heading, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(resolved.markdown.link, [1.0, 0.0, 1.0, 1.0]);
        assert!(resolved.scopes.contains_key("keyword"));
        assert!(resolved.scopes.contains_key("string"));
    }

    #[test]
    fn resolve_lowercase_hex_in_toml() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[palette]
accent = "#ff6b6b"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();
        assert!((resolved.palette.accent[0] - 1.0).abs() < 0.01);
        assert!((resolved.palette.accent[1] - 0.42).abs() < 0.01);
    }

    #[test]
    fn resolve_editor_invalid_hex_errors() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[editor]
cursor = "not-a-color"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let err = file.resolve(&base).unwrap_err();
        assert!(err.to_string().contains("cursor"));
    }

    #[test]
    fn resolve_markdown_invalid_hex_errors() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[markdown]
heading = "invalid"
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let err = file.resolve(&base).unwrap_err();
        assert!(err.to_string().contains("heading"));
    }

    #[test]
    fn resolve_mindmap_branch_palette_and_depth_font_scales() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[mindmap.canvas]
branch_palette = ["#FF0000", "#00FF00"]

[mindmap.geometry]
depth_font_scales = [1.5, 1.2]
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_eq!(
            resolved.mindmap.canvas.branch_palette,
            vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]
        );
        assert_eq!(resolved.mindmap.geometry.depth_font_scales, vec![1.5, 1.2]);
        assert_eq!(resolved.mindmap.geometry.font_scale_for_depth(9), 1.2);
    }

    #[test]
    fn resolve_mindmap_branch_palette_invalid_hex_errors() {
        let base = ThemeDefinition::default_dark();
        let toml_str = r##"
[mindmap.canvas]
branch_palette = ["#FF0000", "not-a-color"]
"##;
        let file: ThemeFile = toml::from_str(toml_str).unwrap();
        let err = file.resolve(&base).unwrap_err();
        assert!(err.to_string().contains("branch_palette"));
    }

    #[test]
    fn resolve_mindmap_unset_palette_and_scales_keep_base() {
        let base = ThemeDefinition::default_dark();
        let file: ThemeFile = toml::from_str("").unwrap();
        let resolved = file.resolve(&base).unwrap();

        assert_eq!(resolved.mindmap.canvas.branch_palette, base.mindmap.canvas.branch_palette);
        assert_eq!(
            resolved.mindmap.geometry.depth_font_scales,
            base.mindmap.geometry.depth_font_scales
        );
    }
}
