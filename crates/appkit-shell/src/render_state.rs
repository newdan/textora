//! Render resource state — GPU context and text rendering resources.
//!
//! Extracted from `app.rs` to separate rendering infrastructure from
//! application lifecycle orchestration.

use crate::gpu::{GpuContext, GpuError};
use crate::render_cache::PreviewRenderCache;
use wgpu::util::DeviceExt;

/// Atlas texture size for glyph caching.
pub const ATLAS_SIZE: u32 = 4096;
const _: () = assert!(ATLAS_SIZE > 0);

/// GPU resources for a single window.
pub struct GpuState {
    pub ctx: GpuContext,
    pub size: winit::dpi::PhysicalSize<u32>,
}

/// Text rendering resources (created after GPU init).
pub struct TextState {
    pub renderer: render::GlyphRenderer,
    pub shaper: shaping::Shaper,
    pub atlas: render::GlyphAtlas,
    pub atlas_texture: wgpu::Texture,
    #[allow(dead_code)] // kept alive for bind group reference
    atlas_view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_capacity: u32,
    /// Gamma correction uniform buffer (updated on theme change).
    gamma_buffer: wgpu::Buffer,
    /// Last written gamma values, for dedup (avoids per-frame write_buffer).
    cached_gamma: render::GammaUniform,
    /// Preview render cache keyed by UiTextLayout.id
    pub preview_cache: PreviewRenderCache,
    /// Atlas generation counter — increments when atlas evictions are likely.
    /// Used to invalidate stale CachedLine entries.
    pub atlas_generation: u64,
    /// Counter for periodic generation bumps.
    glyph_resolve_count: u64,
}

impl TextState {
    /// Create all text rendering resources: atlas, renderer, shaper, buffers.
    /// `font_system` is the shared FontSystem (Arc<Mutex<FontSystem>>) created once at startup.
    pub fn init(
        gpu: &GpuState,
        font_size: f32,
        font_system: std::sync::Arc<std::sync::Mutex<shaping::FontSystem>>,
        font_family: &str,
    ) -> Result<Self, GpuError> {
        let renderer = render::GlyphRenderer::new(&gpu.ctx.device, gpu.ctx.format);

        let atlas_texture = gpu.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas texture"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Write a solid white pixel at atlas (0,0) for cursor/caret rendering.
        gpu.ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &[255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );

        // Gamma uniform buffer — initial values for light theme.
        let gamma_uniform = render::GammaUniform { contrast: 1.0, gamma: 1.45 };
        let gamma_buffer = gpu.ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gamma uniform"),
            contents: bytemuck::cast_slice(&[gamma_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

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
                wgpu::BindGroupEntry { binding: 2, resource: gamma_buffer.as_entire_binding() },
            ],
        });

        let vertex_capacity = 1024 * 6; // 1024 glyphs * 6 vertices each
        let vertex_buffer = gpu.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertex buffer"),
            size: (vertex_capacity as usize * std::mem::size_of::<render::GlyphVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shaper = shaping::Shaper::from_shared_font_system(font_system, font_size, font_family);

        Ok(Self {
            renderer,
            shaper,
            bind_group,
            vertex_buffer,
            vertex_capacity,
            atlas: render::GlyphAtlas::new(ATLAS_SIZE, ATLAS_SIZE, 8192, 1),
            atlas_texture,
            atlas_view,
            gamma_buffer,
            cached_gamma: gamma_uniform,
            preview_cache: PreviewRenderCache::new(),
            atlas_generation: 1,
            glyph_resolve_count: 0,
        })
    }

    /// Update gamma uniform buffer only if values have changed.
    /// Returns true if the buffer was updated.
    pub fn update_gamma_if_changed(
        &mut self,
        queue: &wgpu::Queue,
        new_gamma: render::GammaUniform,
    ) -> bool {
        if self.cached_gamma.contrast == new_gamma.contrast
            && self.cached_gamma.gamma == new_gamma.gamma
        {
            return false;
        }
        self.cached_gamma = new_gamma;
        queue.write_buffer(&self.gamma_buffer, 0, bytemuck::cast_slice(&[new_gamma]));
        true
    }

    /// Track glyph resolution — bumps atlas generation every 5000 calls.
    /// This ensures stale PreviewRenderCache entries are eventually invalidated
    /// after atlas evictions.
    pub fn track_glyph_resolve(&mut self) {
        self.glyph_resolve_count += 1;
        if self.glyph_resolve_count >= 5000 {
            self.atlas_generation += 1;
            self.glyph_resolve_count = 0;
            self.preview_cache.invalidate_stale_atlas(self.atlas_generation);
        }
    }
}
