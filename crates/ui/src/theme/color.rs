#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SelectedControlColors {
    pub(crate) background: [f32; 4],
    pub(crate) foreground: [f32; 4],
}

/// Semantic design tokens for UI chrome — backgrounds, text, borders, accents.
#[derive(Debug, Clone)]
pub struct ColorPalette {
    // Backgrounds (5)
    pub bg_base: [f32; 4],
    pub bg_surface: [f32; 4],
    pub bg_elevated: [f32; 4],
    pub bg_hover: [f32; 4],
    pub bg_active: [f32; 4],

    // Text (3)
    pub text_main: [f32; 4],
    pub text_muted: [f32; 4],
    pub text_inverse: [f32; 4],

    // Borders & Shadow (3)
    pub border_subtle: [f32; 4],
    pub border_strong: [f32; 4],
    pub shadow: [f32; 4],

    // Accents & Feedback (11)
    pub accent: [f32; 4],
    pub on_accent: [f32; 4],
    pub(crate) selected_control: Option<SelectedControlColors>,
    pub accent_text: [f32; 4],
    pub highlight: [f32; 4],
    pub inactive_highlight: [f32; 4],
    pub danger: [f32; 4],
    pub danger_text: [f32; 4],
    pub warning: [f32; 4],
    pub warning_text: [f32; 4],
    pub focus_ring: [f32; 4],

    // Input (3)
    pub input_bg: [f32; 4],
    pub input_border: [f32; 4],
    pub input_fg: [f32; 4],

    // Sidebar (3)
    pub sidebar_hover_bg: [f32; 4],
    pub sidebar_active_bg: [f32; 4],
    pub sidebar_active_fg: [f32; 4],
}

impl ColorPalette {
    pub(crate) fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.bg_base,
            &mut self.bg_surface,
            &mut self.bg_elevated,
            &mut self.bg_hover,
            &mut self.bg_active,
            &mut self.text_main,
            &mut self.text_muted,
            &mut self.text_inverse,
            &mut self.border_subtle,
            &mut self.border_strong,
            &mut self.shadow,
            &mut self.accent,
            &mut self.on_accent,
            &mut self.accent_text,
            &mut self.highlight,
            &mut self.inactive_highlight,
            &mut self.danger,
            &mut self.danger_text,
            &mut self.warning,
            &mut self.warning_text,
            &mut self.focus_ring,
            &mut self.input_bg,
            &mut self.input_border,
            &mut self.input_fg,
            &mut self.sidebar_hover_bg,
            &mut self.sidebar_active_bg,
            &mut self.sidebar_active_fg,
        ]
        .into_iter()
        .chain(
            self.selected_control
                .iter_mut()
                .flat_map(|colors| [&mut colors.background, &mut colors.foreground]),
        ) {
            for ch in c[..3].iter_mut() {
                *ch = ch.powf(gamma);
            }
        }
    }
}
