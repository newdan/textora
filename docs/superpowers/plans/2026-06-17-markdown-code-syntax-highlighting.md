# Markdown Code Block Syntax Highlighting — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add LSH-powered syntax highlighting to fenced code blocks in markdown preview, using dependency-inverted trait design to keep markdown crate free of lsh/core dependencies.

**Architecture:** markdown crate defines `CodeHighlighter` trait + `HighlightSpan` struct. `LazyLayout`/precision-layout calls the trait for visible `CodeBlock` lines. `core::highlight` provides `find_language()` for tag→Language matching. `app` implements the trait by wiring LSH runtime → `highlight_kind_scope()` → `Theme::scope_color()`.

**Tech Stack:** Rust, LSH VM runtime, pulldown-cmark, existing Theme system

---

## File Map

| File | Role |
|------|------|
| `crates/markdown/src/builder.rs` | Add `HighlightSpan`, `CodeHighlighter` trait |
| `crates/markdown/src/layout.rs` | Add `highlight_spans` to `LaidOutLine`; thread highlighter; call during CodeBlock layout |
| `crates/markdown/src/render.rs` | Render code lines per-span when `highlight_spans` non-empty |
| `crates/markdown/src/lib.rs` | Thread `Option<&dyn CodeHighlighter>` through public API |
| `crates/core/src/highlight/mod.rs` | Add `find_language(tag)` — 3-level fuzzy match |
| `crates/core/Cargo.toml` | No changes |
| `crates/app/src/md_preview.rs` | Inject `CodeHighlighter` impl; wire into render |
| `crates/app/Cargo.toml` | No changes (already depends on core + markdown) |

---

### Task 1: Add HighlightSpan and CodeHighlighter trait to markdown crate

**Files:**
- Modify: `crates/markdown/src/builder.rs`

- [ ] **Step 1: Add struct and trait to builder.rs**

After the `InlineStyle` enum (line 62), add:

```rust
/// A highlighted span within a code line.
/// `start` and `len` are **byte offsets** (not char indices),
/// matching the LSH runtime's offset convention and Rust's UTF-8 layout.
#[derive(Clone, Debug)]
pub struct HighlightSpan {
    pub start: usize,
    pub len: usize,
    pub color: [f32; 4],
}

/// Injected by the host application. Stateless — called once per code block.
pub trait CodeHighlighter {
    /// Highlight an entire code block. Returns per-line spans.
    fn highlight(&self, language: &str, code: &str) -> Vec<Vec<HighlightSpan>>;
}
```

- [ ] **Step 2: Build check**

```bash
cargo check -p edit-plus-markdown 2>&1 | tail -5
```
Expected: compiles cleanly (new types unused, no warnings preferred but may have "unused" warnings).

- [ ] **Step 3: Commit**

```bash
git add crates/markdown/src/builder.rs
git commit -m "feat(markdown): add HighlightSpan and CodeHighlighter trait"
```

---

### Task 2: Add highlight_spans field to LaidOutLine

**Files:**
- Modify: `crates/markdown/src/layout.rs:279-296`

- [ ] **Step 1: Add field to LaidOutLine**

In `LaidOutLine` (around line 296, before the closing `}`), add:

```rust
    /// Syntax highlight spans for code lines. Empty for non-code lines.
    /// Populated by the CodeHighlighter during precision layout.
    pub highlight_spans: Vec<crate::builder::HighlightSpan>,
```

- [ ] **Step 2: Initialize field everywhere LaidOutLine is constructed**

In `layout_block` → `CodeBlock` branch (~line 758), add the field to the constructor:

```rust
laid_out_lines.push(LaidOutLine {
    text: line_text.clone(),
    rect: Rect::new(ctx.indent + pad, ly, ctx.available_width() - pad * 2.0, line_h),
    font_size,
    is_code: true,
    color_override: Some(ctx.style.code_color),
    styles: vec![],
    style_segments: vec![],
    shaped: None,
    text_layout: None,
    highlight_spans: vec![],  // NEW
});
```

Find and update all other `LaidOutLine` constructions. Grep for `LaidOutLine {` to find them all.

Run:
```bash
grep -n "LaidOutLine {" crates/markdown/src/layout.rs
```

Each must have `highlight_spans: vec![],` added. This is the only safe default — no highlighting.

- [ ] **Step 3: Build check**

```bash
cargo check -p edit-plus-markdown 2>&1 | tail -10
```
Expected: compiles cleanly.

- [ ] **Step 4: Run existing tests**

```bash
cargo test -p edit-plus-markdown 2>&1 | tail -20
```
Expected: all existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/layout.rs
git commit -m "feat(markdown): add highlight_spans field to LaidOutLine"
```

---

### Task 3: Thread CodeHighlighter through layout

**Files:**
- Modify: `crates/markdown/src/layout.rs`
- Modify: `crates/markdown/src/lib.rs`

- [ ] **Step 1: Add highlighter to LayoutCtx**

In `LayoutCtx` struct (search for `struct LayoutCtx`), add field:

```rust
    highlighter: Option<&'a dyn crate::builder::CodeHighlighter>,
```

And update `LayoutCtx::new` to accept and set it:

```rust
fn new(
    style: &'a MarkdownStyle,
    viewport_w: f32,
    shaper: Option<&'a mut shaping::Shaper>,
    highlighter: Option<&'a dyn crate::builder::CodeHighlighter>,
) -> Self {
    // ... existing fields ...
    highlighter,
}
```

- [ ] **Step 2: Pass language tag through to LaidOutBlockKind::CodeBlock**

In `layout_block` → `CodeBlock` branch (~line 744), change:

```rust
// Before:
BlockKind::CodeBlock { language: _ } => {
    // ...
    LaidOutBlockKind::CodeBlock {
        lines: laid_out_lines,
        language: None,
    },

// After:
BlockKind::CodeBlock { language } => {
    // ...
    LaidOutBlockKind::CodeBlock {
        lines: laid_out_lines,
        language: language.clone(),
    },
```

- [ ] **Step 3: Call highlighter during code block layout (estimation path)**

In the same CodeBlock branch, after collecting `lines` and before creating `laid_out_lines`, add:

```rust
// Syntax highlighting via injected highlighter
let all_highlight_spans: Vec<Vec<HighlightSpan>> = match (language.as_deref(), ctx.highlighter) {
    (Some(tag), Some(hl)) => hl.highlight(tag, &raw.join("\n")),
    _ => vec![],
};

// Then when creating each LaidOutLine:
laid_out_lines.push(LaidOutLine {
    // ...
    highlight_spans: all_highlight_spans
        .get(line_idx)
        .cloned()
        .unwrap_or_default(),
});
```

- [ ] **Step 4: Update layout_doc_with_shaper signature**

```rust
pub fn layout_doc_with_shaper(
    doc: &MarkdownDoc,
    style: &MarkdownStyle,
    viewport_w: f32,
    shaper: Option<&mut Shaper>,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
) -> LaidOutDoc {
    let mut ctx = LayoutCtx::new(style, viewport_w, shaper, highlighter);
    // ... rest unchanged
```

- [ ] **Step 5: Update layout_doc (no-shaper variant)**

```rust
pub fn layout_doc(doc: &MarkdownDoc, style: &MarkdownStyle, viewport_w: f32) -> LaidOutDoc {
    layout_doc_with_shaper(doc, style, viewport_w, None, None)
}
```

- [ ] **Step 6: Thread highlighter through LazyLayout and precise_block_at**

In `LazyLayout::from_doc`, accept highlighter:

```rust
pub fn from_doc(
    doc: MarkdownDoc,
    style: &MarkdownStyle,
    viewport_w: f32,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
) -> Self {
    let laid_out = layout_doc_with_shaper(&doc, style, viewport_w, None, highlighter);
    // ... rest unchanged
```

In `ensure_precise_range`, add highlighter param and pass to `precise_block_at`:

```rust
pub fn ensure_precise_range(
    &mut self,
    scroll_y: f32,
    viewport_h: f32,
    style: &MarkdownStyle,
    shaper: &mut shaping::Shaper,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
) -> Vec<(usize, f32)> {
    // ... existing range logic ...
    for i in indices {
        let delta = self.precise_block_at(i, style, shaper, highlighter);
        // ...
    }
```

In `precise_block_at`, add highlighter param:

```rust
pub fn precise_block_at(
    &mut self,
    idx: usize,
    style: &MarkdownStyle,
    shaper: &mut shaping::Shaper,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
) -> f32 {
    // ...
    let mut ctx = LayoutCtx::new(
        style,
        self.laid_out.blocks[idx].rect.w + self.laid_out.blocks[idx].rect.x,
        Some(shaper),
        highlighter,
    );
    // ... rest unchanged
```

- [ ] **Step 7: Update lib.rs public API**

In `crates/markdown/src/lib.rs`, update `render_markdown_with_offset` to accept highlighter and pass it through. Add a variant that takes highlighter:

```rust
pub fn render_markdown_with_offset(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
    offset_x: f32,
    offset_y: f32,
) -> DrawList {
    render_markdown_highlighted(src, style, viewport_w, viewport_h, scroll_y, shaper, offset_x, offset_y, None)
}

/// Render with optional syntax highlighting for code blocks.
pub fn render_markdown_highlighted(
    src: &str,
    style: &style::MarkdownStyle,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
    offset_x: f32,
    offset_y: f32,
    highlighter: Option<&dyn builder::CodeHighlighter>,
) -> DrawList {
    let parsed = parser::parse_markdown(src);
    let doc = builder::MarkdownDoc::build(&parsed, style);
    let laid_out = layout::layout_doc_with_shaper(&doc, style, viewport_w, shaper.as_deref_mut(), highlighter);
    let mut dl = DrawList::new();
    render::render_doc_with_offset(&laid_out, style, &mut dl, scroll_y, viewport_h, offset_x, offset_y, shaper, &[]);
    dl
}
```

- [ ] **Step 8: Build and test**

```bash
cargo check -p edit-plus-markdown 2>&1 | tail -10
cargo test -p edit-plus-markdown 2>&1 | tail -20
```
Expected: compiles cleanly, all tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/markdown/src/layout.rs crates/markdown/src/lib.rs
git commit -m "feat(markdown): thread CodeHighlighter through layout pipeline"
```

---

### Task 4: Render highlight spans in code lines

**Files:**
- Modify: `crates/markdown/src/render.rs`

- [ ] **Step 1: Add highlight rendering path in render_line_with_offset**

At the start of `render_line_with_offset`, before the existing fast-path check, add:

```rust
// Highlighted code line: render each span with its color
if !line.highlight_spans.is_empty() {
    for span in &line.highlight_spans {
        let span_end = (span.start + span.len).min(text_len);
        if span.start >= text_len { continue; }
        let segment = &line.text[safe_byte_idx(&line.text, span.start)..safe_byte_idx(&line.text, span_end)];
        if segment.is_empty() { continue; }
        let x = line_x + estimate_text_width(
            &line.text[..safe_byte_idx(&line.text, span.start)],
            font_size,
        );
        if let Some(ref mut s) = shaper {
            dl.text_shaped_with_font(
                x,
                ly + font_size,
                font_size,
                span.color,
                segment,
                line.is_code.then(|| style.code_font_family.clone()).flatten(),
                Weight::NORMAL,
                Style::Normal,
                false,
                s,
            );
        }
    }
    return;
}
```

Note: use the existing `estimate_text_width` function at the bottom of render.rs and `safe_byte_idx` from the crate root.

- [ ] **Step 2: Build and test**

```bash
cargo test -p edit-plus-markdown 2>&1 | tail -20
```
Expected: all existing tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/markdown/src/render.rs
git commit -m "feat(markdown): render highlight spans in code lines"
```

---

### Task 5: Add find_language() to core::highlight

**Files:**
- Modify: `crates/core/src/highlight/mod.rs`

- [ ] **Step 1: Add language matcher**

After the existing `highlight_kind_scope` function, add:

```rust
use std::collections::HashMap;
use std::sync::LazyLock;

struct LanguageMatcher {
    by_id: HashMap<&'static str, &'static Language>,
    by_name: HashMap<&'static str, &'static Language>,
    by_ext: HashMap<&'static str, &'static Language>,
}

fn build_language_matcher() -> LanguageMatcher {
    let mut by_id = HashMap::new();
    let mut by_name = HashMap::new();
    let mut by_ext = HashMap::new();

    for lang in LANGUAGES {
        by_id.insert(lang.id, lang);
        by_name.insert(lang.name.to_lowercase().leak(), lang);
    }

    for (pattern, lang) in FILE_ASSOCIATIONS {
        // Extract extension from "**/*.ext" pattern
        if let Some(ext) = pattern.rsplit('.').next() {
            if ext != "*" && !ext.contains('/') {
                by_ext.entry(ext.to_lowercase().leak()).or_insert(lang);
            }
        }
    }

    LanguageMatcher { by_id, by_name, by_ext }
}

static LANG_MATCHER: LazyLock<LanguageMatcher> = LazyLock::new(build_language_matcher);

/// Find an LSH language by code block tag (e.g., "rust", "js", "python").
/// Three-level fuzzy match: exact id → case-insensitive name → extension.
pub fn find_language(tag: &str) -> Option<&'static Language> {
    let m = &*LANG_MATCHER;

    // 1. Exact id match
    if let Some(lang) = m.by_id.get(tag) {
        return Some(lang);
    }

    let tag_lower = tag.to_lowercase();

    // 2. Case-insensitive display name match
    if let Some(lang) = m.by_name.get(tag_lower.as_str()) {
        return Some(lang);
    }

    // 3. Extension match
    if let Some(lang) = m.by_ext.get(tag_lower.as_str()) {
        return Some(lang);
    }

    None
}
```

The `to_lowercase().leak()` calls allocate static strings for HashMap keys. These leak a handful of short strings at startup — acceptable for a long-lived editor process.

- [ ] **Step 2: Build and test**

```bash
cargo check -p edit-plus-core 2>&1 | tail -10
cargo test -p edit-plus-core 2>&1 | tail -20
```
Expected: compiles cleanly, existing tests pass.

- [ ] **Step 3: Write unit test for find_language**

In `crates/core/src/highlight/mod.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_language_exact_id() {
        let lang = find_language("javascript");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "javascript");
    }

    #[test]
    fn find_language_case_insensitive_name() {
        let lang = find_language("JavaScript");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "javascript");
    }

    #[test]
    fn find_language_extension() {
        let lang = find_language("js");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "javascript");
    }

    #[test]
    fn find_language_unknown_returns_none() {
        assert!(find_language("nonexistent-language").is_none());
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p edit-plus-core highlight 2>&1 | tail -20
```
Expected: 4 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/highlight/mod.rs
git commit -m "feat(core): add find_language() for code block tag matching"
```

---

### Task 6: Implement CodeHighlighter in app crate

**Files:**
- Modify: `crates/app/src/md_preview.rs`

- [ ] **Step 1: Add AppCodeHighlighter struct**

In `md_preview.rs`, add imports and the impl:

```rust
use edit_plus_core::highlight::{Highlighter, HighlightKind, find_language};
use edit_plus_core::highlight::definitions::{ASSEMBLY, STRINGS, CHARSETS};
use edit_plus_markdown::builder::{CodeHighlighter, HighlightSpan};
use lsh::runtime::Runtime;

/// Implements markdown code block highlighting using LSH runtime.
struct AppCodeHighlighter {
    theme: ui::Theme,
}

impl CodeHighlighter for AppCodeHighlighter {
    fn highlight(&self, language: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
        let Some(lang) = find_language(language) else {
            return vec![];
        };

        let mut runtime = Runtime::new(&ASSEMBLY, &STRINGS, &CHARSETS, lang.entrypoint);
        let arena = stdext::arena::Arena::new();

        code.lines()
            .map(|line| {
                let highlights = runtime.parse_next_line(&arena, line.as_bytes());
                highlights
                    .iter()
                    .map(|h| {
                        let kind = HighlightKind::try_from(h.kind.0).unwrap_or(HighlightKind::Other);
                        let scope = edit_plus_core::highlight::highlight_kind_scope(kind);
                        let color = self.theme.scope_color(scope);
                        HighlightSpan {
                            start: h.start,
                            len: h.end - h.start,
                            color,
                        }
                    })
                    .collect()
            })
            .collect()
    }
}
```

- [ ] **Step 2: Build check**

```bash
cargo check -p edit-plus-app 2>&1 | tail -20
```
Expected: compiles (may have "unused" warning for AppCodeHighlighter, OK for now).

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "feat(app): implement CodeHighlighter via LSH runtime and Theme colors"
```

---

### Task 7: Wire highlighter into MarkdownPreview::render()

**Files:**
- Modify: `crates/app/src/md_preview.rs`

- [ ] **Step 1: Add CodeHighlighter to render() signature and wire through**

Change `MarkdownPreview::render()` to accept `highlighter` and thread it to `LazyLayout::from_doc` and `ensure_precise_range`:

In `render()`:
```rust
pub fn render(
    &mut self,
    theme: &Theme,
    viewport_w: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
) -> (DrawList, bool) {
    let style = MarkdownStyle::from_theme(/* ... */);
    let highlighter = AppCodeHighlighter { theme: theme.clone() };

    // In the dirty/lazy-init block:
    if self.dirty || /* ... */ {
        // ...
        let mut lazy = LazyLayout::from_doc(doc, &style, viewport_w, Some(&highlighter));
        if let Some(ref mut s) = shaper {
            lazy.ensure_precise_range(self.scroll_y, viewport_h, &style, s, Some(&highlighter));
        }
        // ...
    }

    // In the scroll/re-precision block:
    if let Some(ref mut lazy) = self.lazy {
        if let Some(ref mut s) = shaper {
            let deltas = lazy.ensure_precise_range(self.scroll_y, viewport_h, &style, s, Some(&highlighter));
            // ...
        }
    }
}
```

- [ ] **Step 2: Fix callers**

Search for callers of `MarkdownPreview::render()` and update if signature changed. If `render()` is only called internally, no external changes needed.

```bash
grep -rn "\.render(" crates/app/src/ | grep -i "md\|preview\|markdown"
```

- [ ] **Step 3: Build and test**

```bash
cargo build -p edit-plus-app 2>&1 | tail -20
```
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "feat(app): wire CodeHighlighter into MarkdownPreview render pipeline"
```

---

### Task 8: Integration test — render highlighted code block

**Files:**
- Create/Modify: `crates/markdown/src/lib.rs` (integration tests section)

- [ ] **Step 1: Add integration test**

In the integration test module of `lib.rs`, add:

```rust
#[test]
fn e2e_code_block_with_highlighting() {
    use crate::builder::{CodeHighlighter, HighlightSpan};

    struct MockHighlighter;
    impl CodeHighlighter for MockHighlighter {
        fn highlight(&self, _language: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
            code.lines()
                .map(|line| {
                    if line.contains("fn") {
                        vec![HighlightSpan {
                            start: 0,
                            len: 2,
                            color: [1.0, 0.5, 0.0, 1.0], // orange for keyword
                        }]
                    } else {
                        vec![]
                    }
                })
                .collect()
        }
    }

    let md = "```rust\nfn main() {\n    let x = 1;\n}\n```";
    let style = test_utils::default_style();
    let parsed = parser::parse_markdown(md);
    let doc = builder::MarkdownDoc::build(&parsed, &style);
    let hl: &dyn CodeHighlighter = &MockHighlighter;
    let laid_out = layout::layout_doc_with_shaper(&doc, &style, 400.0, None, Some(hl));
    let mut dl = DrawList::new();
    let mut shaper = shaping::Shaper::new().ok();
    render::render_doc_with_offset(
        &laid_out, &style, &mut dl, 0.0, 600.0, 0.0, 0.0,
        shaper.as_mut(), &[],
    );

    // Verify that "fn" in the code block has the orange highlight color
    let orange_texts: Vec<_> = dl.cmds.iter().filter_map(|c| {
        if let ui::core::DrawCmd::TextLayout { layout, color, .. } = c {
            if layout.text.contains("fn") && color[0] > 0.9 && color[1] < 0.6 {
                Some(layout.text.clone())
            } else { None }
        } else { None }
    }).collect();
    assert!(!orange_texts.is_empty(), "keyword 'fn' should have highlight color");
}
```

- [ ] **Step 2: Run integration test**

```bash
cargo test -p edit-plus-markdown e2e_code_block_with_highlighting 2>&1
```
Expected: test passes.

- [ ] **Step 3: Run full test suite**

```bash
cargo test -p edit-plus-markdown 2>&1 | tail -20
cargo test -p edit-plus-core 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/markdown/src/lib.rs
git commit -m "test(markdown): add integration test for code block highlighting"
```

---

### Task 9: Manual smoke test

- [ ] **Step 1: Build release**

```bash
cargo build --release 2>&1 | tail -10
```
Expected: compiles cleanly.

- [ ] **Step 2: Open a markdown file in the editor with fenced code blocks**

Manually verify:
- Code blocks with recognized language tags (```rust, ```python, ```js) render with colored syntax
- Code blocks without language tags render as plain monospace (current behavior preserved)
- Scrolling triggers lazy highlighting of newly visible blocks
- No crashes, no visual glitches

---

## Test Plan Summary

| Test | Location | What it verifies |
|------|----------|-----------------|
| `find_language_exact_id` | core/highlight/mod.rs | Exact id match (e.g., "javascript") |
| `find_language_case_insensitive_name` | core/highlight/mod.rs | Name match ("JavaScript") |
| `find_language_extension` | core/highlight/mod.rs | Extension match ("js") |
| `find_language_unknown_returns_none` | core/highlight/mod.rs | Unknown tag → None |
| `e2e_code_block_with_highlighting` | markdown/lib.rs | End-to-end: highlighter → layout → render |
| All existing markdown tests | markdown/* | No regression in existing behavior |
