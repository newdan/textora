# Text Rendering Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve text rendering quality via two independent optimizations: (1) subpixel variant caching for smoother kerning, (2) dynamic contrast enhancement with theme-aware gamma correction.

**Architecture:** Group 1 (subpixel) introduces a `split_subpixel()` helper that decomposes float coordinates into integer pixel positions + 0-3 phase, passes phase through the atlas key to swash's `Render::offset()` for subpixel-shifted bitmaps. Group 2 (contrast) adds a `GammaUniform` uniform buffer at `@binding(2)` and replaces the hardcoded `mix()` in the fragment shader with Zed-style brightness-threshold interpolation, deriving gamma from the active theme.

**Tech Stack:** Rust, wgpu, WGSL, swash 0.1.19, cosmic-text 0.12

---

## File Structure

| File | Group | Role |
|------|-------|------|
| `crates/render/src/lib.rs` | 1, 2 | `split_subpixel()` helper, GlyphKey, GlyphRenderer, SHADER_SRC, GammaUniform |
| `crates/shaping/src/lib.rs` | 1 | `rasterize_glyph()` — add `subpixel_offset` param |
| `crates/app/src/text_rasterize.rs` | 1 | `resolve_glyph()` — accept phase, encode in GlyphKey |
| `crates/app/src/render_cache.rs` | 1 | `emit_vertices_for_visual_line` — use integer coords |
| `crates/app/src/paint_backend.rs` | 1 | `emit_text` — split coords, pass phase |
| `crates/app/src/render_pipeline.rs` | 1 | Cache population — split coords, phase-aware atlas lookup |
| `crates/app/src/render_state.rs` | 2 | Create gamma uniform buffer + updated bind group |
| `crates/app/src/app_renderer.rs` | 2 | Update gamma uniform on theme change |

---

## Group 1: Subpixel Variant Caching

### Task 1: Add `split_subpixel` helper to render crate

**Files:**
- Modify: `crates/render/src/lib.rs`

- [ ] **Step 1: Add the `split_subpixel` function**

Insert after the `GlyphAtlas` impl block (after line 228, before `#[cfg(test)]`):

```rust
/// Split a coordinate into integer pixel position and subpixel phase (0-3).
///
/// Quantizes to 1/4-pixel grid: `(coord * 4).round() / 4`, then returns the
/// integer-truncated position and the fractional phase (0..4).
pub fn split_subpixel(coord: f32) -> (f32, u8) {
    let sub = (coord * 4.0).round() / 4.0;
    let int_part = sub.trunc();
    let phase = ((sub - int_part) * 4.0) as u8;
    (int_part, phase)
}
```

- [ ] **Step 2: Add unit tests for `split_subpixel`**

Append inside the existing `#[cfg(test)] mod tests` block in `crates/render/src/lib.rs`:

```rust
#[test]
fn split_subpixel_exact_integer() {
    assert_eq!(split_subpixel(10.0), (10.0, 0));
    assert_eq!(split_subpixel(0.0), (0.0, 0));
}

#[test]
fn split_subpixel_quarter() {
    assert_eq!(split_subpixel(10.25), (10.0, 1));
    assert_eq!(split_subpixel(10.5), (10.0, 2));
    assert_eq!(split_subpixel(10.75), (10.0, 3));
}

#[test]
fn split_subpixel_rounding() {
    // 10.3 → 10.25 (quantized) → int=10, phase=1
    let (int_part, phase) = split_subpixel(10.3);
    assert_eq!(int_part, 10.0);
    assert_eq!(phase, 1);
    // 10.1 → 10.0
    let (int_part, phase) = split_subpixel(10.1);
    assert_eq!(int_part, 10.0);
    assert_eq!(phase, 0);
}

#[test]
fn split_subpixel_negative() {
    let (int_part, phase) = split_subpixel(-0.3);
    // -0.3 → -0.25 (quantized) → int=-1? or 0?
    // For text rendering, x is always >= 0 from left margin.
    // This test just verifies the function doesn't panic.
    let _ = (int_part, phase);
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p render split_subpixel
```

Expected: 4 tests PASS

- [ ] **Step 4: Commit**

```bash
git add crates/render/src/lib.rs
git commit -m "feat(render): add split_subpixel helper for 1/4-pixel coordinate quantization"
```

---

### Task 2: Add subpixel_offset to Shaper::rasterize_glyph

**Files:**
- Modify: `crates/shaping/src/lib.rs`

- [ ] **Step 1: Add `subpixel_offset` parameter**

Change the signature of `rasterize_glyph` (line 449-454):

```rust
/// Rasterize a glyph to an alpha bitmap using swash.
///
/// `subpixel_offset` is the fractional-pixel offset (x, y) for subpixel positioning.
/// Pass `(0.0, 0.0)` for no offset.
///
/// Returns `None` if the glyph cannot be rasterized (e.g., space character).
pub fn rasterize_glyph(
    &mut self,
    font_id: cosmic_text::fontdb::ID,
    glyph_id: u16,
    font_size: f32,
    subpixel_offset: (f32, f32),
) -> Option<GlyphBitmap> {
```

- [ ] **Step 2: Pass offset to swash Render**

Replace the Render builder call (lines 464-470) to include `.offset()`:

```rust
    // Render with grayscale alpha mask
    let image = swash::scale::Render::new(&[
        swash::scale::Source::ColorOutline(0),
        swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
        swash::scale::Source::Outline,
    ])
    .format(swash::zeno::Format::Alpha)
    .offset(swash::zeno::Vector::new(subpixel_offset.0, subpixel_offset.1))
    .render(&mut scaler, glyph_id)?;
```

- [ ] **Step 3: Run existing tests**

```bash
cargo test -p shaping
```

Expected: all existing tests PASS (they call `rasterize_glyph` with `(0.0, 0.0)` after updating callers — see next step)

- [ ] **Step 4: Fix callers**

Search for all callers of `rasterize_glyph` and add `(0.0, 0.0)` as the last argument. The only caller should be in `text_rasterize.rs` (which we'll update in Task 3). There may be dead references — fix them:

```bash
grep -rn "rasterize_glyph" crates/
```

Expected: 1 caller in `text_rasterize.rs` (the definition in shaping + the call site). Update the call site to pass `(0.0, 0.0)` temporarily:

In `crates/app/src/text_rasterize.rs` line 43, change:
```rust
let bitmap = shaper.rasterize_glyph(font_id, glyph_id, font_size)?;
```
to:
```rust
let bitmap = shaper.rasterize_glyph(font_id, glyph_id, font_size, (0.0, 0.0))?;
```

- [ ] **Step 5: Run full test suite to confirm nothing broke**

```bash
cargo test -p shaping -p app
```

- [ ] **Step 6: Commit**

```bash
git add crates/shaping/src/lib.rs crates/app/src/text_rasterize.rs
git commit -m "feat(shaping): add subpixel_offset parameter to rasterize_glyph"
```

---

### Task 3: Thread subpixel phase through text_rasterize::resolve_glyph

**Files:**
- Modify: `crates/app/src/text_rasterize.rs`

- [ ] **Step 1: Add `subpixel_phase` parameter to `resolve_glyph`**

Change the function signature (line 13-21):

```rust
/// Look up a glyph in the atlas, or rasterize + upload it on cache miss.
///
/// `subpixel_phase` is the X subpixel phase (0-3) for atlas variant selection.
///
/// Returns `Some(GlyphSlot)` with position in the atlas texture, or `None` if
/// rasterization failed, the glyph bitmap was empty, or atlas insertion failed.
pub(crate) fn resolve_glyph(
    font_id: FontId,
    glyph_id: u16,
    font_size: f32,
    subpixel_phase: u8,
    shaper: &mut Shaper,
    atlas: &mut GlyphAtlas,
    atlas_texture: &wgpu::Texture,
    queue: &wgpu::Queue,
) -> Option<GlyphSlot> {
```

- [ ] **Step 2: Use subpixel_phase in GlyphKey and rasterization**

Replace lines 30-43:

```rust
    let key = GlyphKey {
        glyph_id: glyph_id as u32,
        font_id: font_id_usize,
        font_size: (font_size * 64.0) as u32,
        subpixel_phase,
    };

    // Cache hit
    if let Some(cached) = atlas.get(&key) {
        return Some(*cached);
    }

    // Cache miss: rasterize with subpixel offset
    let offset_x = subpixel_phase as f32 / 4.0;
    let bitmap = shaper.rasterize_glyph(font_id, glyph_id, font_size, (offset_x, 0.0))?;
```

- [ ] **Step 3: Update all callers of `resolve_glyph` to pass `subpixel_phase`**

Find callers:
```bash
grep -rn "resolve_glyph" crates/
```

There are 3 call sites. For now, pass `0` at each to keep behavior unchanged (we'll update them in subsequent tasks):

In `crates/app/src/paint_backend.rs` line 137-140:
```rust
let Some(slot) = crate::text_rasterize::resolve_glyph(
    cluster.font_id, cluster.glyph_id as u16, font_size,
    0, // subpixel_phase — will be computed in Task 6
    &mut text.shaper, &mut text.atlas,
    &text.atlas_texture, &gpu.ctx.queue,
) else {
```

In `crates/app/src/render_pipeline.rs` line 791-794 (the non-cached path):
```rust
let Some(slot) = crate::text_rasterize::resolve_glyph(
    font_id, glyph_id, font_size,
    0, // subpixel_phase — will be computed in Task 5
    &mut text.shaper, &mut text.atlas,
    &text.atlas_texture, &gpu.ctx.queue,
) else {
```

In `crates/app/src/render_pipeline.rs` line 1039 (line number gutter):
```rust
let Some(slot) = crate::text_rasterize::resolve_glyph(
    cluster.font_id, cluster.glyph_id as u16, line_number_font_size,
    0, // subpixel_phase — line numbers don't need subpixel
    &mut text.shaper, &mut text.atlas,
    &text.atlas_texture, &gpu.ctx.queue,
) else {
```

- [ ] **Step 4: Build check**

```bash
cargo build -p app 2>&1 | head -40
```

Expected: compiles without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/text_rasterize.rs crates/app/src/paint_backend.rs crates/app/src/render_pipeline.rs
git commit -m "feat(text_rasterize): thread subpixel_phase through resolve_glyph"
```

---

### Task 4: Update render_cache vertex emission with subpixel coordinates

**Files:**
- Modify: `crates/app/src/render_cache.rs`

- [ ] **Step 1: Remove `.round()` from vertex emission**

In `emit_vertices_for_visual_line` (lines 118-119), replace:

```rust
let px = (inst_x + inst.bearing_x).round();
let py = (y_base - inst.bearing_y).round();
```

with:

```rust
// Coordinates are already quantized to 1/4-pixel grid during cache population;
// inst.x and bearing are integer-aligned. No .round() needed.
let px = inst_x + inst.bearing_x;
let py = y_base - inst.bearing_y;
```

- [ ] **Step 2: Build check**

```bash
cargo build -p app 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/render_cache.rs
git commit -m "fix(render_cache): remove .round() from vertex emission, coords now pre-quantized"
```

---

### Task 5: Update render_pipeline cache population with subpixel phases

**Files:**
- Modify: `crates/app/src/render_pipeline.rs`

- [ ] **Step 1: Compute subpixel phase during non-cached glyph rendering**

Around line 818 (the `generate_vertices` call), split x coordinate before resolving:

```rust
// Split x position for subpixel positioning
let (px, phase_x) = render::split_subpixel(render_x + cluster.x_offset);

let Some(slot) = crate::text_rasterize::resolve_glyph(
    font_id, glyph_id, font_size,
    phase_x,
    &mut text.shaper, &mut text.atlas,
    &text.atlas_texture, &gpu.ctx.queue,
) else {
    x_cursor += advance;
    continue;
};

let verts = GlyphRenderer::generate_vertices(
    &[(slot, px, y_base)],
    ATLAS_SIZE,
    ATLAS_SIZE,
    ctx.screen_w,
    ctx.screen_h,
    color,
);
```

- [ ] **Step 2: Update cache population code to use subpixel phase**

Around line 868-897 (where `GlyphKey` is constructed with `subpixel_phase: 0`), split the x coordinate and use the phase:

```rust
let (px_c, phase_c) = render::split_subpixel(x_c + cluster.x_offset);

let c_key = GlyphKey {
    glyph_id: cluster.glyph_id,
    font_id: c_fid,
    font_size: (c_fs * 64.0) as u32,
    subpixel_phase: phase_c,
};

if let Some(c_slot) = text.atlas.get(&c_key) {
    let aw = ATLAS_SIZE as f32;
    let ah = ATLAS_SIZE as f32;
    all_instances.push(GlyphInstance {
        x: px_c,  // store integer position
        y: 0.0,
        bearing_x: c_slot.bearing_x,
        // ... rest unchanged
    });
}
```

Note: if the atlas lookup misses here (cache population path), we skip the glyph rather than rasterizing. This is existing behavior — during cache population we only store glyphs already in the atlas. The non-cached path above handles rasterization.

- [ ] **Step 3: Build check**

```bash
cargo build -p app 2>&1 | head -20
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "feat(render_pipeline): use subpixel phase during glyph resolution and cache population"
```

---

### Task 6: Update paint_backend emit_text with subpixel coordinates

**Files:**
- Modify: `crates/app/src/paint_backend.rs`

- [ ] **Step 1: Split coordinates in emit_text**

Around line 137-148, replace the resolve_glyph call and coordinate computation:

```rust
// Split x position for subpixel positioning
let (px, phase_x) = render::split_subpixel(x_cursor);

let Some(slot) = crate::text_rasterize::resolve_glyph(
    cluster.font_id, cluster.glyph_id as u16, font_size,
    phase_x,
    &mut text.shaper, &mut text.atlas,
    &text.atlas_texture, &gpu.ctx.queue,
) else {
    x_cursor += advance;
    continue;
};

// Use integer positions (px) instead of float x_cursor for quad placement
let g_l = px + slot.bearing_x;
let g_t = y_baseline - slot.bearing_y;
```

- [ ] **Step 2: Build check**

```bash
cargo build -p app 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/paint_backend.rs
git commit -m "feat(paint_backend): use subpixel phase for UI chrome text rendering"
```

---

### Task 7: Integration test — subpixel variant caching

**Files:**
- Modify: `crates/render/src/lib.rs` (add test)

- [ ] **Step 1: Add integration-style test for split_subpixel + atlas key differentiation**

Append in the test module:

```rust
#[test]
fn subpixel_phase_produces_distinct_atlas_entries() {
    let mut atlas = GlyphAtlas::new(256, 256, 100, 4);

    // Same glyph at 4 subpixel phases → 4 distinct atlas entries
    for phase in 0..4u8 {
        let key = GlyphKey {
            glyph_id: 42,
            font_id: 0,
            font_size: 14 * 64,
            subpixel_phase: phase,
        };
        assert!(atlas.insert(key, 10, 10, 0.0, 0.0).is_some(),
            "phase {phase} should get its own atlas entry");
    }

    assert_eq!(atlas.glyph_count(), 4, "4 phases → 4 entries");
}

#[test]
fn subpixel_phase_round_trip() {
    // Simulate the coordinate → phase → atlas key flow
    let x_cursor = 10.3f32;
    let (px, phase) = split_subpixel(x_cursor);
    assert_eq!(phase, 1); // 10.3 → 10.25 quantized → phase 1
    assert_eq!(px, 10.0);

    // Building the atlas key
    let key = GlyphKey {
        glyph_id: 1,
        font_id: 0,
        font_size: 14 * 64,
        subpixel_phase: phase,
    };
    assert_eq!(key.subpixel_phase, 1);
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p render subpixel_phase
```

Expected: all tests PASS

- [ ] **Step 3: Run full test suite**

```bash
cargo test --workspace
```

- [ ] **Step 4: Commit**

```bash
git add crates/render/src/lib.rs
git commit -m "test(render): add subpixel phase round-trip and atlas entry tests"
```

---

## Group 2: Dynamic Contrast Enhancement

### Task 8: Add GammaUniform + update shader in render crate

**Files:**
- Modify: `crates/render/src/lib.rs`

- [ ] **Step 1: Add `GammaUniform` struct after `GlyphVertex` (after line 435)**

```rust
/// Uniform buffer for gamma correction and contrast parameters.
/// Passed to the fragment shader at @binding(2).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GammaUniform {
    /// Base contrast multiplier. The shader interpolates this based on
    /// foreground brightness (0 = no enhancement, 1 = full).
    pub contrast: f32,
    /// Gamma exponent denominator: `pow(alpha, 1.0 / gamma)`.
    /// Light theme: ~1.45, dark theme: ~2.2.
    pub gamma: f32,
}
```

- [ ] **Step 2: Add binding 2 to the bind group layout in `GlyphRenderer::new`**

In the `bind_group_layout` entries array (after line 473, after the sampler entry), add:

```rust
wgpu::BindGroupLayoutEntry {
    binding: 2,
    visibility: wgpu::ShaderStages::FRAGMENT,
    ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: Some(std::num::NonZeroU64::new(
            std::mem::size_of::<GammaUniform>() as u64,
        ).unwrap()),
    },
    count: None,
},
```

- [ ] **Step 3: Update the pipeline layout to include the new bind group layout entry**

The pipeline layout already references `&bind_group_layout` which now has 3 entries. No change needed — wgpu handles this automatically since we added the entry before building the pipeline layout.

Wait — check the order. The bind_group_layout is created BEFORE the pipeline_layout. Since we added entry 2 before `create_pipeline_layout`, the layout is correctly updated. Good.

- [ ] **Step 4: Replace SHADER_SRC with the updated shader**

Replace the entire `SHADER_SRC` constant (lines 606-655):

```rust
const SHADER_SRC: &str = r#"
fn color_brightness(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.299, 0.587, 0.114));
}

fn enhance_contrast(alpha: f32, k: f32) -> f32 {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

fn light_on_dark_contrast(base_contrast: f32, color: vec3<f32>) -> f32 {
    let brightness = color_brightness(color);
    // Smooth falloff: full contrast for dark text (brightness → 0),
    // zero contrast for bright text (brightness ≥ 0.75).
    let multiplier = saturate(4.0 * (0.75 - brightness));
    return base_contrast * multiplier;
}

struct GammaUniform {
    contrast: f32,
    gamma: f32,
}

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    return out;
}

@group(0) @binding(0) var atlas_texture: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var<uniform> gamma_params: GammaUniform;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas_texture, atlas_sampler, in.tex_coords).r;

    // Dynamic contrast: brightness-aware stem darkening.
    // Dark text on light backgrounds gets full contrast enhancement;
    // light text on dark backgrounds gets reduced enhancement to avoid over-bolding.
    let dilation = light_on_dark_contrast(gamma_params.contrast, in.color.rgb);
    let alpha_corrected = enhance_contrast(coverage, dilation);

    // Gamma correction to counter sRGB linear blending.
    let final_alpha = pow(alpha_corrected, 1.0 / gamma_params.gamma);
    return vec4<f32>(in.color.rgb, in.color.a * final_alpha);
}
"#;
```

- [ ] **Step 5: Update the existing GPU test to handle the new binding**

The test `renderer_creation` creates a renderer and checks it exists. The new binding entry requires a corresponding buffer when creating bind groups. But this test doesn't create bind groups — it only creates the renderer. So no change needed for tests.

- [ ] **Step 6: Run render tests**

```bash
cargo test -p render
```

Expected: all tests PASS (renderer_creation test still passes because it doesn't create a bind group)

- [ ] **Step 7: Commit**

```bash
git add crates/render/src/lib.rs
git commit -m "feat(render): add GammaUniform, binding(2), and Zed-style contrast interpolation in shader"
```

---

### Task 9: Create gamma uniform buffer in TextState

**Files:**
- Modify: `crates/app/src/render_state.rs`

- [ ] **Step 1: Add gamma fields to TextState**

After the `vertex_capacity` field (line 42), add:

```rust
    /// Gamma correction uniform buffer (updated on theme change).
    pub(crate) gamma_buffer: wgpu::Buffer,
```

- [ ] **Step 2: Create the gamma uniform buffer and updated bind group**

In `TextState::init`, after creating the vertex buffer (after line 114), add:

```rust
// Gamma uniform buffer — initial values for light theme.
let gamma_uniform = render::GammaUniform {
    contrast: 1.0,
    gamma: 1.45,
};
let gamma_buffer = gpu.ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some("gamma uniform"),
    contents: bytemuck::cast_slice(&[gamma_uniform]),
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
});
```

- [ ] **Step 3: Update the bind group to include binding 2**

Replace the bind group creation (lines 93-106) to include the gamma buffer:

```rust
let bind_group = gpu.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("atlas bind group"),
    layout: renderer.bind_group_layout(),
    entries: &[
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&atlas_view),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(renderer.sampler()),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: gamma_buffer.as_entire_binding(),
        },
    ],
});
```

- [ ] **Step 4: Return gamma_buffer in the Ok(Self { ... })**

Add `gamma_buffer` to the struct literal.

- [ ] **Step 5: Build check**

```bash
cargo build -p app 2>&1 | head -20
```

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/render_state.rs
git commit -m "feat(render_state): create gamma uniform buffer and add binding(2) to bind group"
```

---

### Task 10: Update gamma uniform on theme change in app_renderer

**Files:**
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: Add gamma update logic in the render method**

Find where `render_pass.set_bind_group(0, &text.bind_group, &[])` is called (line 533). Before this line, add theme-aware gamma update:

```rust
// Update gamma uniform based on current theme's background brightness.
// Dark backgrounds (brightness < 0.5) get higher gamma to prevent over-bolding.
{
    let bg = self.current_theme.background;
    let bg_brightness = 0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2];
    let gamma = if bg_brightness < 0.5 { 2.2 } else { 1.45 };
    let gamma_uniform = render::GammaUniform {
        contrast: 1.0,
        gamma,
    };
    gpu.ctx.queue.write_buffer(
        &text.gamma_buffer,
        0,
        bytemuck::cast_slice(&[gamma_uniform]),
    );
}
```

- [ ] **Step 2: Build check**

```bash
cargo build -p app 2>&1 | head -20
```

- [ ] **Step 3: Full build and test**

```bash
cargo build --workspace 2>&1 | tail -20
cargo test --workspace 2>&1 | tail -30
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "feat(app_renderer): update gamma uniform based on theme background brightness"
```

---

## Verification

- [ ] **Compile:** `cargo build --workspace` — no errors
- [ ] **Tests:** `cargo test --workspace` — all tests pass
- [ ] **Visual smoke test:** Run the app, verify text renders correctly in both light and dark themes
- [ ] **Subpixel check:** Inspect text at various font sizes (12-24px) for even character spacing
- [ ] **Contrast check:** Switch between light/dark themes — verify text is neither washed out nor over-bold
