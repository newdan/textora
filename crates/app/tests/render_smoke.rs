//! Render smoke tests: verify shaping + vertex generation pipeline.
//!
//! GPU-dependent rendering tests skip gracefully when no adapter is available.

use render::{GammaUniform, GlyphAtlas, GlyphRenderer, GlyphSlot};
use shaping::Shaper;
use wgpu::util::DeviceExt;

#[test]
fn shape_and_generate_vertices() {
    // Verify the full shaping → vertex pipeline works (no GPU needed)
    let mut shaper = match Shaper::new() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("skipping: no fonts available");
            return;
        }
    };

    let run = shaper.shape("Hello").expect("shaping failed");
    assert_eq!(run.clusters.len(), 5);
    assert!(run.width > 0.0);

    // Build fake glyph slots (normally from atlas upload)
    let mut atlas = GlyphAtlas::new(512, 512, 100, 4);
    let mut glyph_positions = Vec::new();
    let mut x = 10.0f32;

    for cluster in &run.clusters {
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            cluster.font_id.hash(&mut h);
            render::GlyphKey {
                glyph_id: cluster.glyph_id,
                font_id: h.finish() as usize,
                font_size: 14 * 64,
                subpixel_phase: 0,
            }
        };
        let slot = atlas.insert(key, 10, 12, 0.0, 10.0).expect("atlas insert failed");
        glyph_positions.push((slot, x, 50.0));
        x += cluster.advance;
    }

    let vertices = GlyphRenderer::generate_vertices(
        &glyph_positions,
        512,
        512,
        800.0,
        600.0,
        [1.0, 1.0, 1.0, 1.0],
    );

    // 5 glyphs * 6 vertices = 30
    assert_eq!(vertices.len(), 30);
}

#[test]
fn render_pipeline_creation() {
    // Verify the wgpu render pipeline can be created
    let instance = wgpu::Instance::default();
    // Try hardware first, then fallback
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .or_else(|_| {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: true,
            ..Default::default()
        }))
    });
    let Ok(adapter) = adapter else {
        eprintln!("skipping: no GPU adapter");
        return;
    };

    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("test device"),
        ..Default::default()
    }))
    .expect("device creation failed");

    let renderer = GlyphRenderer::new(&device, wgpu::TextureFormat::R8Unorm);
    // Verify pipeline was created
    let _ = renderer.pipeline();
}

// --- Golden image test ---

/// Simple SSIM over a single channel (luminance).
/// Returns value in [0, 1] where 1 = identical.
fn ssim_luma(img_a: &[u8], img_b: &[u8], width: usize, height: usize) -> f64 {
    assert_eq!(img_a.len(), img_b.len());
    assert_eq!(img_a.len(), width * height);

    let n = (width * height) as f64;
    if n == 0.0 {
        return 1.0;
    }

    let mut sum_a = 0.0f64;
    let mut sum_b = 0.0f64;
    for i in 0..img_a.len() {
        sum_a += img_a[i] as f64;
        sum_b += img_b[i] as f64;
    }
    let mu_a = sum_a / n;
    let mu_b = sum_b / n;

    let mut sigma_a2 = 0.0f64;
    let mut sigma_b2 = 0.0f64;
    let mut sigma_ab = 0.0f64;
    for i in 0..img_a.len() {
        let da = img_a[i] as f64 - mu_a;
        let db = img_b[i] as f64 - mu_b;
        sigma_a2 += da * da;
        sigma_b2 += db * db;
        sigma_ab += da * db;
    }
    sigma_a2 /= n;
    sigma_b2 /= n;
    sigma_ab /= n;

    let c1: f64 = (0.01 * 255.0_f64).powi(2);
    let c2: f64 = (0.03 * 255.0_f64).powi(2);

    let num = (2.0 * mu_a * mu_b + c1) * (2.0 * sigma_ab + c2);
    let den = (mu_a.powi(2) + mu_b.powi(2) + c1) * (sigma_a2 + sigma_b2 + c2);
    num / den
}

/// Extract luminance (Y) channel from RGBA image.
fn rgba_to_luma(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut luma = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            let r = rgba[idx] as f64;
            let g = rgba[idx + 1] as f64;
            let b = rgba[idx + 2] as f64;
            let y_val = 0.299 * r + 0.587 * g + 0.114 * b;
            luma.push(y_val.clamp(0.0, 255.0) as u8);
        }
    }
    luma
}

/// Write RGBA pixels as PPM (P6) for human inspection.
fn write_ppm(path: &std::path::Path, rgba: &[u8], width: u32, height: u32) {
    use std::io::Write;
    let mut out = Vec::with_capacity(64 + (width * height * 3) as usize);
    write!(out, "P6\n{width} {height}\n255\n").unwrap();
    for chunk in rgba.chunks(4) {
        out.push(chunk[0]);
        out.push(chunk[1]);
        out.push(chunk[2]);
    }
    std::fs::write(path, &out).expect("write ppm failed");
}

const GOLDEN_WIDTH: u32 = 800;
const GOLDEN_HEIGHT: u32 = 600;

#[test]
fn render_hello_to_png() {
    // --- GPU setup ---
    let instance = wgpu::Instance::default();
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    })) {
        Ok(a) => a,
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            return;
        }
    };

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("golden test device"),
        ..Default::default()
    }))
    .expect("device creation failed");

    let format = wgpu::TextureFormat::R8Unorm;

    // --- Create atlas texture (R8Unorm alpha) ---
    let atlas_size = 512u32;
    let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas"),
        size: wgpu::Extent3d { width: atlas_size, height: atlas_size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // --- Create render target (must match pipeline MSAA 4x) ---
    let render_target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render target"),
        size: wgpu::Extent3d {
            width: GOLDEN_WIDTH,
            height: GOLDEN_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 4,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let render_view = render_target.create_view(&wgpu::TextureViewDescriptor::default());

    // Resolve target for readback (MSAA → single-sample)
    let resolve_target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resolve target"),
        size: wgpu::Extent3d {
            width: GOLDEN_WIDTH,
            height: GOLDEN_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let resolve_view = resolve_target.create_view(&wgpu::TextureViewDescriptor::default());

    // --- Renderer setup ---
    let renderer = GlyphRenderer::new(&device, format);

    // Gamma uniform buffer (required by bind group layout even when unused)
    let gamma_uniform = GammaUniform { contrast: 1.0, gamma: 1.45 };
    let gamma_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gamma uniform"),
        contents: bytemuck::cast_slice(&[gamma_uniform]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bind group"),
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

    // --- Shape text ---
    let font_size = 14.0f32;
    let mut shaper = Shaper::new().expect("shaper creation failed");
    shaper = shaper.with_font_size(font_size);
    let shaped = shaper.shape("Hello, edit+").expect("shaping failed");
    assert!(!shaped.clusters.is_empty(), "should produce glyphs");

    // --- Rasterize and upload glyphs to atlas ---
    let mut atlas = GlyphAtlas::new(atlas_size, atlas_size, 4096, 4);
    let mut glyph_positions: Vec<(GlyphSlot, f32, f32)> = Vec::new();
    let mut x = 8.0f32;
    let y_base = 50.0f32;

    for cluster in &shaped.clusters {
        let font_id_usize = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            cluster.font_id.hash(&mut h);
            h.finish() as usize
        };
        let key = render::GlyphKey {
            glyph_id: cluster.glyph_id,
            font_id: font_id_usize,
            font_size: (font_size * 64.0) as u32,
            subpixel_phase: 0,
        };

        let slot = if let Some(cached) = atlas.get(&key) {
            *cached
        } else if let Some(bitmap) =
            shaper.rasterize_glyph(cluster.font_id, cluster.glyph_id as u16, font_size, (0.0, 0.0))
        {
            if bitmap.width > 0 && bitmap.height > 0 {
                if let Some(slot) = atlas.insert(
                    key,
                    bitmap.width,
                    bitmap.height,
                    bitmap.left as f32,
                    bitmap.top as f32,
                ) {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &atlas_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x: slot.x, y: slot.y, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &bitmap.data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(bitmap.width),
                            rows_per_image: Some(bitmap.height),
                        },
                        wgpu::Extent3d {
                            width: bitmap.width,
                            height: bitmap.height,
                            depth_or_array_layers: 1,
                        },
                    );
                    slot
                } else {
                    continue;
                }
            } else {
                continue;
            }
        } else {
            continue;
        };

        glyph_positions.push((slot, x, y_base));
        x += cluster.advance;
    }

    assert!(!glyph_positions.is_empty(), "should have rendered glyphs");

    // --- Generate vertices ---
    let vertices = GlyphRenderer::generate_vertices(
        &glyph_positions,
        atlas_size,
        atlas_size,
        GOLDEN_WIDTH as f32,
        GOLDEN_HEIGHT as f32,
        [1.0, 1.0, 1.0, 1.0],
    );
    assert!(!vertices.is_empty());

    // --- Upload vertex buffer ---
    let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vertex buffer"),
        size: (vertices.len() * std::mem::size_of::<render::GlyphVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

    // --- Render pass ---
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("encoder") });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("golden render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &render_view,
                resolve_target: Some(&resolve_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.12, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
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

    // --- Copy render target to buffer ---
    let bytes_per_row = (GOLDEN_WIDTH * 4 + 255) & !255;
    let buffer_size = (bytes_per_row * GOLDEN_HEIGHT) as u64;
    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &resolve_target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(GOLDEN_HEIGHT),
            },
        },
        wgpu::Extent3d { width: GOLDEN_WIDTH, height: GOLDEN_HEIGHT, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(encoder.finish()));

    // --- Read back pixels ---
    let buffer_slice = readback_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    rx.recv().unwrap().expect("buffer map failed");

    let padded_data = buffer_slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((GOLDEN_WIDTH * GOLDEN_HEIGHT * 4) as usize);
    for row in 0..GOLDEN_HEIGHT {
        let start = (row * bytes_per_row) as usize;
        let end = start + (GOLDEN_WIDTH * 4) as usize;
        pixels.extend_from_slice(&padded_data[start..end]);
    }
    drop(padded_data);
    readback_buffer.unmap();

    // --- Verify non-blank ---
    let non_black = pixels.chunks(4).filter(|px| px[0] > 20 || px[1] > 20 || px[2] > 20).count();
    assert!(non_black > 10, "rendered image appears blank ({non_black} non-background pixels)");

    // --- Golden image comparison ---
    let golden_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/golden");
    let golden_path = golden_dir.join("hello_textora.ppm");

    if std::env::var("RENDER_GOLDEN_UPDATE").is_ok() || !golden_path.exists() {
        write_ppm(&golden_path, &pixels, GOLDEN_WIDTH, GOLDEN_HEIGHT);
        eprintln!("Golden image saved to {}", golden_path.display());
        if std::env::var("RENDER_GOLDEN_UPDATE").is_ok() {
            return;
        }
    }

    // Load golden and compare
    let golden_data = std::fs::read(&golden_path).expect("failed to read golden ppm");
    // Parse PPM P6: skip header, read RGB data

    // Find end of "255\n" line
    let mut data_start = 0usize;
    let mut newline_count = 0usize;
    for (i, &b) in golden_data.iter().enumerate() {
        if b == b'\n' {
            newline_count += 1;
            if newline_count == 3 {
                data_start = i + 1;
                break;
            }
        }
    }
    assert!(data_start > 0, "invalid PPM: couldn't find data start");

    let golden_rgb = &golden_data[data_start..];
    // Convert golden RGB back to RGBA for comparison
    let mut golden_rgba = Vec::with_capacity((GOLDEN_WIDTH * GOLDEN_HEIGHT * 4) as usize);
    for chunk in golden_rgb.chunks(3) {
        if chunk.len() == 3 {
            golden_rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
        }
    }
    assert_eq!(
        golden_rgba.len(),
        pixels.len(),
        "golden size mismatch: {} vs {}",
        golden_rgba.len(),
        pixels.len()
    );

    let luma_actual = rgba_to_luma(&pixels, GOLDEN_WIDTH as usize, GOLDEN_HEIGHT as usize);
    let luma_golden = rgba_to_luma(&golden_rgba, GOLDEN_WIDTH as usize, GOLDEN_HEIGHT as usize);
    let ssim = ssim_luma(&luma_actual, &luma_golden, GOLDEN_WIDTH as usize, GOLDEN_HEIGHT as usize);

    eprintln!("SSIM = {ssim:.4}");
    assert!(ssim >= 0.95, "SSIM {ssim:.4} < 0.95: rendered image differs too much from golden");
}

/// Diagnostic: show exact truncation behaviour for CJK filename with real shaper.
#[test]
fn diagnostic_truncate_cjk_precise() {
    use textora_app::dev_support::MeasureFromShaper;
    use ui::core::measure::TextMeasure;
    use ui::core::text_util::truncate_title_precise;

    let mut shaper = match Shaper::new() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("skipping: no fonts available");
            return;
        }
    };

    let title = "《我家娘子，不对劲》（校对版）.txt";
    let font_size = 14.0;

    // Per-character widths
    println!("\n=== Per-character widths (font_size={font_size}) ===");
    let mut total = 0.0;
    for (i, ch) in title.chars().enumerate() {
        let s: String = [ch].iter().collect();
        let w = {
            let mut m = MeasureFromShaper(&mut shaper);
            m.measure(&s, font_size)
        };
        total += w;
        println!("  [{i:2}] '{ch}'  width={w:.2}px");
    }
    let full_w = {
        let mut m = MeasureFromShaper(&mut shaper);
        m.measure(title, font_size)
    };
    println!("  TOTAL: estimated={total:.2}px  measured={full_w:.2}px");
    println!();

    // Test truncation at various max widths
    for &max_w in &[400.0, 350.0, 300.0, 280.0, 260.0, 240.0, 220.0, 200.0, 180.0, 160.0] {
        let mut m = MeasureFromShaper(&mut shaper);
        let result = truncate_title_precise(title, max_w, font_size, &mut m);
        let result_w = {
            let mut m2 = MeasureFromShaper(&mut shaper);
            m2.measure(&result, font_size)
        };
        println!(
            "max={max_w:.0}px  truncated=\"{result}\"  width={result_w:.1}px  full={full_w:.1}px"
        );
    }
}

/// Quick diagnostic for sidebar label truncation
#[test]
fn diagnostic_sidebar_label() {
    use textora_app::dev_support::MeasureFromShaper;
    use ui::core::measure::TextMeasure;
    use ui::core::text_util::truncate_title_precise;

    let mut shaper = match Shaper::new() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("skipping: no fonts");
            return;
        }
    };

    let font_size = 15.0;

    // test punctuation widths
    let puncts = [
        ("CJK comma", "，"),
        ("CJK period", "。"),
        ("CJK quote L", "《"),
        ("CJK quote R", "》"),
        ("CJK paren L", "（"),
        ("CJK paren R", "）"),
        ("CJK colon", "："),
        ("ASCII period", "."),
        ("ASCII comma", ","),
        ("ellipsis", "\u{2026}"),
        ("CJK ideograph", "我"),
        ("ASCII letter", "a"),
    ];
    println!("\n=== Punctuation widths at font_size={font_size} ===");
    for (name, ch) in puncts {
        let mut m = MeasureFromShaper(&mut shaper);
        let w = m.measure(ch, font_size);
        let expected = if ch.len() == 1 && ch.as_bytes()[0].is_ascii() {
            format!("~{:.0}px", font_size * 0.6)
        } else {
            format!("1em={font_size}px")
        };
        println!("  {name:>16} '{ch}' = {w:.1}px  (expected {expected})");
    }

    let max_w = 188.0;

    let titles =
        ["《我家娘子，不对劲》（校对版）.txt", "07_把抗战精神转化为提升三服务水平强大动力.md"];

    for title in titles {
        let mut m = MeasureFromShaper(&mut shaper);
        let full_w = m.measure(title, font_size);
        // per-char
        println!("\n=== {title} ===");
        let mut sum = 0.0;
        for (i, ch) in title.chars().enumerate() {
            let s: String = [ch].iter().collect();
            let w = m.measure(&s, font_size);
            sum += w;
            print!("[{i}]'{ch}'={w:.1} ");
        }
        println!("\nsum={sum:.1}  full={full_w:.1}  max={max_w:.0}");

        let result = truncate_title_precise(title, max_w, font_size, &mut m);
        let result_w = m.measure(&result, font_size);
        let ellipsis_w = m.measure("\u{2026}", font_size);
        let parts: Vec<&str> = result.split('\u{2026}').collect();
        let pre_w = m.measure(parts[0], font_size);
        let suf_w = if parts.len() > 1 { m.measure(parts[1], font_size) } else { 0.0 };
        let total = pre_w + ellipsis_w + suf_w;
        println!(
            "result=\"{result}\"  w={result_w:.1}  pre={pre_w:.1}  …={ellipsis_w:.1}  suf={suf_w:.1}  total={total:.1}  slack={:.1}",
            max_w - total
        );
    }
}
