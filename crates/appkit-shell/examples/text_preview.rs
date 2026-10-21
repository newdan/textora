//! Offscreen text rendering harness.
//!
//! Renders a single text run through the same glyph atlas + shader path the
//! application uses, without opening a window. Useful for calibrating coverage
//! curves against native Windows output at any font size.
//!
//! Usage:
//!   cargo run -p textora-appkit-shell --example text_preview -- \
//!     <out.png> <font-family> <font-size-px> <text> <gamma> <contrast> [light|dark]

use render::{GammaUniform, GlyphRenderer, GlyphVertex};
use shaping::Shaper;
use wgpu::util::DeviceExt;

const ATLAS_SIZE: u32 = 1024;
const PADDING: f32 = 8.0;
/// Baseline position inside a sheet cell, leaving room for descenders.
const SHEET_BASELINE_RATIO: f32 = 0.78;

struct PackedGlyph {
    atlas_x: u32,
    atlas_y: u32,
    width: u32,
    height: u32,
    bearing_x: i32,
    bearing_y: i32,
}

struct AtlasPacker {
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    pixels: Vec<u8>,
}

impl AtlasPacker {
    fn new() -> Self {
        Self {
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE) as usize],
        }
    }

    fn insert(&mut self, bitmap: &shaping::GlyphBitmap) -> Option<PackedGlyph> {
        if bitmap.width == 0 || bitmap.height == 0 || bitmap.width > ATLAS_SIZE {
            return None;
        }
        if self.cursor_x + bitmap.width > ATLAS_SIZE {
            self.cursor_x = 0;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }
        if self.cursor_y + bitmap.height > ATLAS_SIZE {
            return None;
        }
        let x = self.cursor_x;
        let y = self.cursor_y;
        for row in 0..bitmap.height {
            let source = (row * bitmap.width) as usize;
            let destination = ((y + row) * ATLAS_SIZE + x) as usize;
            self.pixels[destination..destination + bitmap.width as usize]
                .copy_from_slice(&bitmap.data[source..source + bitmap.width as usize]);
        }
        self.cursor_x += bitmap.width + 1;
        self.row_height = self.row_height.max(bitmap.height);
        Some(PackedGlyph {
            atlas_x: x,
            atlas_y: y,
            width: bitmap.width,
            height: bitmap.height,
            bearing_x: bitmap.left,
            bearing_y: bitmap.top,
        })
    }
}

struct GlyphPlacement {
    packed: PackedGlyph,
    x: f32,
    y: f32,
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let output = arguments.get(1).cloned().unwrap_or_else(|| "text-preview.png".to_owned());
    let family = arguments.get(2).cloned().unwrap_or_else(|| "Segoe UI".to_owned());
    let size: f32 = arguments.get(3).and_then(|value| value.parse().ok()).unwrap_or(18.75);
    let text = arguments.get(4).cloned().unwrap_or_else(|| "中文汉字测试 Notora 2027".to_owned());
    // A leading `@` reads the text from a UTF-8 file, which keeps long or
    // punctuation-heavy samples out of the shell command line.
    let text = match text.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("text file {path} must be readable: {error}"))
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
        None => text,
    };
    let gamma: f32 = arguments.get(5).and_then(|value| value.parse().ok()).unwrap_or(0.714);
    let contrast: f32 = arguments.get(6).and_then(|value| value.parse().ok()).unwrap_or(0.0);
    let dark = arguments.get(7).is_some_and(|value| value == "dark");
    // `sheet:<columns>:<cell>` lays every glyph out in its own square cell so a
    // reference renderer can be compared glyph by glyph.
    let sheet = arguments.get(8).and_then(|value| value.strip_prefix("sheet:")).map(|value| {
        let mut parts = value.split(':');
        let columns: u32 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(50);
        let cell: f32 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(26.0);
        (columns.max(1), cell)
    });

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
        backends: wgpu::Backends::PRIMARY,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("text preview needs a GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("text preview device creation failed");

    let mut shaper =
        Shaper::new().expect("system font database must load").with_font_family(&family);
    shaper.set_font_size(size);
    let shaped = shaper.shape(&text).expect("preview text must shape");

    let mut packer = AtlasPacker::new();
    let mut placements = Vec::new();
    let mut pen_x = PADDING;
    let baseline_y = PADDING + size * 1.2;
    for (index, cluster) in shaped.clusters.iter().enumerate() {
        let Some(bitmap) = shaper.rasterize_glyph(
            cluster.font_id,
            u16::try_from(cluster.glyph_id).expect("glyph id must fit u16"),
            size,
            (0.0, 0.0),
        ) else {
            pen_x += cluster.advance;
            continue;
        };
        let Some(packed) = packer.insert(&bitmap) else {
            panic!("preview atlas overflow");
        };
        let (x, y) = match sheet {
            Some((columns, cell)) => {
                let cell_x = PADDING + (index as u32 % columns) as f32 * cell;
                let cell_y = PADDING + (index as u32 / columns) as f32 * cell;
                let cell_baseline = cell_y + cell * SHEET_BASELINE_RATIO;
                let centered_x = cell_x + (cell - cluster.advance) * 0.5;
                (
                    (centered_x + packed.bearing_x as f32).round(),
                    (cell_baseline - packed.bearing_y as f32).round(),
                )
            }
            None => (
                (pen_x + cluster.x_offset + packed.bearing_x as f32).round(),
                (baseline_y - packed.bearing_y as f32).round(),
            ),
        };
        placements.push(GlyphPlacement { packed, x, y });
        pen_x += cluster.advance;
    }

    let (width, height) = match sheet {
        Some((columns, cell)) => {
            let rows = shaped.clusters.len().div_ceil(columns as usize) as f32;
            (PADDING * 2.0 + columns as f32 * cell, PADDING * 2.0 + rows * cell)
        }
        None => (pen_x + PADDING, baseline_y + size * 1.2),
    };
    let (width, height) = (width.ceil() as u32, height.ceil() as u32);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;

    let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview atlas"),
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
    queue.write_texture(
        atlas_texture.as_image_copy(),
        &packer.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_SIZE),
            rows_per_image: Some(ATLAS_SIZE),
        },
        wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
    );
    let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let placeholder = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview image placeholder"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        placeholder.as_image_copy(),
        &[0, 0, 0, 0],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    let placeholder_view = placeholder.create_view(&wgpu::TextureViewDescriptor::default());

    let renderer = GlyphRenderer::new(&device, format);
    let gamma_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("preview gamma"),
        contents: bytemuck::cast_slice(&[GammaUniform { contrast, gamma }]),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("preview bind group"),
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
                resource: wgpu::BindingResource::TextureView(&placeholder_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(renderer.image_sampler()),
            },
        ],
    });

    let background = if dark { [0.090, 0.102, 0.122] } else { [1.0, 1.0, 1.0] };
    // Dark-surface foreground matches the color the app actually renders
    // (#989898), not the theme's nominal sRGB value.
    let foreground = if dark { [0.600, 0.600, 0.600] } else { [0.2157, 0.2549, 0.3176] };
    let linear = |value: f32| value.powf(2.2);
    let text_color = [
        linear(foreground[0]),
        linear(foreground[1]),
        linear(foreground[2]),
        1.0,
    ];
    let clear_color = wgpu::Color {
        r: linear(background[0]) as f64,
        g: linear(background[1]) as f64,
        b: linear(background[2]) as f64,
        a: 1.0,
    };

    let vertices = build_vertices(&placements, width, height, text_color);
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("preview vertices"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview msaa"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 4,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let msaa_view = msaa.create_view(&wgpu::TextureViewDescriptor::default());
    let readback_width = width.div_ceil(64) * 64;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("preview readback"),
        size: (readback_width * height * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("preview encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("preview pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &msaa_view,
                depth_slice: None,
                resolve_target: Some(&target_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(renderer.pipeline());
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..vertices.len() as u32, 0..1);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(readback_width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("preview receiver must stay alive");
    });
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    receiver.recv().expect("preview readback must arrive").expect("preview readback map failed");
    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * readback_width * 4) as usize;
        let end = start + (width * 4) as usize;
        for pixel in mapped[start..end].chunks_exact(4) {
            // Target is sRGB; readback bytes are already display-encoded.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    drop(mapped);
    readback.unmap();

    let image = image::RgbaImage::from_raw(width, height, rgba)
        .expect("preview image must match readback size");
    image.save(&output).expect("preview png must save");
    println!(
        "saved {output} ({width}x{height}) family={family} size={size} gamma={gamma} contrast={contrast} dark={dark}"
    );
}

fn build_vertices(
    placements: &[GlyphPlacement],
    width: u32,
    height: u32,
    color: [f32; 4],
) -> Vec<GlyphVertex> {
    let mut vertices = Vec::with_capacity(placements.len() * 6);
    let screen = |x: f32, y: f32| {
        [x / width as f32 * 2.0 - 1.0, 1.0 - y / height as f32 * 2.0]
    };
    for placement in placements {
        let packed = &placement.packed;
        let left = placement.x;
        let top = placement.y;
        let right = left + packed.width as f32;
        let bottom = top + packed.height as f32;
        let atlas = ATLAS_SIZE as f32;
        let uv_left = packed.atlas_x as f32 / atlas;
        let uv_top = packed.atlas_y as f32 / atlas;
        let uv_right = (packed.atlas_x + packed.width) as f32 / atlas;
        let uv_bottom = (packed.atlas_y + packed.height) as f32 / atlas;
        let top_left = GlyphVertex {
            position: screen(left, top),
            tex_coords: [uv_left, uv_top],
            color,
        };
        let top_right = GlyphVertex {
            position: screen(right, top),
            tex_coords: [uv_right, uv_top],
            color,
        };
        let bottom_left = GlyphVertex {
            position: screen(left, bottom),
            tex_coords: [uv_left, uv_bottom],
            color,
        };
        let bottom_right = GlyphVertex {
            position: screen(right, bottom),
            tex_coords: [uv_right, uv_bottom],
            color,
        };
        vertices.extend([top_left, top_right, bottom_left, top_right, bottom_right, bottom_left]);
    }
    vertices
}
