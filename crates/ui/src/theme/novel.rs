use super::MarkdownSpacing;

/// Novel reading theme — independent section, does not fall back to markdown.
/// Reuses `MarkdownSpacing` for shared spacing ratios.
#[derive(Debug, Clone)]
pub struct NovelTheme {
    // Colors (RGBA)
    pub heading: [f32; 4],
    pub quote: [f32; 4],
    pub toc_text: [f32; 4],
    pub toc_hover_text: [f32; 4],
    pub toc_background: [f32; 4],
    pub toc_active_background: [f32; 4],

    // Spacing
    pub spacing: MarkdownSpacing,
}

impl NovelTheme {
    pub(crate) fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.heading,
            &mut self.quote,
            &mut self.toc_text,
            &mut self.toc_hover_text,
            &mut self.toc_background,
            &mut self.toc_active_background,
        ] {
            for ch in c[..3].iter_mut() {
                *ch = ch.powf(gamma);
            }
        }
    }
}
