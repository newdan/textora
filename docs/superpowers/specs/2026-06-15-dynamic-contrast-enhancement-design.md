# Dynamic Contrast Enhancement for Text Rendering

**Date:** 2026-06-15
**Status:** design-approved
**Scope:** text rendering — dynamic stem darkening and gamma correction (optimization 2 of 2 from `plans-text-rendering-optimization.md`)

## Problem

The WGSL fragment shader in `crates/render/src/lib.rs` hardcodes contrast dilation and gamma correction:

- `dilation = mix(1.2, 0.2, brightness)` — fixed linear interpolation
- `pow(alpha, 1.0 / 1.45)` — fixed gamma
- No awareness of dark vs. light theme background

This causes light-on-dark text to appear over-bold and dark-on-light text to appear washed out, because the same parameters apply regardless of the surrounding background luminance.

## Solution

Two-part change:

1. **Shader:** replace the hardcoded `mix()` with Zed's smooth brightness-threshold interpolation, so contrast enhancement is proportional to foreground brightness at the per-glyph level.
2. **Uniform buffer:** pass a small `GammaUniform` (contrast, gamma) from Rust to the shader, allowing theme-aware adjustment without shader recompilation.

No user-facing settings. Parameters are derived automatically from the active theme.

## Design

### Uniform buffer

```rust
// Rust side (crates/render/src/lib.rs)
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GammaUniform {
    contrast: f32,  // base contrast multiplier (default 1.0)
    gamma: f32,     // gamma exponent for pow(alpha, 1/gamma) (default 1.45)
}
```

```wgsl
// WGSL side (SHADER_SRC)
struct GammaUniform {
    contrast: f32,
    gamma: f32,
}

@group(0) @binding(2) var<uniform> gamma_params: GammaUniform;
```

### Revised fragment shader

```wgsl
fn light_on_dark_contrast(base_contrast: f32, color: vec3<f32>) -> f32 {
    let brightness = color_brightness(color);
    let multiplier = saturate(4.0 * (0.75 - brightness));
    return base_contrast * multiplier;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas_texture, atlas_sampler, in.tex_coords).r;
    let dilation = light_on_dark_contrast(gamma_params.contrast, in.color.rgb);
    let alpha_corrected = enhance_contrast(coverage, dilation);
    let final_alpha = pow(alpha_corrected, 1.0 / gamma_params.gamma);
    return vec4<f32>(in.color.rgb, in.color.a * final_alpha);
}
```

### Theme-aware gamma selection

In `GlyphRenderer` or the render state initialization, set uniform values based on the active theme:

| Theme | contrast | gamma | Rationale |
|-------|----------|-------|-----------|
| Light | 1.0 | 1.45 | Stronger stem darkening for dark-on-light text |
| Dark  | 1.0 | 2.2  | Less aggressive gamma to prevent light-on-dark over-bolding |

The brightness interpolation in the shader handles per-glyph variation. The uniform values provide the base parameters that differ by overall theme background.

### Uniform buffer lifecycle

- Created once during `GlyphRenderer` initialization (or `TextState::init`)
- Updated on theme change (winit `ThemeChanged` event → `queue.write_buffer`)
- Bound at `@group(0) @binding(2)` in the existing bind group

## Files changed

| File | Change |
|------|--------|
| `crates/render/src/lib.rs` | Add `GammaUniform` struct; create uniform buffer + bind group layout entry; update `SHADER_SRC` with `light_on_dark_contrast` and `gamma_params` uniform; expose method to update uniform on theme change |
| `crates/app/src/render_state.rs` | Initialize gamma uniform buffer during `TextState::init`; update on theme transition |
| `crates/app/src/app_renderer.rs` | Pass current gamma uniform to bind group during render |

## Not in scope

- User-configurable gamma/contrast via settings.toml (uniform buffer infrastructure supports future addition)
- LCD subpixel rendering (Format::Subpixel)
