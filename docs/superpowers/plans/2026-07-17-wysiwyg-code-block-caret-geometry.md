# WYSIWYG Code Block Caret Geometry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make activated Markdown WYSIWYG code-block text, caret positioning, and hit-testing use the same shaped glyph advances.

**Architecture:** Shape each materialized code line during block layout with the configured code font and retain the resulting `ShapedRun` on `LaidOutLine`. The existing flat-line projection then carries that geometry into `grapheme_x` and `grapheme_at_x`, so no code-block-specific cursor compensation is needed.

**Tech Stack:** Rust, `textora-markdown`, HarfBuzz-backed `shaping::Shaper`, Cargo tests.

## Global Constraints

- Preserve lazy layout: only shape code lines in materialized blocks.
- Do not change code-block activation, syntax highlighting, source projection, or app/ui dependency boundaries.
- Do not introduce fixed-width ratios or code-block-specific caret offsets.
- Follow test-first RED → GREEN and run `cargo fmt` before completion.

---

### Task 1: Retain shaped code-line geometry

**Files:**
- Modify: `crates/markdown/src/view.rs` (WYSIWYG regression tests)
- Modify: `crates/markdown/src/layout/block.rs` (code-block line construction)

**Interfaces:**
- Consumes: `crate::layout::shaping::shape_line(text, font_size, weight, style, font_family, shaper)`.
- Produces: `LaidOutLine.shaped: Option<shaping::ShapedRun>` populated for non-empty materialized code lines; existing `LazyLayout::build_flat_lines` copies it into `FlatLine.shaped`.

- [ ] **Step 1: Write the failing regression test**

Add this test to `crates/markdown/src/view.rs` inside `mod wysiwyg_tests` near the existing cursor/hit-test geometry tests:

```rust
#[test]
fn active_code_block_cursor_uses_shaped_code_font_geometry() {
    use ui::plugin::{PluginMessage, ViewPlugin};

    let source = "```text\nabcdefghij\n```";
    let code_start = source.find("abcdefghij").expect("fixture must contain code text");
    let cursor_byte = code_start + 8;
    let mut document = StubDoc::new(source);
    let mut view = MarkdownEditorView::new();
    view.set_source(document.text.clone(), 1);
    view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut document);
    render_editor_once(&mut view, &document);

    let visual_position = view
        .engine()
        .cursor_visual_position_for_byte(cursor_byte, CursorAffinity::Downstream)
        .expect("active code byte must have a visual position");
    let flat_line_idx = view
        .engine()
        .lazy
        .as_ref()
        .and_then(|lazy| lazy.flat_line_idx_for_projection(visual_position.flat_line_idx))
        .expect("active code projection must map to a flat line");
    let flat_line = &view.engine().flat_lines()[flat_line_idx];
    let shaped = flat_line.shaped.as_ref().expect("active code line must retain shaping");
    let local_byte = cursor_byte - code_start;
    let expected_advance: f32 = shaped
        .clusters
        .iter()
        .take_while(|cluster| cluster.byte_range.start < local_byte)
        .map(|cluster| cluster.advance)
        .sum();
    let (cursor_x, cursor_y, _cursor_width, cursor_height) =
        view.engine().cursor_screen_pos().expect("active code cursor must resolve");

    assert!(
        (cursor_x - (flat_line.rect.x + expected_advance)).abs() < 0.01,
        "cursor x {cursor_x} must use shaped code advance {expected_advance}"
    );
    assert_eq!(
        view.engine().hit_test_byte(
            cursor_x,
            cursor_y + cursor_height * 0.5,
            0.0,
            0.0,
        ),
        Some(cursor_byte),
        "hit-testing at the shaped caret boundary must return the same source byte"
    );
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p textora-markdown view::wysiwyg_tests::active_code_block_cursor_uses_shaped_code_font_geometry -- --exact
```

Expected: FAIL at `active code line must retain shaping`, proving code lines currently publish estimated rather than shaped geometry.

- [ ] **Step 3: Populate `LaidOutLine.shaped` for code lines**

In the `BlockKind::CodeBlock` loop in `crates/markdown/src/layout/block.rs`, shape each line before constructing `LaidOutLine`:

```rust
let (shaped, _) = super::shaping::shape_line(
    line_text,
    font_size,
    Weight::NORMAL,
    shaping::Style::Normal,
    ctx.style.code_font_family.as_deref(),
    ctx.shaper.as_deref_mut(),
);
```

Then replace the existing field value:

```rust
shaped,
```

Keep `text_layout: None` so syntax-highlighted and plain code lines retain the existing render path; this change only supplies canonical geometry to flat lines.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
cargo test -p textora-markdown view::wysiwyg_tests::active_code_block_cursor_uses_shaped_code_font_geometry -- --exact
```

Expected: PASS.

- [ ] **Step 5: Run formatting and Markdown regression tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown
```

Expected: both commands exit 0 with no failing tests.

- [ ] **Step 6: Review the scoped diff**

Run:

```bash
git diff --check
git diff -- crates/markdown/src/layout/block.rs crates/markdown/src/view.rs
```

Expected: no whitespace errors; the production diff only adds shaped geometry to code lines and the test diff only adds the focused regression.

- [ ] **Step 7: Commit the fix**

```bash
git add crates/markdown/src/layout/block.rs crates/markdown/src/view.rs docs/superpowers/plans/2026-07-17-wysiwyg-code-block-caret-geometry.md
git commit -m "fix(markdown): align code block caret geometry"
```
