import sys

with open('crates/app/src/app.rs', 'r') as f:
    content = f.read()

helper = """
    pub(crate) fn visible_rows(&self, screen_height: f32) -> usize {
        self.settings.visible_rows(screen_height, self.workspace.current_tab_bar_height(self.settings.dpi_scale))
    }

    pub(crate) fn visible_height_lines(&self, screen_height: f32) -> f64 {
        self.settings.visible_height_lines(screen_height, self.workspace.current_tab_bar_height(self.settings.dpi_scale))
    }
"""

if "pub(crate) fn visible_rows(" not in content:
    content = content.replace("impl App {", "impl App {" + helper)
    with open('crates/app/src/app.rs', 'w') as f:
        f.write(content)
