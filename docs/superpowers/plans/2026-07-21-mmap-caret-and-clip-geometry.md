# mmap Caret and Clip Geometry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align mmap caret geometry with rendered shaped text and preserve original rounded geometry while applying viewport clips.

**Architecture:** mmap hit geometry will derive each grapheme x edge from a single shaped title run, matching `DrawList::text_shaped`. The paint backend will retain the fast rectangular intersection path for square rectangles; it will tessellate rounded geometry first and clip resulting triangles to the active clip stack.

**Tech Stack:** Rust, `shaping::Shaper`, `ui::DrawList`, `render::GlyphVertex`, Cargo unit tests.

## Global Constraints

- Keep the `ui` crate independent from application state; no new app dependency in `ui`.
- Do not use `.unwrap()` in production paths.
- Preserve existing behavior for un-clipped and square rectangles.
- Run `cargo fmt` before each validation command.

---

## File map

| File | Responsibility |
| --- | --- |
| `crates/markdown/src/mmf/layout.rs` | Build mmap title hit geometry from whole-run shaping and test its grapheme edges. |
| `crates/app/src/paint_backend.rs` | Tessellate and clip rounded fill/stroke triangles without redefining their corner radii. |

### Task 1: Whole-run mmap title edge geometry

**Files:**

- Modify: `crates/markdown/src/mmf/layout.rs:481-500`
- Test: `crates/markdown/src/mmf/layout.rs` inline tests

**Interfaces:**

- Consumes: `Shaper::shape(&str) -> Result<ShapedRun, ShapeError>` and `GlyphCluster { byte_range, advance, .. }`.
- Produces: `grapheme_edges(title, grapheme_byte_offsets, text_x, shaper) -> Vec<f32>` where each edge is based on the full shaped title.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn grapheme_edges_match_whole_title_shaping_for_latin_kerning() {
    let title = "从toB做起";
    let mut shaper = Shaper::new().expect("test shaper should initialize");
    let edges = grapheme_edges(title, &grapheme_byte_boundaries(title), 0.0, &mut shaper);
    let shaped = shaper.shape(title).expect("test title should shape");
    let expected_width: f32 = shaped.clusters.iter().map(|cluster| cluster.advance).sum();

    assert!((edges.last().copied().unwrap_or_default() - expected_width).abs() < 0.01);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p textora-markdown mmf::layout::tests::grapheme_edges_match_whole_title_shaping_for_latin_kerning -- --exact`

Expected: FAIL because independent grapheme shaping does not equal the full title run for the selected kerning fixture.

- [ ] **Step 3: Write minimal implementation**

```rust
if let Ok(shaped) = shaper.shape(title) {
    return grapheme_edges_from_shaped_clusters(
        grapheme_byte_offsets,
        text_x,
        &shaped.clusters,
    );
}

grapheme_edges_fallback(title, grapheme_byte_offsets, text_x, shaper)
```

The helper must sum each cluster advance into the edge after its `byte_range.start`, leave zero-width boundary gaps unchanged, and retain the current per-grapheme measurement only as an error fallback.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p textora-markdown mmf::layout::tests::grapheme_edges_match_whole_title_shaping_for_latin_kerning -- --exact`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/mmf/layout.rs
git commit -m "fix(mmap): align title caret edges with shaped text"
```

### Task 2: Preserve rounded geometry through clipping

**Files:**

- Modify: `crates/app/src/paint_backend.rs:35-53,282-485`
- Test: `crates/app/src/paint_backend.rs` inline tests

**Interfaces:**

- Consumes: rounded `DrawCmd::FillRect` / `DrawCmd::StrokeRect`, `clip_stack`, and `Screen`.
- Produces: clipped `Vec<GlyphVertex>` whose geometry is the intersection of the original tessellation and the active rectangular clips.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn clipped_rounded_fill_keeps_a_straight_viewport_cut() {
    let mut list = DrawList::new();
    list.clip(Rect::new(20.0, 0.0, 80.0, 100.0), |inner| {
        inner.fill_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 10.0);
    });
    let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

    assert!(vertices.iter().any(|vertex| vertex.position == [-0.6, 1.0]));
    assert!(vertices.iter().any(|vertex| vertex.position == [-0.6, -1.0]));
}

#[test]
fn clipped_rounded_stroke_does_not_create_a_new_left_border() {
    let mut list = DrawList::new();
    list.clip(Rect::new(20.0, 0.0, 80.0, 100.0), |inner| {
        inner.stroke_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 10.0, 2.0);
    });
    let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

    assert!(!vertices.iter().any(|vertex| {
        (vertex.position[0] + 0.6).abs() < f32::EPSILON && vertex.position[1].abs() < 0.7
    }));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p textora-app paint_backend::tests::clipped_rounded`

Expected: fill test FAILS because the pre-fix code rounds the rect starting at x=20; stroke test FAILS because it emits a synthetic left-side border.

- [ ] **Step 3: Write minimal implementation**

```rust
fn append_rounded_with_clip(
    vertices: &mut Vec<GlyphVertex>,
    rect: Rect,
    color: [f32; 4],
    radius: f32,
    screen: &Screen,
    clips: &[Rect],
    tessellate: impl FnOnce(&mut Vec<GlyphVertex>, Rect, [f32; 4], f32, &Screen),
) {
    let mut tessellated = Vec::new();
    tessellate(&mut tessellated, rect, color, radius, screen);
    append_clipped_triangles(vertices, &tessellated, clips, screen);
}
```

Implement `append_clipped_triangles` by clipping each three-vertex polygon against left, right, top, bottom NDC planes for every active clip, interpolating all `GlyphVertex` fields at intersection points, and fan-triangulating the remaining polygon. Use the existing `apply_clip` fast path only when `radius == 0.0`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p textora-app paint_backend::tests::clipped_rounded -- --exact`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/paint_backend.rs
git commit -m "fix(render): preserve rounded geometry under clip"
```

### Task 3: Integrated verification

**Files:**

- Modify: none

**Interfaces:**

- Consumes: the two completed production fixes and existing mmap canvas clip coverage.
- Produces: validated markdown and app crate builds without regressions.

- [ ] **Step 1: Format sources**

Run: `cargo fmt --check`

Expected: PASS after `cargo fmt` if formatting is required.

- [ ] **Step 2: Run focused regression tests**

Run: `cargo test -p textora-markdown mindmap_view::tests::canvas_render_clips_edge_drag_feedback_to_the_viewport -- --exact && cargo test -p textora-app paint_backend::tests --lib`

Expected: PASS.

- [ ] **Step 3: Compile affected crates**

Run: `cargo check -p textora-markdown && cargo check -p textora-app`

Expected: PASS.

- [ ] **Step 4: Commit documentation and verification-ready state**

```bash
git add docs/superpowers/specs/2026-07-21-mmap-caret-and-clip-geometry-design.md docs/superpowers/plans/2026-07-21-mmap-caret-and-clip-geometry.md
git commit -m "docs: plan mmap geometry fixes"
```
