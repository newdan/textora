# Theme Refactor — Modular Design Tokens Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace flat 48-field `Theme` with modular `ColorPalette`/`EditorTheme`/`MarkdownTheme` and migrate all consumers.

**Architecture:** Three color sub-modules each owning their fields + gamma correction. `Theme` becomes a thin container. Consumer migration is a mechanical field-path rename plus markdown style derivation rewrite.

**Tech Stack:** Rust, no new dependencies.

---

### Task 1: Define ColorPalette struct

**Files:**
- Modify: `crates/ui/src/theme.rs:1-10`

- [ ] **Step 1: Add ColorPalette struct after the imports, before the Theme struct**

```rust
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

    // Accents & Feedback (4)
    pub accent: [f32; 4],
    pub highlight: [f32; 4],
    pub danger: [f32; 4],
    pub warning: [f32; 4],

    // Input (3)
    pub input_bg: [f32; 4],
    pub input_border: [f32; 4],
    pub input_fg: [f32; 4],
}
```

- [ ] **Step 2: Add ColorPalette::gamma_correct() impl**

```rust
impl ColorPalette {
    fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.bg_base, &mut self.bg_surface, &mut self.bg_elevated,
            &mut self.bg_hover, &mut self.bg_active,
            &mut self.text_main, &mut self.text_muted, &mut self.text_inverse,
            &mut self.border_subtle, &mut self.border_strong, &mut self.shadow,
            &mut self.accent, &mut self.highlight, &mut self.danger, &mut self.warning,
            &mut self.input_bg, &mut self.input_border, &mut self.input_fg,
        ] {
            for ch in c[..3].iter_mut() { *ch = ch.powf(gamma); }
        }
    }
}
```

- [ ] **Step 3: Run `cargo check -p edit-plus-ui` to verify compilation**

---

### Task 2: Define EditorTheme struct

**Files:**
- Modify: `crates/ui/src/theme.rs` (after ColorPalette)

- [ ] **Step 1: Add EditorTheme struct**

```rust
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
```

- [ ] **Step 2: Add EditorTheme::gamma_correct()**

```rust
impl EditorTheme {
    fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.background, &mut self.foreground, &mut self.gutter_bg,
            &mut self.line_number, &mut self.cursor,
            &mut self.scrollbar_track, &mut self.scrollbar_thumb,
        ] {
            for ch in c[..3].iter_mut() { *ch = ch.powf(gamma); }
        }
        for ch in self.selection[..3].iter_mut() { *ch = ch.powf(gamma); }
    }
}
```

- [ ] **Step 3: Run `cargo check -p edit-plus-ui`**

---

### Task 3: Define MarkdownTheme and MarkdownSpacing structs

**Files:**
- Modify: `crates/ui/src/theme.rs` (after EditorTheme)

- [ ] **Step 1: Add MarkdownSpacing struct**

```rust
/// Spacing and geometry tokens for markdown rendering.
#[derive(Debug, Clone)]
pub struct MarkdownSpacing {
    pub paragraph_spacing: f32,
    pub heading_spacing_top: f32,
    pub heading_spacing_bottom: f32,
    pub list_item_spacing: f32,
    pub list_group_spacing: f32,
    pub list_indent: f32,
    pub code_block_padding: f32,
    pub blockquote_padding: f32,
    pub table_cell_padding: f32,
    pub rule_spacing: f32,
    pub rule_thickness: f32,
    pub rule_width_ratio: f32,
    pub border_radius_base: f32,
    pub border_radius_small: f32,
    pub code_line_height: f32,
}
```

- [ ] **Step 2: Add MarkdownTheme struct**

```rust
/// Markdown rendering + TOC colors.
#[derive(Debug, Clone)]
pub struct MarkdownTheme {
    // Markup colors (5)
    pub heading: [f32; 4],
    pub link: [f32; 4],
    pub inline_code: [f32; 4],
    pub code_bg: [f32; 4],
    pub code_block_bg: [f32; 4],

    // TOC (4)
    pub toc_background: [f32; 4],
    pub toc_active_background: [f32; 4],
    pub toc_text: [f32; 4],
    pub toc_level_indicator: [f32; 4],

    // Spacing (15)
    pub spacing: MarkdownSpacing,
}
```

- [ ] **Step 3: Add MarkdownTheme::gamma_correct()**

```rust
impl MarkdownTheme {
    fn gamma_correct(&mut self) {
        let gamma = 2.2;
        for c in [
            &mut self.heading, &mut self.link, &mut self.inline_code,
            &mut self.code_bg, &mut self.code_block_bg,
            &mut self.toc_background, &mut self.toc_active_background,
            &mut self.toc_text, &mut self.toc_level_indicator,
        ] {
            for ch in c[..3].iter_mut() { *ch = ch.powf(gamma); }
        }
    }
}
```

- [ ] **Step 4: Run `cargo check -p edit-plus-ui`**

---

### Task 4: Rewrite Theme struct with new modular fields

**Files:**
- Modify: `crates/ui/src/theme.rs` (replace old Theme struct)

- [ ] **Step 1: Replace the Theme struct definition (lines 10-107)**

Replace the entire old `Theme` struct with:

```rust
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub scopes: HashMap<String, [f32; 4]>,
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui` — expect errors from old field references, that's fine for now**

---

### Task 5: Rewrite Theme::dark() with new structure

**Files:**
- Modify: `crates/ui/src/theme.rs` (replace old `fn dark()` body)

- [ ] **Step 1: Replace `Theme::dark()` with new modular version**

```rust
pub fn dark() -> Self {
    Self {
        name: "edit+ Dark".into(),
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
            highlight: [1.0, 0.65, 0.2, 0.75],
            danger: [0.8941, 0.3373, 0.2863, 1.0],
            warning: [0.8745, 0.7569, 0.5176, 1.0],
            input_bg: [0.188, 0.204, 0.224, 1.0],
            input_border: [0.133, 0.145, 0.161, 1.0],
            input_fg: [0.8627, 0.8784, 0.8980, 1.0],
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
            heading: [0.8157, 0.4471, 0.4667, 1.0],
            link: [0.4510, 0.6784, 0.9137, 1.0],
            inline_code: [0.6745, 0.6980, 0.7451, 1.0],
            code_bg: [0.03, 0.029, 0.028, 1.0],
            code_block_bg: [0.03, 0.029, 0.028, 1.0],
            toc_background: [0.28, 0.26, 0.23, 1.0],
            toc_active_background: [0.33, 0.29, 0.24, 0.5],
            toc_text: [0.6745, 0.6980, 0.7451, 1.0],
            toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
            spacing: MarkdownSpacing {
                paragraph_spacing: 12.0,
                heading_spacing_top: 24.0,
                heading_spacing_bottom: 6.75,
                list_item_spacing: 3.6,
                list_group_spacing: 12.0,
                list_indent: 30.0,
                code_block_padding: 12.0,
                blockquote_padding: 9.75,
                table_cell_padding: 7.5,
                rule_spacing: 12.0,
                rule_thickness: 2.0,
                rule_width_ratio: 1.0,
                border_radius_base: 8.0,
                border_radius_small: 4.0,
                code_line_height: 22.5,
            },
        },
        scopes: HashMap::from([
            ("comment".into(),            [0.3647, 0.3882, 0.4353, 1.0]),
            ("string".into(),             [0.6314, 0.7569, 0.5059, 1.0]),
            ("keyword.control".into(),    [0.7059, 0.4667, 0.8118, 1.0]),
            ("keyword.other".into(),      [0.7059, 0.4667, 0.8118, 1.0]),
            ("constant.numeric".into(),   [0.7490, 0.5843, 0.4157, 1.0]),
            ("constant.language".into(),  [0.8745, 0.7569, 0.5176, 1.0]),
            ("variable".into(),           [0.6745, 0.6980, 0.7451, 1.0]),
            ("variable.special".into(),   [0.7490, 0.5843, 0.4157, 1.0]),
            ("keyword.import".into(),     [0.7059, 0.4667, 0.8118, 1.0]),
            ("keyword.declaration".into(),[0.7059, 0.4667, 0.8118, 1.0]),
            ("boolean".into(),            [0.7490, 0.5843, 0.4157, 1.0]),
            ("method".into(),             [0.4510, 0.6784, 0.9137, 1.0]),
            ("meta.header".into(),        [0.4549, 0.6784, 0.9098, 1.0]),
            ("markup.heading".into(),     [0.8157, 0.4471, 0.4667, 1.0]),
            ("markup.bold".into(),        [0.7490, 0.5843, 0.4157, 1.0]),
            ("markup.italic".into(),      [0.4549, 0.6784, 0.9098, 1.0]),
            ("markup.list".into(),        [0.8157, 0.4471, 0.4667, 1.0]),
            ("markup.link".into(),        [0.4510, 0.6784, 0.9137, 1.0]),
            ("markup.strikethrough".into(), [0.6745, 0.6980, 0.7451, 1.0]),
            ("markup.changed".into(),     [0.8745, 0.7569, 0.5176, 1.0]),
            ("markup.deleted".into(),     [0.8784, 0.4235, 0.4588, 1.0]),
            ("markup.inserted".into(),    [0.5961, 0.7647, 0.4745, 1.0]),
            ("property".into(),           [0.8157, 0.4471, 0.4667, 1.0]),
        ]),
    }
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`**

---

### Task 6: Rewrite Theme::light() with new structure

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Replace `Theme::light()` with new modular version**

```rust
pub fn light() -> Self {
    Self {
        name: "edit+ Light".into(),
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
            border_subtle: [0.816, 0.816, 0.824, 1.0],
            border_strong: [0.816, 0.816, 0.824, 1.0],
            shadow: [0.0, 0.0, 0.0, 0.08],
            accent: [0.4549, 0.6784, 0.9098, 1.0],
            highlight: [1.0, 0.55, 0.15, 0.7],
            danger: [0.8941, 0.3373, 0.2863, 1.0],
            warning: [0.7569, 0.5176, 0.0039, 1.0],
            input_bg: [0.906, 0.906, 0.914, 1.0],
            input_border: [0.816, 0.816, 0.824, 1.0],
            input_fg: [0.1412, 0.1451, 0.1608, 1.0],
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
            heading: [0.8275, 0.3765, 0.3098, 1.0],
            link: [0.3569, 0.4745, 0.8902, 1.0],
            inline_code: [0.1412, 0.1451, 0.1608, 1.0],
            code_bg: [0.9725, 0.9686, 0.9608, 1.0],
            code_block_bg: [0.9725, 0.9686, 0.9608, 1.0],
            toc_background: [0.98, 0.96, 0.93, 1.0],
            toc_active_background: [0.90, 0.85, 0.75, 0.5],
            toc_text: [0.1412, 0.1451, 0.1608, 1.0],
            toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
            spacing: MarkdownSpacing {
                paragraph_spacing: 12.0,
                heading_spacing_top: 24.0,
                heading_spacing_bottom: 6.75,
                list_item_spacing: 3.6,
                list_group_spacing: 12.0,
                list_indent: 30.0,
                code_block_padding: 12.0,
                blockquote_padding: 9.75,
                table_cell_padding: 7.5,
                rule_spacing: 12.0,
                rule_thickness: 2.0,
                rule_width_ratio: 1.0,
                border_radius_base: 8.0,
                border_radius_small: 4.0,
                code_line_height: 22.5,
            },
        },
        scopes: HashMap::from([
            ("comment".into(),            [0.6353, 0.6392, 0.6549, 1.0]),
            ("string".into(),             [0.3922, 0.6235, 0.3412, 1.0]),
            ("keyword.control".into(),    [0.6431, 0.2863, 0.6706, 1.0]),
            ("keyword.other".into(),      [0.6431, 0.2863, 0.6706, 1.0]),
            ("constant.numeric".into(),   [0.6784, 0.4314, 0.1451, 1.0]),
            ("constant.language".into(),  [0.7569, 0.5176, 0.0039, 1.0]),
            ("variable".into(),           [0.1412, 0.1451, 0.1608, 1.0]),
            ("variable.special".into(),   [0.6784, 0.4314, 0.1451, 1.0]),
            ("keyword.import".into(),     [0.6431, 0.2863, 0.6706, 1.0]),
            ("keyword.declaration".into(),[0.6431, 0.2863, 0.6706, 1.0]),
            ("boolean".into(),            [0.6784, 0.4314, 0.1451, 1.0]),
            ("method".into(),             [0.3569, 0.4745, 0.8902, 1.0]),
            ("meta.header".into(),        [0.3608, 0.4706, 0.8863, 1.0]),
            ("markup.heading".into(),     [0.8275, 0.3765, 0.3098, 1.0]),
            ("markup.bold".into(),        [0.6784, 0.4314, 0.1451, 1.0]),
            ("markup.italic".into(),      [0.3608, 0.4706, 0.8863, 1.0]),
            ("markup.list".into(),        [0.8275, 0.3765, 0.3098, 1.0]),
            ("markup.link".into(),        [0.3569, 0.4745, 0.8902, 1.0]),
            ("markup.strikethrough".into(), [0.1412, 0.1451, 0.1608, 1.0]),
            ("markup.changed".into(),     [0.7569, 0.5176, 0.0039, 1.0]),
            ("markup.deleted".into(),     [0.8941, 0.3373, 0.2863, 1.0]),
            ("markup.inserted".into(),    [0.3137, 0.6314, 0.3098, 1.0]),
            ("property".into(),           [0.8275, 0.3765, 0.3098, 1.0]),
        ]),
    }
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`**

---

### Task 7: Rewrite Theme::claude_light()

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Replace `Theme::claude_light()` — starts from `Self::light()`, overrides with warm palette**

```rust
pub fn claude_light() -> Self {
    let mut t = Self::light();
    t.name = "Claude Light".into();

    let palette = &mut t.palette;
    palette.bg_base = [1.0, 1.0, 1.0, 1.0];
    palette.bg_surface = [0.9529, 0.9490, 0.9333, 1.0];
    palette.bg_elevated = [0.9765, 0.9745, 0.9667, 1.0];
    palette.bg_hover = [0.898, 0.886, 0.867, 1.0];
    palette.bg_active = [0.843, 0.831, 0.812, 1.0];
    palette.text_main = [0.1, 0.1, 0.08, 1.0];
    palette.text_muted = [0.45, 0.45, 0.43, 1.0];
    palette.text_inverse = [0.95, 0.95, 0.95, 1.0];
    palette.border_subtle = [0.847, 0.843, 0.827, 1.0];
    palette.border_strong = [0.847, 0.843, 0.827, 1.0];
    palette.shadow = [0.0, 0.0, 0.0, 0.1];
    palette.accent = [0.8706, 0.4510, 0.3373, 1.0];
    palette.input_bg = [0.9529, 0.9490, 0.9333, 1.0];
    palette.input_border = [0.9137, 0.9020, 0.8824, 1.0];
    palette.input_fg = [0.1, 0.1, 0.08, 1.0];

    let editor = &mut t.editor;
    editor.background = [1.0, 1.0, 1.0, 1.0];
    editor.gutter_bg = [1.0, 1.0, 1.0, 1.0];
    editor.line_number = [0.45, 0.45, 0.43, 1.0];
    editor.selection = [235.0/255.0, 235.0/255.0, 235.0/255.0, 1.0];
    editor.cursor = [0.1137, 0.0392, 0.0431, 1.0];
    editor.foreground = [0.1, 0.1, 0.08, 1.0];

    let md = &mut t.markdown;
    md.toc_background = [0.98, 0.96, 0.93, 1.0];
    md.toc_active_background = [0.8706, 0.4510, 0.3373, 0.12];
    md.toc_text = [0.1, 0.1, 0.08, 1.0];

    t
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`**

---

### Task 8: Rewrite Theme::claude_dark()

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Replace `Theme::claude_dark()` — starts from `Self::dark()`, overrides with warm palette**

```rust
pub fn claude_dark() -> Self {
    let mut t = Self::dark();
    t.name = "Claude Dark".into();

    let palette = &mut t.palette;
    palette.bg_base = [0.04, 0.038, 0.036, 1.0];
    palette.bg_surface = [0.055, 0.053, 0.051, 1.0];
    palette.bg_elevated = [0.07, 0.068, 0.066, 1.0];
    palette.bg_hover = [0.08, 0.078, 0.076, 1.0];
    palette.bg_active = [0.10, 0.098, 0.096, 1.0];
    palette.text_main = [0.9608, 0.9529, 0.9412, 1.0];
    palette.text_muted = [0.65, 0.64, 0.62, 1.0];
    palette.text_inverse = [0.9, 0.9, 0.9, 1.0];
    palette.border_subtle = [0.05, 0.048, 0.046, 1.0];
    palette.border_strong = [0.05, 0.048, 0.046, 1.0];
    palette.shadow = [0.0, 0.0, 0.0, 0.5];
    palette.accent = [0.8706, 0.4510, 0.3373, 1.0];
    palette.input_bg = [0.055, 0.053, 0.051, 1.0];
    palette.input_border = [0.08, 0.078, 0.076, 1.0];
    palette.input_fg = [0.9608, 0.9529, 0.9412, 1.0];

    let editor = &mut t.editor;
    editor.background = [0.04, 0.038, 0.036, 1.0];
    editor.gutter_bg = [0.04, 0.038, 0.036, 1.0];
    editor.line_number = [0.65, 0.64, 0.62, 1.0];
    editor.selection = [0.10, 0.10, 0.10, 1.0];
    editor.cursor = [0.8706, 0.4510, 0.3373, 1.0];
    editor.foreground = [0.9608, 0.9529, 0.9412, 1.0];

    let md = &mut t.markdown;
    md.toc_background = [0.08, 0.078, 0.076, 1.0];
    md.toc_active_background = [0.8706, 0.4510, 0.3373, 0.12];
    md.toc_text = [0.9608, 0.9529, 0.9412, 1.0];

    t
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`**

---

### Task 9: Integrate gamma_correct, from_winit, resolve, scope_color, remove old TOC accessors

**Files:**
- Modify: `crates/ui/src/theme.rs`

- [ ] **Step 1: Replace standalone `gamma_correct()` with module-delegating version**

```rust
impl Theme {
    fn gamma_correct(&mut self) {
        self.palette.gamma_correct();
        self.editor.gamma_correct();
        self.markdown.gamma_correct();
        let gamma = 2.2;
        for color in self.scopes.values_mut() {
            for ch in color[..3].iter_mut() { *ch = ch.powf(gamma); }
        }
    }
}
```

- [ ] **Step 2: Update `from_winit()` — same logic, new struct shape**

```rust
pub fn from_winit(theme: winit::window::Theme) -> Self {
    let mut t = match theme {
        winit::window::Theme::Dark => Self::claude_dark(),
        winit::window::Theme::Light => Self::claude_light(),
    };
    t.gamma_correct();
    t
}
```

- [ ] **Step 3: Update `resolve()` — same match, new struct shape**

```rust
pub fn resolve(mode: crate::settings::ThemeMode, system_theme: winit::window::Theme) -> Self {
    let mut t = match mode {
        crate::settings::ThemeMode::System => match system_theme {
            winit::window::Theme::Dark => Self::claude_dark(),
            winit::window::Theme::Light => Self::claude_light(),
        },
        crate::settings::ThemeMode::Dark => Self::claude_dark(),
        crate::settings::ThemeMode::Light => Self::claude_light(),
        crate::settings::ThemeMode::ClaudeLight => Self::claude_light(),
        crate::settings::ThemeMode::ClaudeDark => Self::claude_dark(),
    };
    t.gamma_correct();
    t
}
```

- [ ] **Step 4: Keep `scope_color()` — unchanged**

```rust
pub fn scope_color(&self, name: &str) -> [f32; 4] {
    self.scopes
        .get(name)
        .copied()
        .unwrap_or(self.editor.foreground)
}
```

- [ ] **Step 5: Remove the 9 TOC accessor methods (old lines 428-436)**

Delete all 9 `pub fn toc_*(&self)` wrapper methods — consumers will access `theme.markdown.toc_*` directly.

- [ ] **Step 6: Run `cargo check -p edit-plus-ui`**

---

### Task 10: Update test_theme() function

**Files:**
- Modify: `crates/ui/src/theme.rs` (lines 447-500)

- [ ] **Step 1: Replace `test_theme()` with new structure**

```rust
pub fn test_theme() -> Theme {
    Theme {
        name: "test".into(),
        is_dark: true,
        palette: ColorPalette {
            bg_base: [0.0; 4], bg_surface: [0.0; 4], bg_elevated: [0.0; 4],
            bg_hover: [0.0; 4], bg_active: [0.0; 4],
            text_main: [0.0; 4], text_muted: [0.0; 4], text_inverse: [0.0; 4],
            border_subtle: [0.0; 4], border_strong: [0.0; 4], shadow: [0.0; 4],
            accent: [0.0; 4], highlight: [0.0; 4], danger: [0.0; 4], warning: [0.0; 4],
            input_bg: [0.0; 4], input_border: [0.0; 4], input_fg: [0.0; 4],
        },
        editor: EditorTheme {
            background: [0.0; 4], foreground: [0.0; 4], gutter_bg: [0.0; 4],
            line_number: [0.0; 4], selection: [0.0; 4], cursor: [0.0; 4],
            scrollbar_track: [0.0; 4], scrollbar_thumb: [0.0; 4],
        },
        markdown: MarkdownTheme {
            heading: [0.0; 4], link: [0.0; 4], inline_code: [0.0; 4],
            code_bg: [0.0; 4], code_block_bg: [0.0; 4],
            toc_background: [0.0; 4], toc_active_background: [0.0; 4],
            toc_text: [0.0; 4], toc_level_indicator: [0.0; 4],
            spacing: MarkdownSpacing {
                paragraph_spacing: 0.0, heading_spacing_top: 0.0,
                heading_spacing_bottom: 0.0, list_item_spacing: 0.0,
                list_group_spacing: 0.0, list_indent: 0.0,
                code_block_padding: 0.0, blockquote_padding: 0.0,
                table_cell_padding: 0.0, rule_spacing: 0.0,
                rule_thickness: 0.0, rule_width_ratio: 0.0,
                border_radius_base: 0.0, border_radius_small: 0.0,
                code_line_height: 0.0,
            },
        },
        scopes: HashMap::new(),
    }
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-ui`**

---

### Task 11: Update existing tests in theme.rs

**Files:**
- Modify: `crates/ui/src/theme.rs` (tests module, lines 501-577)

- [ ] **Step 1: Fix `scope_color_falls_back_to_foreground` — change `t.foreground` to `t.editor.foreground`**

```rust
#[test]
fn scope_color_falls_back_to_foreground() {
    let t = Theme::dark();
    assert_eq!(t.scope_color("nonexistent_scope"), t.editor.foreground);
}
```

- [ ] **Step 2: Fix `dark_and_light_have_different_backgrounds` — change `dark.background` to `dark.editor.background`**

```rust
#[test]
fn dark_and_light_have_different_backgrounds() {
    let dark = Theme::dark();
    let light = Theme::light();
    assert_ne!(dark.editor.background, light.editor.background);
}
```

- [ ] **Step 3: Fix `resolve_applies_gamma_correction` — change `t.background` to `t.editor.background`**

```rust
#[test]
fn resolve_applies_gamma_correction() {
    let t = Theme::resolve(crate::settings::ThemeMode::Dark, winit::window::Theme::Dark);
    let raw = Theme::claude_dark();
    assert_ne!(t.editor.background, raw.editor.background);
}
```

- [ ] **Step 4: Run `cargo test -p edit-plus-ui` — all tests in theme.rs should pass**

Expected: 7 tests pass (dark_theme_has_dark_flag, light_theme_has_light_flag, scope_color_*, dark_and_light_*, resolve_*)

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/theme.rs
git commit -m "refactor(ui): modular Theme struct with ColorPalette/EditorTheme/MarkdownTheme

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 12: Migrate Sidebar widgets to new token paths

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/mod.rs`
- Modify: `crates/ui/src/widgets/sidebar/state.rs`
- Modify: `crates/ui/src/widgets/sidebar/types.rs`

- [ ] **Step 1: In `state.rs`, batch-replace field accesses**

Find-replace across the file:
```
theme.sidebar_bg            → theme.palette.bg_surface
theme.sidebar_header_bg     → theme.palette.bg_surface
theme.sidebar_item_active_bg → theme.palette.bg_active
theme.sidebar_item_hover_bg  → theme.palette.bg_hover
theme.sidebar_item_fg        → theme.palette.text_muted
theme.sidebar_item_active_fg → theme.palette.text_main
theme.sidebar_accent         → theme.palette.accent
theme.sidebar_border         → theme.palette.border_subtle
theme.menu_separator         → theme.palette.border_strong
theme.foreground             → theme.editor.foreground
theme.background             → theme.editor.background
```

- [ ] **Step 2: In `mod.rs`, same replacements**

```
theme.sidebar_bg      → theme.palette.bg_surface
theme.sidebar_border  → theme.palette.border_subtle
theme.foreground      → theme.palette.accent   (used for indicator color)
theme.background      → theme.editor.background
```

- [ ] **Step 3: In `types.rs`, update any ThemeMode references — keep ThemeMode enum unchanged for now**

- [ ] **Step 4: Run `cargo check -p edit-plus-ui`**

---

### Task 13: Migrate remaining UI widgets

**Files:**
- Modify: `crates/ui/src/widgets/status_bar.rs`
- Modify: `crates/ui/src/widgets/tab_bar/state.rs`
- Modify: `crates/ui/src/widgets/popup_menu/mod.rs`
- Modify: `crates/ui/src/widgets/popup_menu/types.rs`
- Modify: `crates/ui/src/widgets/tooltip.rs`
- Modify: `crates/ui/src/widgets/scrollbar.rs`
- Modify: `crates/ui/src/widgets/toc.rs`
- Modify: `crates/ui/src/widgets/search_bar.rs`
- Modify: `crates/ui/src/widgets/text_box.rs`
- Modify: `crates/ui/src/widgets/title_bar.rs`
- Modify: `crates/ui/src/gutter.rs`
- Modify: `crates/ui/src/decorations.rs`

- [ ] **Step 1: `status_bar.rs` — replace `theme.status_bar_bg` → `theme.palette.bg_surface`, `theme.status_bar_fg` → `theme.palette.text_muted`**

- [ ] **Step 2: `tab_bar/state.rs` — replace `theme.gutter_bg` → `theme.palette.bg_surface` (tab bar uses darken_color on this), `theme.foreground` → `theme.editor.foreground`, `theme.background` → `theme.editor.background`, `theme.cursor` → `theme.editor.cursor`**

- [ ] **Step 3: `popup_menu/mod.rs` — replace menu field accesses:**

```
theme.menu_bg        → theme.palette.bg_elevated
theme.menu_border    → theme.palette.border_strong
theme.menu_hover     → theme.palette.bg_hover
theme.menu_selected  → theme.palette.bg_active
theme.menu_separator → theme.palette.border_strong
theme.menu_shadow    → theme.palette.shadow
theme.menu_text      → theme.palette.text_main
```

- [ ] **Step 4: `popup_menu/types.rs` — same menu field replacements**

- [ ] **Step 5: `tooltip.rs` — replace:**

```
theme.tooltip_bg     → theme.palette.bg_elevated
theme.tooltip_fg     → theme.palette.text_inverse
theme.tooltip_border → theme.palette.border_strong
```

- [ ] **Step 6: `scrollbar.rs` — replace `theme.scrollbar_track` → `theme.editor.scrollbar_track`, `theme.scrollbar_thumb` → `theme.editor.scrollbar_thumb`**

- [ ] **Step 7: `toc.rs` — replace TOC field accesses:**

```
theme.toc_background()         → theme.markdown.toc_background
theme.toc_border()             → theme.palette.border_strong
theme.toc_active_background()  → theme.markdown.toc_active_background
theme.toc_hover_background()   → theme.markdown.toc_active_background
theme.toc_text_color()         → theme.markdown.toc_text
theme.toc_active_text_color()  → theme.markdown.toc_text
theme.toc_hover_text_color()   → theme.markdown.toc_text
theme.toc_empty_text_color()   → theme.palette.text_muted
theme.toc_level_indicator()    → theme.markdown.toc_level_indicator
```

- [ ] **Step 8: `search_bar.rs` — replace:**

```
theme.search_bar_bg             → theme.palette.input_bg
theme.search_bar_fg             → theme.palette.input_fg
theme.search_bar_border         → theme.palette.input_border
theme.search_bar_no_results_fg  → theme.palette.danger
theme.menu_hover                → theme.palette.bg_hover
theme.sidebar_accent            → theme.palette.accent
```

- [ ] **Step 9: `text_box.rs` — replace:**

```
theme.search_bar_bg      → theme.palette.input_bg
theme.search_bar_fg      → theme.palette.input_fg
theme.search_bar_border  → theme.palette.input_border
theme.sidebar_accent     → theme.palette.accent
theme.selection          → theme.editor.selection
theme.menu_hover         → theme.palette.bg_hover
```

- [ ] **Step 10: `title_bar.rs` — replace:**

```
theme.background        → theme.editor.background
theme.sidebar_item_fg   → theme.palette.text_muted
theme.sidebar_accent    → theme.palette.accent
theme.sidebar_border    → theme.palette.border_subtle
```

- [ ] **Step 11: `gutter.rs` — replace `theme.foreground` → `theme.editor.foreground`, `theme.line_number` → `theme.editor.line_number`**

- [ ] **Step 12: `decorations.rs` — replace:**

```
theme.selection               → theme.editor.selection
theme.cursor                  → theme.editor.cursor
theme.foreground              → theme.editor.foreground
theme.search_match_active     → theme.palette.highlight (use full alpha)
theme.search_match_inactive   → theme.palette.highlight (multiply alpha by 0.5)
theme.scope_color(...)        → theme.scope_color(...) (unchanged)
```

- [ ] **Step 13: Run `cargo check -p edit-plus-ui`**

---

### Task 14: Fix test helper functions that construct Theme

**Files:**
- Modify: `crates/ui/src/widgets/scrollbar.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/status_bar.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/title_bar.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/sidebar/widget_tests.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/tab_bar/widget.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/button.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/popup_menu/mod.rs` (test_theme helper)
- Modify: `crates/ui/src/widgets/toc.rs` (inline test_theme calls)
- Modify: `crates/ui/src/widgets/search_bar.rs` (inline test_theme calls)
- Modify: `crates/ui/src/widgets/text_box.rs` (inline test_theme calls)

- [ ] **Step 1: Update each local `fn test_theme()` that mutates the base `test_theme()` to use new field paths**

For each file's local test_theme helper, replace the old flat field assignments with new modular paths. Example for scrollbar.rs:

```rust
fn test_theme() -> Theme {
    let mut t = crate::theme::test_theme();
    t.editor.scrollbar_track = [0.0, 0.0, 0.0, 1.0];
    t.editor.scrollbar_thumb = [0.0, 0.0, 0.0, 1.0];
    t
}
```

For status_bar.rs:
```rust
fn test_theme() -> Theme {
    let mut t = crate::theme::test_theme();
    t.palette.bg_surface = [0.2, 0.2, 0.2, 1.0];
    t.palette.text_muted = [0.9, 0.9, 0.9, 1.0];
    t
}
```

For sidebar/widget_tests.rs:
```rust
fn test_theme() -> Theme {
    let mut t = crate::theme::test_theme();
    t.palette.bg_surface = [0.188, 0.204, 0.224, 1.0];
    t.palette.text_muted = [0.65, 0.65, 0.65, 1.0];
    t.palette.text_main = [0.8627, 0.8784, 0.8980, 1.0];
    t.palette.bg_hover = [1.0, 1.0, 1.0, 0.05];
    t.palette.bg_active = [1.0, 1.0, 1.0, 0.10];
    t.palette.accent = [0.4549, 0.6784, 0.9098, 1.0];
    t.palette.border_subtle = [0.133, 0.145, 0.161, 1.0];
    t.palette.border_strong = [0.2118, 0.2353, 0.2745, 0.6];
    t.editor.foreground = [0.6745, 0.6980, 0.7451, 1.0];
    t.editor.background = [0.1569, 0.1725, 0.2000, 1.0];
    t.is_dark = true;
    t
}
```

Apply the same pattern to all files. The key principle: every old flat field path maps to the corresponding new modular path per the design doc migration table.

- [ ] **Step 2: Run `cargo test -p edit-plus-ui`**

---

### Task 15: Migrate app crate consumers

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/render_cache.rs`
- Modify: `crates/app/src/render_pipeline.rs`
- Modify: `crates/app/src/render_state.rs`
- Modify: `crates/app/src/md_preview.rs`
- Modify: `crates/app/src/native_menu.rs`

- [ ] **Step 1: `app_renderer.rs` — replace `theme.gutter_bg` → `theme.editor.gutter_bg`, `theme.background` → `theme.editor.background`, `theme.foreground` → `theme.editor.foreground`**

- [ ] **Step 2: `ui_shell.rs` — replace `sidebar_border` → `theme.palette.border_subtle`**

- [ ] **Step 3: `render_cache.rs` — `theme.scope_color()` unchanged, `theme.foreground` → `theme.editor.foreground`**

- [ ] **Step 4: `render_pipeline.rs` — replace any theme.background → theme.editor.background**

- [ ] **Step 5: `render_state.rs` — same pattern**

- [ ] **Step 6: `md_preview.rs` — `theme.scope_color()` unchanged**

- [ ] **Step 7: `native_menu.rs` — likely no Theme struct references (uses macOS NSMenu theme), verify**

- [ ] **Step 8: Update test helpers in `ui_shell.rs` — same pattern as Task 14**

- [ ] **Step 9: Run `cargo check -p edit-plus-app`**

---

### Task 16: Migrate markdown crate consumer

**Files:**
- Modify: `crates/markdown/src/style.rs`

- [ ] **Step 1: Rewrite `MarkdownStyle::from_theme()` to use new token paths**

Replace the entire function body:

```rust
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
    let code_block_bg = theme.markdown.code_block_bg;
    let inline_code_bg = blend_toward_bg(code_bg, bg, 0.94);

    let blockquote_border = if is_dark {
        blend_toward_bg(accent, bg, 0.75)
    } else {
        accent
    };
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
        list_item_spacing: sp.list_item_spacing,
        list_group_spacing: sp.list_group_spacing,
        rule_spacing: sp.rule_spacing,
        paragraph_spacing: sp.paragraph_spacing,
        heading_spacing_top: sp.heading_spacing_top,
        heading_spacing_bottom: sp.heading_spacing_bottom,
        code_block_padding: sp.code_block_padding,
        blockquote_padding: sp.blockquote_padding,
        list_indent: sp.list_indent,
        table_cell_padding: sp.table_cell_padding,
        line_height,
        code_line_height: sp.code_line_height,

        background_color: bg,
        rule_thickness: sp.rule_thickness,
        rule_width_ratio: sp.rule_width_ratio,
    }
}
```

- [ ] **Step 2: Run `cargo check -p edit-plus-markdown`**

---

### Task 17: Full workspace build and test

- [ ] **Step 1: Run `cargo build` from workspace root**

Expected: full workspace compiles cleanly.

- [ ] **Step 2: Run `cargo test` from workspace root**

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "refactor: migrate all consumers to modular Theme tokens

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```
