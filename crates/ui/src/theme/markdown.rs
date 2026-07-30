/// Spacing and geometry tokens for markdown rendering.
#[derive(Debug, Clone)]
pub struct MarkdownSpacing {
    // Ratios multiplied by line_height in MarkdownStyle::from_theme()
    pub paragraph_spacing_ratio: f32,
    pub heading_spacing_top_ratio: f32,
    pub heading_spacing_bottom_ratio: f32,
    pub list_item_spacing_ratio: f32,
    pub list_group_spacing_ratio: f32,
    // Ratios multiplied by base_font_size
    pub list_indent_ratio: f32,
    // Ratios multiplied by code_font_size
    pub code_block_padding_ratio: f32,
    pub code_line_height_ratio: f32,
    // Ratios multiplied by base_font_size
    pub blockquote_padding_ratio: f32,
    pub table_cell_padding_ratio: f32,
    // Fixed constants (px)
    pub rule_spacing: f32,
    pub rule_thickness: f32,
    pub rule_width_ratio: f32,
    pub border_radius_base: f32,
    pub border_radius_small: f32,
}

/// Markdown rendering + TOC colors.
#[derive(Debug, Clone)]
pub struct MarkdownTheme {
    // Markup colors (5)
    pub heading: [f32; 4],
    pub link: [f32; 4],
    pub inline_code: [f32; 4],
    pub code_bg: [f32; 4],
    pub code_block_bg: [f32; 4],

    // TOC (6)
    pub toc_background: [f32; 4],
    pub toc_active_background: [f32; 4],
    pub toc_hover_background: [f32; 4],
    pub toc_text: [f32; 4],
    pub toc_hover_text: [f32; 4],
    pub toc_level_indicator: [f32; 4],

    // Spacing (15)
    pub spacing: MarkdownSpacing,
}

impl MarkdownTheme {
    pub(crate) fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.heading,
            &mut self.link,
            &mut self.inline_code,
            &mut self.code_bg,
            &mut self.code_block_bg,
            &mut self.toc_background,
            &mut self.toc_active_background,
            &mut self.toc_hover_background,
            &mut self.toc_text,
            &mut self.toc_hover_text,
            &mut self.toc_level_indicator,
        ] {
            for ch in c[..3].iter_mut() {
                *ch = ch.powf(gamma);
            }
        }
    }
}
