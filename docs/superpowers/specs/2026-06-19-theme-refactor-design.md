# Theme Refactor — Design Tokens & Modular Structure

## Summary

Replace the flat 48-field `Theme` struct with a modular design-token architecture: `ColorPalette` (18 tokens), `EditorTheme` (8), and `MarkdownTheme` (24 leaf fields). Eliminates dead fields, fixes gamma correction gaps, and provides a semantic color language for all UI components.

## Motivation

- **48 flat fields** in `Theme` — no semantic grouping, no reuse
- **Dead fields**: `sidebar_button_bg`, `sidebar_item_bg`, `tab_bar_bg` set but never read
- **Gamma correction gaps**: 18+ fields (all toc_*, scrollbar_*, search_match_*, etc.) never gamma-corrected
- **Hardcoded markdown colors**: `MarkdownStyle::from_theme()` uses `is_dark` branching instead of actual tokens
- **No spacing tokens**: markdown spacing entirely hardcoded

## Design

### Top-level Theme struct

```rust
pub struct Theme {
    pub name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub scopes: HashMap<String, [f32; 4]>,  // syntax highlighting
}
```

### ColorPalette (18 tokens)

```rust
pub struct ColorPalette {
    // Backgrounds (5)
    pub bg_base: [f32; 4],       // editor perimeter, main view area
    pub bg_surface: [f32; 4],    // Sidebar, TabBar, StatusBar
    pub bg_elevated: [f32; 4],   // Menu, Tooltip
    pub bg_hover: [f32; 4],      // item/button hover
    pub bg_active: [f32; 4],     // item/button active/selected

    // Text (3)
    pub text_main: [f32; 4],     // primary text, menu labels, tab labels, input text
    pub text_muted: [f32; 4],    // sidebar inactive items, status bar, placeholder
    pub text_inverse: [f32; 4],  // tooltip text (light on dark)

    // Borders & Shadow (3)
    pub border_subtle: [f32; 4], // panel borders (sidebar right edge), table borders, horizontal rules
    pub border_strong: [f32; 4], // menu/tooltip/input borders, menu separators, TOC border
    pub shadow: [f32; 4],        // menu drop shadow

    // Accents & Feedback (4)
    pub accent: [f32; 4],        // brand color, active indicator, blockquote border
    pub highlight: [f32; 4],     // search match highlight (inactive: alpha *= 0.5 at use site)
    pub danger: [f32; 4],        // error, search-no-results
    pub warning: [f32; 4],       // warning state

    // Input (3)
    pub input_bg: [f32; 4],      // text input / search bar background
    pub input_border: [f32; 4],  // text input / search bar border
    pub input_fg: [f32; 4],      // input text / cursor
}
```

### EditorTheme (8 fields)

```rust
pub struct EditorTheme {
    pub background: [f32; 4],      // editor content area (independent of palette.bg_base)
    pub foreground: [f32; 4],      // editor default text (independent of palette.text_main)
    pub gutter_bg: [f32; 4],       // line number gutter background
    pub line_number: [f32; 4],     // line number text
    pub selection: [f32; 4],       // text selection highlight
    pub cursor: [f32; 4],          // caret color
    pub scrollbar_track: [f32; 4], // scrollbar track
    pub scrollbar_thumb: [f32; 4], // scrollbar thumb
}
```

### MarkdownTheme (24 leaf fields)

```rust
pub struct MarkdownSpacing {
    // Block spacing
    pub paragraph_spacing: f32,
    pub heading_spacing_top: f32,
    pub heading_spacing_bottom: f32,
    pub list_item_spacing: f32,
    pub list_group_spacing: f32,
    pub list_indent: f32,

    // Padding
    pub code_block_padding: f32,
    pub blockquote_padding: f32,
    pub table_cell_padding: f32,

    // Rule (horizontal line)
    pub rule_spacing: f32,
    pub rule_thickness: f32,
    pub rule_width_ratio: f32,

    // Border radius
    pub border_radius_base: f32,
    pub border_radius_small: f32,

    // Code
    pub code_line_height: f32,
}

pub struct MarkdownTheme {
    // Markup colors (5)
    pub heading: [f32; 4],
    pub link: [f32; 4],
    pub inline_code: [f32; 4],
    pub code_bg: [f32; 4],
    pub code_block_bg: [f32; 4],

    // TOC (4)
    pub toc_background: [f32; 4],
    pub toc_active_background: [f32; 4],  // unified hover + active
    pub toc_text: [f32; 4],               // all states; background signals state change
    pub toc_level_indicator: [f32; 4],

    // Spacing (15)
    pub spacing: MarkdownSpacing,
}
```

### Field migration map

| Old flat field | New token | Notes |
|---|---|---|
| `background` | `editor.background` | also used as window clear color |
| `menu_bg` | `palette.bg_elevated` | |
| `menu_border` | `palette.border_strong` | |
| `menu_hover` | `palette.bg_hover` | |
| `menu_selected` | `palette.bg_active` | |
| `menu_separator` | `palette.border_strong` | merged — separator = border |
| `menu_shadow` | `palette.shadow` | |
| `menu_text` | `palette.text_main` | |
| `tooltip_bg` | `palette.bg_elevated` | |
| `tooltip_fg` | `palette.text_inverse` | |
| `tooltip_border` | `palette.border_strong` | |
| `gutter_bg` | `editor.gutter_bg` | |
| `tab_bar_bg` | `palette.bg_surface` | was dead; tab bar used darken_color(gutter_bg) |
| `status_bar_bg` | `palette.bg_surface` | |
| `status_bar_fg` | `palette.text_muted` | |
| `search_bar_bg` | `palette.input_bg` | |
| `search_bar_fg` | `palette.input_fg` | |
| `search_bar_border` | `palette.input_border` | |
| `search_bar_no_results_fg` | `palette.danger` | |
| `search_match_active` | `palette.highlight` | alpha 1.0 |
| `search_match_inactive` | `palette.highlight` | alpha *= 0.5 at use site |
| `toc_background` | `markdown.toc_background` | |
| `toc_border` | `palette.border_strong` | |
| `toc_active_background` | `markdown.toc_active_background` | |
| `toc_hover_background` | `markdown.toc_active_background` | merged — same color |
| `toc_text_color` | `markdown.toc_text` | |
| `toc_active_text_color` | `markdown.toc_text` | background signals state |
| `toc_hover_text_color` | `markdown.toc_text` | background signals state |
| `toc_empty_text_color` | `palette.text_muted` | |
| `toc_level_indicator` | `markdown.toc_level_indicator` | |
| `line_number` | `editor.line_number` | |
| `selection` | `editor.selection` | |
| `cursor` | `editor.cursor` | |
| `foreground` | `editor.foreground` | |
| `scrollbar_track` | `editor.scrollbar_track` | |
| `scrollbar_thumb` | `editor.scrollbar_thumb` | |
| `sidebar_bg` | `palette.bg_surface` | |
| `sidebar_header_bg` | `palette.bg_surface` | darkened at use site if needed |
| `sidebar_button_bg` | — | **deleted** (dead field) |
| `sidebar_item_bg` | — | **deleted** (dead field) |
| `sidebar_item_active_bg` | `palette.bg_active` | |
| `sidebar_item_hover_bg` | `palette.bg_hover` | |
| `sidebar_item_fg` | `palette.text_muted` | |
| `sidebar_item_active_fg` | `palette.text_main` | |
| `sidebar_accent` | `palette.accent` | |
| `sidebar_border` | `palette.border_subtle` | |

### Markdown derived colors (not tokens, computed in MarkdownStyle::from_theme)

| MarkdownStyle field | Derivation |
|---|---|
| `text_color` | `editor.foreground` |
| `code_color` | `editor.foreground` |
| `heading_color` | `markdown.heading` |
| `link_color` | `markdown.link` |
| `code_bg` | `markdown.code_bg` |
| `code_block_bg` | `markdown.code_block_bg` |
| `inline_code_bg` | `blend_toward_bg(markdown.code_bg, editor.background, 0.94)` |
| `blockquote_border` | `blend_toward_bg(palette.accent, editor.background, 0.75)` (dark) / `palette.accent` (light) |
| `blockquote_bg` | `palette.accent` with alpha 0.08 (dark) / 0.05 (light) |
| `table_border` | `palette.border_subtle` |
| `table_header_bg` | `palette.bg_hover` |
| `table_stripe_bg` | `palette.bg_hover` × 0.5 alpha |
| `rule_color` | `palette.border_subtle` |
| `code_block_border` | `palette.border_subtle` |
| `background_color` | `editor.background` |
| `text_color` (bold) | `editor.foreground` (weight differentiation only) |
| `text_color` (italic) | `editor.foreground` (style differentiation only) |
| `text_color` (strikethrough) | `palette.text_muted` |

### Gamma correction

Each module gets its own `gamma_correct(&mut self)` method that processes every color field — no exceptions:

```rust
impl ColorPalette {
    fn gamma_correct(&mut self) { /* all 18 fields */ }
}
impl EditorTheme {
    fn gamma_correct(&mut self) { /* all 8 fields */ }
}
impl MarkdownTheme {
    fn gamma_correct(&mut self) { /* all 9 color fields */ }
}
impl Theme {
    fn gamma_correct(&mut self) {
        self.palette.gamma_correct();
        self.editor.gamma_correct();
        self.markdown.gamma_correct();
        for c in self.scopes.values_mut() { /* ... */ }
    }
}
```

## Implementation phases

### Phase 1: Define new structures + adapt theme generators
- Define `ColorPalette`, `EditorTheme`, `MarkdownTheme`, `MarkdownSpacing` in `theme.rs`
- Rewrite `Theme` struct with new modular fields
- Rewrite `Theme::dark()`, `Theme::light()`, `Theme::claude_light()`, `Theme::claude_dark()` — mapping old flat values to new tokens
- Implement per-module `gamma_correct()`
- Rewrite `test_theme()` fixtures

### Phase 2: Update UI widgets to use new tokens
- `crates/ui/src/widgets/` — batch replace `theme.<old_field>` → `theme.<new_path>`
- Sidebar: `sidebar_bg` → `palette.bg_surface`, `sidebar_item_fg` → `palette.text_muted`, etc.
- Menu: `menu_bg` → `palette.bg_elevated`, `menu_text` → `palette.text_main`, etc.
- StatusBar, TabBar, SearchBar, Tooltip, Scrollbar, TOC — same pattern
- TextBox: `search_bar_*` → `palette.input_*`
- Remove dead field references

### Phase 3: Update app-level and markdown consumers
- `crates/app/src/` — editor field access: `theme.background` → `theme.editor.background`
- `crates/markdown/src/style.rs` — rewrite `MarkdownStyle::from_theme()` to use `markdown.*` tokens and palette-derived values per the derivation table above
- Remove old `is_dark` branching for colors (keep only where truly structural, e.g., blend direction)

### Phase 4: Verify
- Full workspace `cargo build` passes
- `cargo test` passes
- Visual smoke test: all 4 themes render correctly

## Decisions log

| Decision | Rationale |
|---|---|
| `separator` merged into `border_strong` | Menu separator = border, same visual role |
| `highlight` as new token (not reuse `warning`) | "Found match" ≠ "warning" semantically |
| Inactive search match = `highlight` × 0.5 alpha | One token, alpha variant avoids token bloat |
| `input_*` tokens as new category | TextBox and SearchBar share input semantics; independent from surface colors |
| `status_bar_fg` → `text_muted` | Status bar text is secondary/auxiliary |
| TOC hover + active background merged to `toc_active_background` | Both are "highlighted" states, same color |
| TOC text single field (no hover/active variants) | Background change suffices to signal state |
| `toc_border` → `palette.border_strong` | Panel border, same as menu/tooltip |
| `toc_empty_text` → `palette.text_muted` | Placeholder text |
| Scrollbar stays in EditorTheme | Only editor has scrollbar currently; move to Palette if TOC/menu need it later |
| `editor.background` / `editor.foreground` independent from palette | Editor text area may differ from chrome colors |
| Markdown bold/italic/strikethrough/list_marker not tokens | Derivable from palette.text_main / palette.text_muted |
| Markdown blockquote_bg/table_*/rule/blockquote_border derived from palette | accent + bg_hover + border_subtle cover all cases |
| All spacing/geometry in MarkdownSpacing (15 fields) | Full control; only `line_height` from editor settings |
| Dead fields deleted, not deprecated | No consumers exist, no migration needed |
| No theme config file in this round | Separate project; this round focuses on modular structure |
