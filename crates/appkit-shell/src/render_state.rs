//! Render resource state — GPU context and text rendering resources.
//!
//! Extracted from `app.rs` to separate rendering infrastructure from
//! application lifecycle orchestration.

use crate::gpu::{GpuContext, GpuError};
use crate::image_atlas::ImageAtlas;
use crate::render_cache::PreviewRenderCache;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::time::Instant;
use wgpu::util::DeviceExt;

/// Atlas texture size for glyph caching.
pub const ATLAS_SIZE: u32 = 4096;
const _: () = assert!(ATLAS_SIZE > 0);

const INITIAL_VERTEX_BUFFER_BYTES: u64 =
    1024 * 6 * std::mem::size_of::<render::GlyphVertex>() as u64;
const MAPPED_VERTEX_SLOTS: usize = 3;

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
    pub image_atlas: ImageAtlas,
    #[allow(dead_code)] // kept alive for bind group reference
    atlas_view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    pub(crate) frame_vertices: FrameVertexBuffers,
    /// Gamma correction uniform buffer (updated on theme change).
    gamma_buffer: wgpu::Buffer,
    /// Last written gamma values, for dedup (avoids per-frame write_buffer).
    cached_gamma: render::GammaUniform,
    /// Preview render cache keyed by UiTextLayout.id
    pub preview_cache: PreviewRenderCache,
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
        let initialization_started_at = Instant::now();
        let renderer = render::GlyphRenderer::new_with_sample_count(
            &gpu.ctx.device,
            gpu.ctx.format,
            gpu.ctx.antialiasing.sample_count(),
        );
        let renderer_elapsed = initialization_started_at.elapsed();

        let glyph_atlas_started_at = Instant::now();
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
        let glyph_atlas_elapsed = glyph_atlas_started_at.elapsed();
        let image_atlas_started_at = Instant::now();
        let image_atlas = ImageAtlas::new(&gpu.ctx.device);
        let image_atlas_elapsed = image_atlas_started_at.elapsed();

        let bindings_started_at = Instant::now();
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
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(image_atlas.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(renderer.image_sampler()),
                },
            ],
        });

        let frame_vertices = FrameVertexBuffers::new(&gpu.ctx.device);
        let bindings_elapsed = bindings_started_at.elapsed();

        let shaper_started_at = Instant::now();
        let shaper = shaping::Shaper::from_shared_font_system(font_system, font_size, font_family);
        let shaper_elapsed = shaper_started_at.elapsed();

        let text_state = Self {
            renderer,
            shaper,
            bind_group,
            frame_vertices,
            atlas: render::GlyphAtlas::new(ATLAS_SIZE, ATLAS_SIZE, 8192, 1),
            atlas_texture,
            image_atlas,
            atlas_view,
            gamma_buffer,
            cached_gamma: gamma_uniform,
            preview_cache: PreviewRenderCache::new(),
        };
        eprintln!(
            "[startup:text_init] renderer={:.2}ms glyph_atlas={:.2}ms image_atlas={:.2}ms bindings={:.2}ms shaper={:.2}ms total={:.2}ms",
            renderer_elapsed.as_secs_f64() * 1_000.0,
            glyph_atlas_elapsed.as_secs_f64() * 1_000.0,
            image_atlas_elapsed.as_secs_f64() * 1_000.0,
            bindings_elapsed.as_secs_f64() * 1_000.0,
            shaper_elapsed.as_secs_f64() * 1_000.0,
            initialization_started_at.elapsed().as_secs_f64() * 1_000.0,
        );
        Ok(text_state)
    }

    /// Image entries used by a frame remain pinned until the next frame starts.
    pub fn begin_frame(&mut self) {
        self.image_atlas.begin_frame();
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

    pub fn atlas_generation(&self) -> u64 {
        self.atlas.generation()
    }
}

pub(crate) struct FrameVertexBuffers {
    mapped: Option<[MappedVertexSlot; MAPPED_VERTEX_SLOTS]>,
    staged: Option<wgpu::Buffer>,
}

impl FrameVertexBuffers {
    fn new(device: &wgpu::Device) -> Self {
        let mapped =
            device.features().contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS).then(|| {
                std::array::from_fn(|_| MappedVertexSlot::new(device, INITIAL_VERTEX_BUFFER_BYTES))
            });
        Self { mapped, staged: None }
    }

    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &wgpu::CommandEncoder,
        vertices: &[u8],
    ) -> &wgpu::Buffer {
        debug_assert!(!vertices.is_empty(), "frame vertex upload requires non-empty geometry");
        if let Some(slots) = &mut self.mapped {
            // Poll without waiting if all shared slots are still in flight.
            if !slots.iter().any(MappedVertexSlot::is_reusable) {
                let _ = device.poll(wgpu::PollType::Poll);
            }
            if let Some(slot) = slots.iter_mut().find(|slot| slot.is_reusable()) {
                return slot.write(device, encoder, vertices);
            }
        }
        let size = vertices.len() as u64;
        if self.staged.as_ref().is_none_or(|buffer| buffer.size() < size) {
            self.staged = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("staged editor vertex buffer"),
                size: size.next_power_of_two().max(INITIAL_VERTEX_BUFFER_BYTES),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.staged.as_ref().expect("the staged vertex buffer was allocated above");
        queue.write_buffer(buffer, 0, vertices);
        buffer
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MappedSlotState {
    InFlight = 0,
    Ready = 1,
    Failed = 2,
}

impl MappedSlotState {
    fn from_u8(raw: u8) -> Self {
        match raw {
            0 => Self::InFlight,
            1 => Self::Ready,
            2 => Self::Failed,
            _ => Self::Failed,
        }
    }
}

struct MappedVertexSlot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

impl MappedVertexSlot {
    fn new(device: &wgpu::Device, size: u64) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shared editor vertex buffer"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::MAP_WRITE,
            mapped_at_creation: true,
        });
        Self { buffer, state: Arc::new(AtomicU8::new(MappedSlotState::Ready as u8)) }
    }

    fn load_state(&self) -> MappedSlotState {
        MappedSlotState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn store_state(&self, state: MappedSlotState) {
        self.state.store(state as u8, Ordering::Release);
    }

    fn is_reusable(&self) -> bool {
        matches!(self.load_state(), MappedSlotState::Ready | MappedSlotState::Failed)
    }

    fn write(
        &mut self,
        device: &wgpu::Device,
        encoder: &wgpu::CommandEncoder,
        vertices: &[u8],
    ) -> &wgpu::Buffer {
        let size = vertices.len() as u64;
        if self.load_state() == MappedSlotState::Failed || self.buffer.size() < size {
            *self = Self::new(device, size.next_power_of_two().max(INITIAL_VERTEX_BUFFER_BYTES));
        }
        self.buffer.slice(..size).get_mapped_range_mut().copy_from_slice(vertices);
        self.buffer.unmap();
        self.store_state(MappedSlotState::InFlight);
        let state = Arc::clone(&self.state);
        encoder.map_buffer_on_submit(&self.buffer, wgpu::MapMode::Write, .., move |result| {
            let next =
                if result.is_ok() { MappedSlotState::Ready } else { MappedSlotState::Failed };
            state.store(next as u8, Ordering::Release);
        });
        &self.buffer
    }
}

#[cfg(test)]
mod frame_vertex_tests {
    use super::*;

    #[test]
    fn failed_mapped_slot_recovers_as_reusable_while_in_flight_remains_blocked() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("skipping mapped vertex recovery test: no GPU adapter");
            return;
        };
        let feature = wgpu::Features::MAPPABLE_PRIMARY_BUFFERS;
        if !adapter.features().contains(feature) {
            eprintln!(
                "skipping mapped vertex recovery test: adapter has no primary mapping support"
            );
            return;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: feature,
            ..Default::default()
        }))
        .expect("adapter advertises primary mapping support");

        let mut slot = MappedVertexSlot::new(&device, INITIAL_VERTEX_BUFFER_BYTES);
        assert!(slot.is_reusable());
        slot.store_state(MappedSlotState::InFlight);
        assert!(!slot.is_reusable());

        slot.store_state(MappedSlotState::Failed);
        assert!(slot.is_reusable());

        let encoder = device.create_command_encoder(&Default::default());
        let vertex_bytes = [7; std::mem::size_of::<render::GlyphVertex>()];
        let _ = slot.write(&device, &encoder, &vertex_bytes);
        assert_eq!(slot.load_state(), MappedSlotState::InFlight);

        let submitted = queue.submit(std::iter::once(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None })
            .expect("recovered slot submission must complete");
        assert_eq!(slot.load_state(), MappedSlotState::Ready);
    }

    #[test]
    fn shared_vertex_slots_are_reused_only_after_the_submission_releases_them() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("skipping mapped vertex test: no GPU adapter");
            return;
        };
        let feature = wgpu::Features::MAPPABLE_PRIMARY_BUFFERS;
        if !adapter.features().contains(feature) {
            eprintln!("skipping mapped vertex test: adapter has no primary mapping support");
            return;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: feature,
            ..Default::default()
        }))
        .expect("adapter advertises primary mapping support");

        let mut uploads = FrameVertexBuffers::new(&device);
        let mut pending = Vec::new();
        let vertex_bytes = [42; std::mem::size_of::<render::GlyphVertex>()];
        for _ in 0..MAPPED_VERTEX_SLOTS {
            let encoder = device.create_command_encoder(&Default::default());
            let buffer = uploads.upload(&device, &queue, &encoder, &vertex_bytes).clone();
            assert!(buffer.usage().contains(wgpu::BufferUsages::MAP_WRITE));
            assert!(!pending.iter().any(|(previous, _)| previous == &buffer));
            pending.push((buffer, encoder.finish()));
        }

        let encoder = device.create_command_encoder(&Default::default());
        let staged = uploads.upload(&device, &queue, &encoder, &vertex_bytes);
        assert!(!staged.usage().contains(wgpu::BufferUsages::MAP_WRITE));

        let (first, commands) = pending.remove(0);
        let submitted = queue.submit(std::iter::once(commands));
        device
            .poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None })
            .expect("empty test submission with remapping must complete");
        assert_eq!(uploads.upload(&device, &queue, &encoder, &vertex_bytes), &first);
    }
}
