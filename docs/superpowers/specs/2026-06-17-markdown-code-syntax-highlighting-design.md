# Markdown Code Block Syntax Highlighting — Design Spec

Status: approved (revised)

## Goal

Add syntax highlighting to fenced code blocks in markdown preview, using the
existing LSH highlighting infrastructure.

## Current State

- Markdown code blocks render with monospace font + background + border but **no highlighting**
- Parser already captures language tag from fenced code blocks (`CodeBlock { language: Some("rust") }`)
- LSH has 15 language definitions with a custom VM runtime
- `ReadableDocument` trait is implemented for `&[u8]` and `String`, so LSH
  works on plain strings without a document handle

## Design

### Dependency Inversion via Trait

markdown crate defines a trait. The app crate (composition root, depends on
both core and markdown) implements it. This keeps markdown free of lsh/core
dependencies and avoids a circular dependency (core cannot depend on markdown
because markdown → ui → core).

```rust
// In crates/markdown/src/builder.rs or layout.rs

/// A highlighted span within a code line.
/// `start` and `len` are **byte offsets** (not char indices), matching the
/// LSH runtime's offset convention and Rust's UTF-8 string layout.
pub struct HighlightSpan {
    pub start: usize,
    pub len: usize,
    pub color: [f32; 4], // RGBA, host-determined
}

pub trait CodeHighlighter {
    /// Highlight an entire code block. Returns per-line spans.
    fn highlight(&self, language: &str, code: &str) -> Vec<Vec<HighlightSpan>>;
}
```

### Data Model Changes

**LaidOutLine** gains an optional field:

```rust
pub highlight_spans: Vec<HighlightSpan>, // empty = no highlighting (normal text line)
```

### Language Matching (implemented in core/highlight)

Three-level fuzzy match, in priority order:

1. **Exact id match** — normalize `_` ↔ `-`, compare to LSH language `id`
2. **Case-insensitive name match** — compare lowercase to LSH `display_name`
3. **Extension match** — extract extension from LSH `path` globs (`**/*.js` → `js`), match tag

Startup cost: build three `HashMap<&str, &'static Language>` from the
compile-time-generated `LANGUAGES` and `FILE_ASSOCIATIONS` arrays.

Note: the matching logic only needs `LANGUAGES` and `FILE_ASSOCIATIONS` (both
already in core), so it can live in `core::highlight` without pulling in
markdown or lsh runtime.

### Layout Integration

In precision layout (or full layout for non-lazy mode), when laying out a
`CodeBlock`:

1. Look up language by tag via `find_language(tag)`
2. If found and `CodeHighlighter` is available, call `highlight(language, text)`
3. Store per-line `HighlightSpan` vectors on each `LaidOutLine`

Lazy layout naturally defers this work to when the block enters the viewport,
avoiding the cost of highlighting off-screen code blocks.

### Render Changes

Code line rendering reads `highlight_spans`. When non-empty, render each span
in sequence with its color. Empty spans fall back to the existing `code_color`.

### Color Scheme (app crate)

No hardcoded colors. The existing theme infrastructure already provides
everything needed:

1. `core::highlight::highlight_kind_scope(kind)` maps `HighlightKind` → TextMate-style
   scope string (e.g., `"keyword.control"`, `"string"`, `"comment"`)
2. `ui::Theme::scope_color(scope)` looks up the RGBA color for that scope from
   the current One Dark / One Light theme

In the `app` crate's `CodeHighlighter` impl:

```rust
fn kind_to_color(kind: HighlightKind, theme: &Theme) -> [f32; 4] {
    let scope = core::highlight::highlight_kind_scope(kind);
    theme.scope_color(scope)
}
```

This provides:
- Automatic light/dark theme switching (zero additional work)
- Consistency with the editor's existing syntax colors
- No color definitions to maintain outside the theme

## Dependency Graph

```
app ──┬── core (LSH, HighlightKind, Language, highlight_kind_scope)
      ├── markdown (CodeHighlighter trait, HighlightSpan)
      └── ui (Theme::scope_color)

markdown ── ui (no new deps)
core ── lsh (existing, no new deps)
```

`app` is the composition root — it wires `core::highlight` output through
`ui::Theme` colors into `markdown::HighlightSpan`.

## Files Changed

| File | Change |
| ---- | ------ |
| `crates/markdown/src/builder.rs` | Add `HighlightSpan`, `CodeHighlighter` trait |
| `crates/markdown/src/layout.rs` | Call `CodeHighlighter` during precision layout of CodeBlock; store spans on `LaidOutLine` |
| `crates/markdown/src/render.rs` | Render code lines with per-span colors |
| `crates/core/src/highlight/mod.rs` or new file | `find_language()` matcher |
| `crates/app/` | Implement `CodeHighlighter`: wire LSH runtime + `highlight_kind_scope()` + `Theme::scope_color()` |

## Open Questions

- None remaining. All sections approved.
