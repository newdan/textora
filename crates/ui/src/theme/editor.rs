/// Editor-specific colors — independent from UI chrome palette.
#[derive(Debug, Clone)]
pub struct EditorTheme {
    pub background: [f32; 4],
    pub foreground: [f32; 4],
    pub gutter_bg: [f32; 4],
    pub line_number: [f32; 4],
    pub selection: [f32; 4],
    pub cursor: [f32; 4],
    pub scrollbar_track: [f32; 4],
    pub scrollbar_thumb: [f32; 4],
}

impl EditorTheme {
    pub(crate) fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.background,
            &mut self.foreground,
            &mut self.gutter_bg,
            &mut self.line_number,
            &mut self.cursor,
            &mut self.scrollbar_track,
            &mut self.scrollbar_thumb,
        ] {
            for ch in c[..3].iter_mut() {
                *ch = ch.powf(gamma);
            }
        }
        for ch in self.selection[..3].iter_mut() {
            *ch = ch.powf(gamma);
        }
    }
}
