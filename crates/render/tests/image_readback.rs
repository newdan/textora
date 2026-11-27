use render::{GammaUniform, GlyphRenderer, GlyphVertex};
use wgpu::util::DeviceExt;

#[test]
fn rgba_image_midtones_and_transparency_survive_gpu_blending() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
        backends: wgpu::Backends::PRIMARY,
    });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    })) else {
        eprintln!("skipping image readback: no GPU adapter");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("image readback device"),
        ..Default::default()
    }))
    .expect("image readback device creation failed");

    for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
        for sample_count in [1, 4] {
            let frame = ReadbackPass {
                sample_count,
                store: wgpu::StoreOp::Discard,
                right_edge: 1.0,
                paint: IMAGE_PAINT,
            };
            let opaque = render_pixel(&device, &queue, format, [128, 64, 192, 255], frame);
            assert_color_near(opaque, [128, 64, 192, 255], 2);

            let translucent = render_pixel(&device, &queue, format, [64, 32, 96, 128], frame);
            let expected = if format.is_srgb() { [92, 44, 140, 255] } else { [64, 32, 96, 255] };
            assert_color_near(translucent, expected, 3);
        }
    }
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let edge = ReadbackPass {
        sample_count: 4,
        store: wgpu::StoreOp::Store,
        right_edge: 0.0,
        paint: IMAGE_PAINT,
    };
    let stored = render_pixel(&device, &queue, format, [255; 4], edge);
    let discarded = render_pixel(
        &device,
        &queue,
        format,
        [255; 4],
        ReadbackPass { store: wgpu::StoreOp::Discard, ..edge },
    );
    assert_color_near(stored, [128, 128, 128, 255], 1);
    assert_eq!(discarded, stored);

    for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
        let color = [0.75, 0.25, 0.125, 0.5];
        let sampled = ReadbackPass { paint: ([0.0; 2], color), ..edge };
        let flat = ReadbackPass { paint: (GlyphVertex::SOLID_TEX_COORDS, color), ..edge };
        let sampled_edge = render_pixel(&device, &queue, format, [255; 4], sampled);
        assert!(sampled_edge[0] > 0);
        assert_eq!(render_pixel(&device, &queue, format, [255; 4], flat), sampled_edge);
    }
}

const IMAGE_PAINT: ([f32; 2], [f32; 4]) = ([-1.5, 0.5], [1.0; 4]);

fn assert_color_near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "actual {actual} differs from expected {expected} by more than {tolerance}"
        );
    }
}

#[derive(Clone, Copy)]
struct ReadbackPass {
    sample_count: u32,
    store: wgpu::StoreOp,
    right_edge: f32,
    paint: ([f32; 2], [f32; 4]),
}

fn render_pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    premultiplied_pixel: [u8; 4],
    frame: ReadbackPass,
) -> [u8; 4] {
    let renderer = GlyphRenderer::new_with_sample_count(device, format, frame.sample_count);
    let glyph_texture = texture(
        device,
        "glyph pixel",
        wgpu::TextureFormat::R8Unorm,
        1,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let image_texture = texture(
        device,
        "image pixel",
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        glyph_texture.as_image_copy(),
        &[255],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(1), rows_per_image: Some(1) },
        pixel_extent(),
    );
    queue.write_texture(
        image_texture.as_image_copy(),
        &premultiplied_pixel,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        pixel_extent(),
    );
    let glyph_view = glyph_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let image_view = image_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let gamma = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("readback gamma"),
        contents: bytemuck::cast_slice(&[GammaUniform { contrast: 1.0, gamma: 1.45 }]),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("readback image binding"),
        layout: renderer.bind_group_layout(),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&glyph_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(renderer.sampler()),
            },
            wgpu::BindGroupEntry { binding: 2, resource: gamma.as_entire_binding() },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&image_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(renderer.image_sampler()),
            },
        ],
    });
    let (tex_coords, color) = frame.paint;
    let vertex = |x, y| GlyphVertex { position: [x, y], tex_coords, color };
    let vertices = [
        vertex(-1.0, 1.0),
        vertex(frame.right_edge, 1.0),
        vertex(-1.0, -1.0),
        vertex(frame.right_edge, 1.0),
        vertex(frame.right_edge, -1.0),
        vertex(-1.0, -1.0),
    ];
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("readback image quad"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let msaa = (frame.sample_count > 1).then(|| {
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if frame.store == wgpu::StoreOp::Discard {
            usage |= wgpu::TextureUsages::TRANSIENT;
        }
        texture(device, "readback MSAA", format, frame.sample_count, usage)
    });
    let resolved = texture(
        device,
        "readback resolved",
        format,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let msaa_view =
        msaa.map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
    let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback buffer"),
        size: 256,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("readback pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: msaa_view.as_ref().unwrap_or(&resolved_view),
                depth_slice: None,
                resolve_target: msaa_view.as_ref().map(|_| &resolved_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: msaa_view.as_ref().map_or(wgpu::StoreOp::Store, |_| frame.store),
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
        resolved.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        pixel_extent(),
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("readback receiver must remain alive");
    });
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    receiver.recv().expect("readback result must arrive").expect("readback map must succeed");
    let mapped = slice.get_mapped_range();
    let pixel = [mapped[0], mapped[1], mapped[2], mapped[3]];
    drop(mapped);
    readback.unmap();
    pixel
}

fn pixel_extent() -> wgpu::Extent3d {
    wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }
}

fn texture(
    device: &wgpu::Device,
    label: &'static str,
    format: wgpu::TextureFormat,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: pixel_extent(),
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
