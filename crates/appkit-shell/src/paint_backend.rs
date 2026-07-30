//! DrawList → GlyphVertex 转换。
//! Phase 2：FillRect / StrokeRect / PushClip / PopClip 路径。
//! Phase 3：Text 路径——把 DrawCmd::Text 翻译为 atlas + GlyphVertex。

use render::GlyphVertex;
use ui::core::text_layout::ITALIC_SHEAR;
use ui::core::{DrawCmd, DrawList, Rect, Screen};

use crate::render_cache::{CachedLine, GlyphInstance};
use crate::render_state::{GpuState, TextState};

const ROUNDED_FILL_FEATHER_PX: f32 = 0.5;
const CLIPPED_TRIANGLE_AREA_EPSILON: f32 = 1.0e-8;

/// 将 DrawList 中的命令转换为 GPU 顶点。
/// 消耗整个 DrawList，返回 Vec<GlyphVertex>。
/// 返回的顶点已映射到 NDC 空间。
///
/// `text` 和 `gpu` 仅 Text 命令需要；传 None 时 Text 命令静默跳过。
pub fn drain(
    list: DrawList,
    screen: Screen,
    mut text: Option<&mut TextState>,
    gpu: Option<&GpuState>,
) -> Vec<GlyphVertex> {
    let _drain_t0 = std::time::Instant::now();
    let mut vertices = Vec::new();
    let mut clip_stack: Vec<Rect> = Vec::new();
    let sw = screen.w;
    let sh = screen.h;
    let mut text_layout_count: usize = 0;
    let mut glyph_resolve_count: usize = 0;
    let mut glyph_cache_hit: usize = 0;
    let mut glyph_cache_miss: usize = 0;

    for cmd in list.cmds {
        match cmd {
            DrawCmd::FillRect { rect, color, radius } => {
                if radius > 0.0 {
                    if has_non_positive_dimension(rect) {
                        continue;
                    }
                    let tessellation_bounds = Rect::new(
                        rect.x - ROUNDED_FILL_FEATHER_PX,
                        rect.y - ROUNDED_FILL_FEATHER_PX,
                        rect.w + ROUNDED_FILL_FEATHER_PX * 2.0,
                        rect.h + ROUNDED_FILL_FEATHER_PX * 2.0,
                    );
                    if !intersects_final_clip(tessellation_bounds, &clip_stack) {
                        continue;
                    }
                    append_rounded_with_clip(
                        &mut vertices,
                        rect,
                        color,
                        radius,
                        &screen,
                        &clip_stack,
                        push_fill,
                    );
                } else {
                    let r = apply_clip(&clip_stack, rect);
                    if r.w <= 0.0 || r.h <= 0.0 {
                        continue;
                    }
                    let ndc = screen.rect_to_ndc(r);
                    push_quad(&mut vertices, ndc, color);
                }
            }
            DrawCmd::StrokeRect { rect, color, radius, line_width } => {
                let radius = radius.max(0.0);
                let line_width = line_width.max(0.5);
                if radius > 0.0 {
                    if has_non_positive_dimension(rect)
                        || !intersects_final_clip(
                            rounded_stroke_bounds(rect, line_width),
                            &clip_stack,
                        )
                    {
                        continue;
                    }
                    append_rounded_with_clip(
                        &mut vertices,
                        rect,
                        color,
                        radius,
                        &screen,
                        &clip_stack,
                        |vertices, rect, color, radius, screen| {
                            push_stroke(vertices, rect, color, radius, line_width, screen);
                        },
                    );
                } else {
                    let r = apply_clip(&clip_stack, rect);
                    if r.w <= 0.0 || r.h <= 0.0 {
                        continue;
                    }
                    push_stroke(&mut vertices, r, color, radius, line_width, &screen);
                }
            }
            DrawCmd::TextLayout { layout, x, y_baseline, color } => {
                let (Some(text_state), Some(gpu)) = (text.as_deref_mut(), gpu) else {
                    continue;
                };
                text_layout_count += 1;
                let cache_key = layout.id;

                // 尝试从预览 cache 获取
                let cached = text_state.preview_cache.get(cache_key);

                if let Some(cached_line) = cached {
                    // Cache hit — 从 GlyphInstance 直接发射
                    let verts = emit_from_instances(
                        &cached_line.instances,
                        x,
                        y_baseline,
                        sw,
                        sh,
                        color,
                        &clip_stack,
                        layout.italic,
                    );
                    vertices.extend(verts);
                } else {
                    // Cache miss — rasterize from shaped data
                    let mut instances = Vec::new();
                    let mut x_cursor = x;

                    for cluster in &layout.shaped.clusters {
                        let advance = cluster.advance.max(1.0);
                        if layout
                            .text
                            .as_bytes()
                            .get(cluster.byte_range.clone())
                            .is_some_and(|bytes| bytes.iter().all(|&b| b == b' ' || b == b'\t'))
                        {
                            x_cursor += advance;
                            continue;
                        }

                        glyph_resolve_count += 1;
                        let (_int_x, phase) = render::split_subpixel(x_cursor);
                        // Check atlas hit before resolve_glyph
                        let font_id_usize = {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::hash::DefaultHasher::new();
                            cluster.font_id.hash(&mut h);
                            h.finish() as usize
                        };
                        let key = render::GlyphKey {
                            glyph_id: cluster.glyph_id,
                            font_id: font_id_usize,
                            font_size: (layout.font_size * 64.0) as u32,
                            subpixel_phase: phase,
                        };
                        if text_state.atlas.get(&key).is_some() {
                            glyph_cache_hit += 1;
                        } else {
                            glyph_cache_miss += 1;
                        }

                        if let Some(slot) = crate::text_rasterize::resolve_glyph(
                            cluster.font_id,
                            cluster.glyph_id as u16,
                            layout.font_size,
                            phase,
                            &mut text_state.shaper,
                            &mut text_state.atlas,
                            &text_state.atlas_texture,
                            &gpu.ctx.queue,
                        ) {
                            text_state.track_glyph_resolve();
                            let aw = crate::render_state::ATLAS_SIZE as f32;
                            let ah = crate::render_state::ATLAS_SIZE as f32;
                            instances.push(GlyphInstance {
                                x: x_cursor - x,
                                y: 0.0,
                                bearing_x: slot.bearing_x,
                                bearing_y: slot.bearing_y,
                                width: slot.width as f32,
                                height: slot.height as f32,
                                uv: [
                                    slot.x as f32 / aw,
                                    slot.y as f32 / ah,
                                    (slot.x + slot.width) as f32 / aw,
                                    (slot.y + slot.height) as f32 / ah,
                                ],
                                atlas_page: slot.page,
                                highlight_kind: 0,
                            });
                        }
                        x_cursor += advance;
                    }

                    // 构建并缓存 CachedLine
                    let cluster_data: Vec<_> = layout
                        .shaped
                        .clusters
                        .iter()
                        .map(|c| (c.byte_range.start, c.byte_range.end, c.advance.max(1.0)))
                        .collect();

                    // 发射顶点（先发射，再 move instances 到缓存）
                    let verts = emit_from_instances(
                        &instances,
                        x,
                        y_baseline,
                        sw,
                        sh,
                        color,
                        &clip_stack,
                        layout.italic,
                    );
                    vertices.extend(verts);

                    let cached_line = CachedLine {
                        instances,
                        line_number_glyphs: vec![],
                        atlas_generation: text_state.atlas_generation,
                        visual_line_count: 1,
                        content_hash: cache_key, // layout.id, used as cache key (not a content hash)
                        visual_lines: vec![(0, layout.shaped.clusters.len(), x_cursor - x)],
                        visual_line_instance_starts: vec![0],
                        cluster_data,
                        subset_start: 0,
                    };
                    text_state.preview_cache.insert(cache_key, cached_line);
                }
            }
            DrawCmd::TaperedMesh { mesh, translation, color } => {
                let bounds = translated_rect(mesh.bounds, translation);
                let clip = clip_stack.last().copied();
                match mesh_clip_relation(bounds, clip) {
                    MeshClipRelation::Outside => {}
                    MeshClipRelation::Inside => {
                        vertices.extend(tapered_mesh_vertices(&mesh, translation, color, &screen));
                    }
                    MeshClipRelation::Intersecting => {
                        let tessellated = tapered_mesh_vertices(&mesh, translation, color, &screen);
                        let clip = clip.expect("an intersecting mesh requires an active clip");
                        append_clipped_triangles(&mut vertices, &tessellated, clip, &screen);
                    }
                }
            }
            DrawCmd::FillTriangle { p0, p1, p2, color } => {
                // Apply clip to each point (simple bounding-box clip)
                let clip = clip_stack.last().copied().unwrap_or(Rect::new(0.0, 0.0, sw, sh));
                let pts = [p0, p1, p2];
                // Skip only if all points are outside the SAME clip edge
                if pts.iter().all(|p| p[0] < clip.x)      // all left
                    || pts.iter().all(|p| p[0] > clip.right())  // all right
                    || pts.iter().all(|p| p[1] < clip.y)        // all above
                    || pts.iter().all(|p| p[1] > clip.bottom())
                // all below
                {
                    continue;
                }
                let ndc0 = screen.px_to_ndc(p0[0], p0[1]);
                let ndc1 = screen.px_to_ndc(p1[0], p1[1]);
                let ndc2 = screen.px_to_ndc(p2[0], p2[1]);
                let uv = [0.0, 0.0];
                vertices.push(GlyphVertex { position: ndc0, tex_coords: uv, color });
                vertices.push(GlyphVertex { position: ndc1, tex_coords: uv, color });
                vertices.push(GlyphVertex { position: ndc2, tex_coords: uv, color });
            }
            DrawCmd::PushClip(r) => {
                let clipped = apply_clip(&clip_stack, r);
                clip_stack.push(clipped);
            }
            DrawCmd::PopClip => {
                clip_stack.pop();
            }
        }
    }

    let _drain_elapsed = _drain_t0.elapsed();
    eprintln!(
        "[drain] time={:.1}ms text_layout_commands={} glyph_resolve={} cache_hit={} cache_miss={} total_vertices={}",
        _drain_elapsed.as_secs_f64() * 1000.0,
        text_layout_count,
        glyph_resolve_count,
        glyph_cache_hit,
        glyph_cache_miss,
        vertices.len(),
    );

    vertices
}

/// 将一组语义绘制命令追加到既有顶点缓冲，保持产品 chrome/editor 的提交顺序。
pub fn drain_into(
    list: DrawList,
    screen: Screen,
    text: Option<&mut TextState>,
    gpu: Option<&GpuState>,
    vertices: &mut Vec<GlyphVertex>,
) {
    vertices.extend(drain(list, screen, text, gpu));
}

/// 从 GlyphInstance 列表直接发射 NDC 顶点（用于预览 TextLayout 路径）。
fn emit_from_instances(
    instances: &[GlyphInstance],
    origin_x: f32,
    baseline_y: f32,
    screen_w: f32,
    screen_h: f32,
    color: [f32; 4],
    clip_stack: &[Rect],
    italic: bool,
) -> Vec<render::GlyphVertex> {
    let shear = if italic { ITALIC_SHEAR } else { 0.0 };
    let mut verts = Vec::with_capacity(instances.len() * 6);
    for inst in instances {
        let px = (origin_x + inst.x + inst.bearing_x).round();
        let py = (baseline_y - inst.bearing_y).round();
        // Expand clip rect horizontally to account for italic shear
        let max_shear = (inst.height * shear).abs().ceil();
        let c_rect = apply_clip(
            clip_stack,
            ui::core::Rect::new(px - max_shear, py, inst.width + max_shear * 2.0, inst.height),
        );
        if c_rect.w <= 0.0 || c_rect.h <= 0.0 {
            continue;
        }
        let left = c_rect.x / screen_w * 2.0 - 1.0;
        let top = 1.0 - c_rect.y / screen_h * 2.0;
        let right = (c_rect.x + c_rect.w) / screen_w * 2.0 - 1.0;
        let bottom = 1.0 - (c_rect.y + c_rect.h) / screen_h * 2.0;
        // Italic shear in NDC: x += (baseline_y - y_px) * shear * 2 / screen_w
        let top_shear = (baseline_y - c_rect.y) * shear / screen_w * 2.0;
        let bottom_shear = (baseline_y - (c_rect.y + c_rect.h)) * shear / screen_w * 2.0;
        // Adjust UVs proportionally when glyph is partially clipped
        let u_range = inst.uv[2] - inst.uv[0];
        let v_range = inst.uv[3] - inst.uv[1];
        let ul = inst.uv[0] + u_range * ((c_rect.x - px) / inst.width).max(0.0);
        let ur = inst.uv[0] + u_range * ((c_rect.x + c_rect.w - px) / inst.width).min(1.0);
        let ut = inst.uv[1] + v_range * ((c_rect.y - py) / inst.height).max(0.0);
        let ub = inst.uv[1] + v_range * ((c_rect.y + c_rect.h - py) / inst.height).min(1.0);
        let sx_tl = left + top_shear;
        let sx_tr = right + top_shear;
        let sx_bl = left + bottom_shear;
        let sx_br = right + bottom_shear;
        verts.push(render::GlyphVertex { position: [sx_tl, top], tex_coords: [ul, ut], color });
        verts.push(render::GlyphVertex { position: [sx_tr, top], tex_coords: [ur, ut], color });
        verts.push(render::GlyphVertex { position: [sx_bl, bottom], tex_coords: [ul, ub], color });
        verts.push(render::GlyphVertex { position: [sx_tr, top], tex_coords: [ur, ut], color });
        verts.push(render::GlyphVertex { position: [sx_br, bottom], tex_coords: [ur, ub], color });
        verts.push(render::GlyphVertex { position: [sx_bl, bottom], tex_coords: [ul, ub], color });
    }
    verts
}

/// 将当前裁剪栈应用到 rect，返回交集。
fn apply_clip(stack: &[Rect], rect: Rect) -> Rect {
    let mut r = rect;
    for clip in stack {
        let x = r.x.max(clip.x);
        let y = r.y.max(clip.y);
        let r2 = r.x + r.w;
        let c2 = clip.x + clip.w;
        let b2 = r.y + r.h;
        let cb2 = clip.y + clip.h;
        let w = (r2.min(c2) - x).max(0.0);
        let h = (b2.min(cb2) - y).max(0.0);
        r = Rect::new(x, y, w, h);
    }
    r
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeshClipRelation {
    Inside,
    Outside,
    Intersecting,
}

fn translated_rect(rect: Rect, translation: [f32; 2]) -> Rect {
    Rect::new(rect.x + translation[0], rect.y + translation[1], rect.w, rect.h)
}

fn mesh_clip_relation(bounds: Rect, clip: Option<Rect>) -> MeshClipRelation {
    let Some(clip) = clip else {
        return MeshClipRelation::Inside;
    };
    if has_non_positive_dimension(clip) {
        return MeshClipRelation::Outside;
    }

    if bounds.right() <= clip.x
        || bounds.x >= clip.right()
        || bounds.bottom() <= clip.y
        || bounds.y >= clip.bottom()
    {
        return MeshClipRelation::Outside;
    }

    if bounds.x >= clip.x
        && bounds.right() <= clip.right()
        && bounds.y >= clip.y
        && bounds.bottom() <= clip.bottom()
    {
        MeshClipRelation::Inside
    } else {
        MeshClipRelation::Intersecting
    }
}

fn tapered_mesh_vertices(
    mesh: &ui::tapered_path::TaperedMesh,
    translation: [f32; 2],
    color: [f32; 4],
    screen: &Screen,
) -> Vec<GlyphVertex> {
    mesh.vertices
        .iter()
        .map(|mesh_vertex| GlyphVertex {
            position: screen.px_to_ndc(
                mesh_vertex.position[0] + translation[0],
                mesh_vertex.position[1] + translation[1],
            ),
            tex_coords: [0.0, 0.0],
            color: [color[0], color[1], color[2], color[3] * mesh_vertex.alpha_multiplier],
        })
        .collect()
}

fn has_non_positive_dimension(rect: Rect) -> bool {
    rect.w <= 0.0 || rect.h <= 0.0
}

fn intersects_final_clip(bounds: Rect, clips: &[Rect]) -> bool {
    let Some(clip) = clips.last() else {
        return true;
    };
    if has_non_positive_dimension(*clip) {
        return false;
    }

    bounds.x < clip.right()
        && bounds.right() > clip.x
        && bounds.y < clip.bottom()
        && bounds.bottom() > clip.y
}

fn rounded_stroke_bounds(rect: Rect, line_width: f32) -> Rect {
    let horizontal_extension = (line_width - rect.w).max(0.0);
    let vertical_extension = (line_width - rect.h).max(0.0);
    Rect::new(
        rect.x - horizontal_extension,
        rect.y - vertical_extension,
        rect.w + horizontal_extension * 2.0,
        rect.h + vertical_extension * 2.0,
    )
}

fn append_rounded_with_clip(
    vertices: &mut Vec<GlyphVertex>,
    rect: Rect,
    color: [f32; 4],
    radius: f32,
    screen: &Screen,
    clips: &[Rect],
    tessellate: impl FnOnce(&mut Vec<GlyphVertex>, Rect, [f32; 4], f32, &Screen),
) {
    let Some(clip) = clips.last().copied() else {
        tessellate(vertices, rect, color, radius, screen);
        return;
    };

    let mut tessellated = Vec::new();
    tessellate(&mut tessellated, rect, color, radius, screen);
    append_clipped_triangles(vertices, &tessellated, clip, screen);
}

fn append_clipped_triangles(
    vertices: &mut Vec<GlyphVertex>,
    tessellated: &[GlyphVertex],
    clip: Rect,
    screen: &Screen,
) {
    let clip_ndc = screen.rect_to_ndc(clip);
    let mut clip_buffers = None;

    for triangle in tessellated.chunks_exact(3) {
        match triangle_clip_relation(triangle, clip_ndc) {
            TriangleClipRelation::Inside => {
                append_triangle_above_area_epsilon(vertices, triangle[0], triangle[1], triangle[2])
            }
            TriangleClipRelation::Outside => {}
            TriangleClipRelation::Intersecting => {
                let (polygon, clipped) = clip_buffers
                    .get_or_insert_with(|| (Vec::with_capacity(7), Vec::with_capacity(7)));
                polygon.clear();
                polygon.extend_from_slice(triangle);
                for edge in [ClipEdge::Left, ClipEdge::Right, ClipEdge::Top, ClipEdge::Bottom] {
                    clipped.clear();
                    clip_polygon_to_edge(polygon, edge, clip_ndc, clipped);
                    std::mem::swap(polygon, clipped);
                    if polygon.is_empty() {
                        break;
                    }
                }
                for index in 1..polygon.len().saturating_sub(1) {
                    append_triangle_above_area_epsilon(
                        vertices,
                        polygon[0],
                        polygon[index],
                        polygon[index + 1],
                    );
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TriangleClipRelation {
    Inside,
    Outside,
    Intersecting,
}

fn triangle_clip_relation(triangle: &[GlyphVertex], clip: [f32; 4]) -> TriangleClipRelation {
    debug_assert_eq!(triangle.len(), 3);
    let Some(first) = triangle.first() else {
        return TriangleClipRelation::Outside;
    };
    let mut minimum = first.position;
    let mut maximum = first.position;
    for vertex in &triangle[1..] {
        minimum[0] = minimum[0].min(vertex.position[0]);
        minimum[1] = minimum[1].min(vertex.position[1]);
        maximum[0] = maximum[0].max(vertex.position[0]);
        maximum[1] = maximum[1].max(vertex.position[1]);
    }

    if maximum[0] <= clip[0]
        || minimum[0] >= clip[1]
        || maximum[1] <= clip[3]
        || minimum[1] >= clip[2]
    {
        return TriangleClipRelation::Outside;
    }

    if minimum[0] >= clip[0]
        && maximum[0] <= clip[1]
        && minimum[1] >= clip[3]
        && maximum[1] <= clip[2]
    {
        TriangleClipRelation::Inside
    } else {
        TriangleClipRelation::Intersecting
    }
}

fn append_triangle_above_area_epsilon(
    vertices: &mut Vec<GlyphVertex>,
    first: GlyphVertex,
    second: GlyphVertex,
    third: GlyphVertex,
) {
    let signed_area = ((second.position[0] - first.position[0])
        * (third.position[1] - first.position[1])
        - (third.position[0] - first.position[0]) * (second.position[1] - first.position[1]))
        * 0.5;
    if signed_area.abs() <= CLIPPED_TRIANGLE_AREA_EPSILON {
        return;
    }
    vertices.extend_from_slice(&[first, second, third]);
}

#[derive(Clone, Copy)]
enum ClipEdge {
    Left,
    Right,
    Top,
    Bottom,
}

fn clip_polygon_to_edge(
    polygon: &[GlyphVertex],
    edge: ClipEdge,
    rect: [f32; 4],
    clipped: &mut Vec<GlyphVertex>,
) {
    let Some(mut previous) = polygon.last().copied() else {
        return;
    };
    let mut previous_inside = vertex_is_inside_edge(previous, edge, rect);

    for current in polygon.iter().copied() {
        let current_inside = vertex_is_inside_edge(current, edge, rect);
        if current_inside != previous_inside {
            clipped.push(interpolate_at_clip_edge(previous, current, edge, rect));
        }
        if current_inside {
            clipped.push(current);
        }
        previous = current;
        previous_inside = current_inside;
    }
}

fn vertex_is_inside_edge(vertex: GlyphVertex, edge: ClipEdge, rect: [f32; 4]) -> bool {
    match edge {
        ClipEdge::Left => vertex.position[0] >= rect[0],
        ClipEdge::Right => vertex.position[0] <= rect[1],
        ClipEdge::Top => vertex.position[1] <= rect[2],
        ClipEdge::Bottom => vertex.position[1] >= rect[3],
    }
}

fn interpolate_at_clip_edge(
    start: GlyphVertex,
    end: GlyphVertex,
    edge: ClipEdge,
    rect: [f32; 4],
) -> GlyphVertex {
    let (axis, boundary) = match edge {
        ClipEdge::Left => (0, rect[0]),
        ClipEdge::Right => (0, rect[1]),
        ClipEdge::Top => (1, rect[2]),
        ClipEdge::Bottom => (1, rect[3]),
    };
    let factor = (boundary - start.position[axis]) / (end.position[axis] - start.position[axis]);
    let mut intersection = interpolate_vertex(start, end, factor);
    intersection.position[axis] = boundary;
    intersection
}

fn interpolate_vertex(start: GlyphVertex, end: GlyphVertex, factor: f32) -> GlyphVertex {
    GlyphVertex {
        position: interpolate_components(start.position, end.position, factor),
        tex_coords: interpolate_components(start.tex_coords, end.tex_coords, factor),
        color: [
            start.color[0] + (end.color[0] - start.color[0]) * factor,
            start.color[1] + (end.color[1] - start.color[1]) * factor,
            start.color[2] + (end.color[2] - start.color[2]) * factor,
            start.color[3] + (end.color[3] - start.color[3]) * factor,
        ],
    }
}

fn interpolate_components(start: [f32; 2], end: [f32; 2], factor: f32) -> [f32; 2] {
    [start[0] + (end[0] - start[0]) * factor, start[1] + (end[1] - start[1]) * factor]
}

/// 生成填充矩形的 6 个顶点（2 个三角形，NDC 空间）。

/// Generate rounded rectangle vertices using radial triangles.
/// Each corner uses `segments` angular steps; higher = smoother.
fn push_fill(
    vertices: &mut Vec<GlyphVertex>,
    rect: Rect,
    color: [f32; 4],
    radius: f32,
    screen: &Screen,
) {
    let r = radius.min(rect.w * 0.5).min(rect.h * 0.5);
    if r <= 0.0 {
        let ndc = screen.rect_to_ndc(rect);
        push_quad(vertices, ndc, color);
        return;
    }
    let segments = ((r * 5.0).ceil() as usize).clamp(8, 64);
    let n = segments.max(1);
    let nf = n as f32;
    let half_pi = std::f32::consts::FRAC_PI_2;

    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.w;
    let y1 = rect.y + rect.h;
    let sw = screen.w;
    let sh = screen.h;

    // Center rectangle (fill) covers the entire height from top to bottom
    let center_ndc = screen.rect_to_ndc(Rect::new(x0 + r, y0, rect.w - 2.0 * r, rect.h));
    push_quad(vertices, center_ndc, color);
    // Left edge
    let left_ndc = screen.rect_to_ndc(Rect::new(x0, y0 + r, r, rect.h - 2.0 * r));
    push_quad(vertices, left_ndc, color);
    // Right edge
    let right_ndc = screen.rect_to_ndc(Rect::new(x1 - r, y0 + r, r, rect.h - 2.0 * r));
    push_quad(vertices, right_ndc, color);

    // Corner wedges (TL, TR, BL, BR) with soft-edge feathering
    let corners: [(f32, f32, f32); 4] = [
        (x0 + r, y0 + r, half_pi),              // TL: top→left
        (x1 - r, y0 + r, 0.0),                  // TR: right→top
        (x0 + r, y1 - r, std::f32::consts::PI), // BL: left→bottom
        (x1 - r, y1 - r, 3.0 * half_pi),        // BR: bottom→right
    ];
    let feather_r = r + ROUNDED_FILL_FEATHER_PX;
    let mut color_alpha0 = color;
    color_alpha0[3] = 0.0;

    for (cx, cy, start_angle) in corners {
        let _c = corner_vertex(cx, cy, color, screen, None);
        for i in 0..n {
            let a = start_angle + (i as f32) / nf * half_pi;
            let b = start_angle + ((i + 1) as f32) / nf * half_pi;
            let ax = cx + r * a.cos();
            let ay = cy - r * a.sin();
            let bx = cx + r * b.cos();
            let by = cy - r * b.sin();
            // Feather outer edge points (alpha → 0 at feather_r)
            let fax = cx + feather_r * a.cos();
            let fay = cy - feather_r * a.sin();
            let fbx = cx + feather_r * b.cos();
            let fby = cy - feather_r * b.sin();
            let p = |x: f32, y: f32| GlyphVertex {
                position: [x / sw * 2.0 - 1.0, 1.0 - y / sh * 2.0],
                tex_coords: [0.0, 0.0],
                color,
            };
            let pf = |x: f32, y: f32| GlyphVertex {
                position: [x / sw * 2.0 - 1.0, 1.0 - y / sh * 2.0],
                tex_coords: [0.0, 0.0],
                color: color_alpha0,
            };
            // Solid inner triangle
            vertices.push(p(cx, cy));
            vertices.push(p(bx, by));
            vertices.push(p(ax, ay));
            // Feather ring: inner edge (full alpha) → outer edge (alpha=0)
            vertices.push(p(bx, by));
            vertices.push(pf(fbx, fby));
            vertices.push(pf(fax, fay));
            vertices.push(p(bx, by));
            vertices.push(pf(fax, fay));
            vertices.push(p(ax, ay));
        }
    }
}

/// 绘制圆角矩形的描边轮廓。
fn push_stroke(
    vertices: &mut Vec<GlyphVertex>,
    rect: Rect,
    color: [f32; 4],
    radius: f32,
    line_width: f32,
    screen: &Screen,
) {
    let r = radius.min(rect.w * 0.5).min(rect.h * 0.5);
    if r <= 0.0 || line_width <= 0.0 {
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.w;
        let y1 = rect.y + rect.h;
        let lw = line_width;
        let ndc = screen.rect_to_ndc(Rect::new(x0, y0, rect.w, lw));
        push_quad(vertices, ndc, color);
        let ndc = screen.rect_to_ndc(Rect::new(x0, y1 - lw, rect.w, lw));
        push_quad(vertices, ndc, color);
        let ndc = screen.rect_to_ndc(Rect::new(x0, y0 + lw, lw, rect.h - 2.0 * lw));
        push_quad(vertices, ndc, color);
        let ndc = screen.rect_to_ndc(Rect::new(x1 - lw, y0 + lw, lw, rect.h - 2.0 * lw));
        push_quad(vertices, ndc, color);
        return;
    }

    let segments = ((r * 5.0).ceil() as usize).clamp(8, 64);
    let n = segments.max(1);
    let nf = n as f32;
    let half_pi = std::f32::consts::FRAC_PI_2;

    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.w;
    let y1 = rect.y + rect.h;
    let lw = line_width;
    let r_inner = (r - lw).max(0.0);

    let sw = screen.w;
    let sh = screen.h;

    let p = |x: f32, y: f32| GlyphVertex {
        position: [x / sw * 2.0 - 1.0, 1.0 - y / sh * 2.0],
        tex_coords: [0.0, 0.0],
        color,
    };

    let top = screen.rect_to_ndc(Rect::new(x0 + r, y0, rect.w - 2.0 * r, lw));
    push_quad(vertices, top, color);
    let bot = screen.rect_to_ndc(Rect::new(x0 + r, y1 - lw, rect.w - 2.0 * r, lw));
    push_quad(vertices, bot, color);
    let left = screen.rect_to_ndc(Rect::new(x0, y0 + r, lw, rect.h - 2.0 * r));
    push_quad(vertices, left, color);
    let right = screen.rect_to_ndc(Rect::new(x1 - lw, y0 + r, lw, rect.h - 2.0 * r));
    push_quad(vertices, right, color);

    let corners: [(f32, f32, f32); 4] = [
        (x0 + r, y0 + r, half_pi),
        (x1 - r, y0 + r, 0.0),
        (x0 + r, y1 - r, std::f32::consts::PI),
        (x1 - r, y1 - r, 3.0 * half_pi),
    ];

    for (cx, cy, start_angle) in corners {
        for i in 0..n {
            let a0 = start_angle + (i as f32) / nf * half_pi;
            let a1 = start_angle + ((i + 1) as f32) / nf * half_pi;

            let ox0 = cx + r * a0.cos();
            let oy0 = cy - r * a0.sin();
            let ox1 = cx + r * a1.cos();
            let oy1 = cy - r * a1.sin();

            let ix0 = cx + r_inner * a0.cos();
            let iy0 = cy - r_inner * a0.sin();
            let ix1 = cx + r_inner * a1.cos();
            let iy1 = cy - r_inner * a1.sin();

            vertices.push(p(ix0, iy0));
            vertices.push(p(ox0, oy0));
            vertices.push(p(ox1, oy1));

            vertices.push(p(ix0, iy0));
            vertices.push(p(ox1, oy1));
            vertices.push(p(ix1, iy1));
        }
    }
}

fn corner_vertex(
    x_px: f32,
    y_px: f32,
    color: [f32; 4],
    screen: &Screen,
    _clip: Option<[f32; 4]>,
) -> GlyphVertex {
    let l = x_px / screen.w * 2.0 - 1.0;
    let t = 1.0 - y_px / screen.h * 2.0;
    GlyphVertex { position: [l, t], tex_coords: [0.0, 0.0], color }
}

fn push_quad(v: &mut Vec<GlyphVertex>, ndc: [f32; 4], color: [f32; 4]) {
    let [left, right, top, bottom] = ndc;
    let tl = [left, top];
    let tr = [right, top];
    let bl = [left, bottom];
    let br = [right, bottom];
    let uv = [0.0, 0.0];

    v.push(GlyphVertex { position: tl, tex_coords: uv, color });
    v.push(GlyphVertex { position: tr, tex_coords: uv, color });
    v.push(GlyphVertex { position: bl, tex_coords: uv, color });

    v.push(GlyphVertex { position: tr, tex_coords: uv, color });
    v.push(GlyphVertex { position: br, tex_coords: uv, color });
    v.push(GlyphVertex { position: bl, tex_coords: uv, color });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_mesh(
        vertices: [[f32; 2]; 3],
        alpha: [f32; 3],
    ) -> std::sync::Arc<ui::tapered_path::TaperedMesh> {
        use ui::tapered_path::{TaperedMesh, TaperedMeshVertex};

        let min_x = vertices.iter().map(|point| point[0]).fold(f32::INFINITY, f32::min);
        let max_x = vertices.iter().map(|point| point[0]).fold(f32::NEG_INFINITY, f32::max);
        let min_y = vertices.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
        let max_y = vertices.iter().map(|point| point[1]).fold(f32::NEG_INFINITY, f32::max);
        let mesh_vertices = vertices
            .into_iter()
            .zip(alpha)
            .map(|(position, alpha_multiplier)| TaperedMeshVertex { position, alpha_multiplier })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        std::sync::Arc::new(TaperedMesh {
            vertices: mesh_vertices,
            bounds: Rect::new(min_x, min_y, max_x - min_x, max_y - min_y),
        })
    }

    #[test]
    fn tapered_mesh_inside_clip_preserves_vertices_translation_and_alpha() {
        let mesh = triangle_mesh([[10.0, 10.0], [90.0, 10.0], [50.0, 90.0]], [1.0, 0.5, 0.0]);
        let translation = [20.0, 30.0];
        let clip = Rect::new(20.0, 30.0, 100.0, 100.0);
        assert_eq!(
            mesh_clip_relation(translated_rect(mesh.bounds, translation), Some(clip)),
            MeshClipRelation::Inside
        );
        assert_eq!(mesh_clip_relation(mesh.bounds, Some(clip)), MeshClipRelation::Intersecting);
        let mut list = DrawList::new();
        list.clip(clip, |inner| {
            inner.tapered_mesh(mesh, translation, [0.2, 0.4, 0.6, 0.8]);
        });

        let screen = Screen::new(200.0, 200.0);
        let vertices = drain(list, screen, None, None);
        assert_eq!(vertices.len(), 3);
        assert_eq!(vertices[0].position, screen.px_to_ndc(30.0, 40.0));
        assert_eq!(vertices[0].color, [0.2, 0.4, 0.6, 0.8]);
        assert_eq!(vertices[1].color, [0.2, 0.4, 0.6, 0.4]);
        assert_eq!(vertices[2].color, [0.2, 0.4, 0.6, 0.0]);
    }

    #[test]
    fn tapered_mesh_zero_area_clip_short_circuits_without_vertices() {
        let mesh = triangle_mesh([[10.0, 10.0], [90.0, 10.0], [50.0, 90.0]], [1.0; 3]);
        let clip = Rect::new(20.0, 0.0, 0.0, 100.0);
        assert_eq!(
            mesh_clip_relation(mesh.bounds, Some(clip)),
            MeshClipRelation::Outside,
            "零面积 clip 必须在遍历网格顶点前短路"
        );

        let mut list = DrawList::new();
        list.clip(clip, |inner| {
            inner.tapered_mesh(mesh, [0.0, 0.0], [1.0; 4]);
        });

        assert!(drain(list, Screen::new(100.0, 100.0), None, None).is_empty());
    }

    #[test]
    fn tapered_mesh_outside_clip_emits_no_vertices() {
        let mesh = triangle_mesh([[10.0, 10.0], [90.0, 10.0], [50.0, 90.0]], [1.0; 3]);
        let mut list = DrawList::new();
        list.clip(Rect::new(300.0, 300.0, 20.0, 20.0), |inner| {
            inner.tapered_mesh(mesh, [0.0, 0.0], [1.0; 4]);
        });

        assert!(drain(list, Screen::new(400.0, 400.0), None, None).is_empty());
    }

    #[test]
    fn tapered_mesh_crossing_clip_stays_inside_and_interpolates_alpha() {
        let mesh = triangle_mesh([[-20.0, 50.0], [120.0, 20.0], [120.0, 80.0]], [0.0, 1.0, 1.0]);
        let mut list = DrawList::new();
        let clip = Rect::new(0.0, 0.0, 100.0, 100.0);
        list.clip(clip, |inner| {
            inner.tapered_mesh(mesh, [0.0, 0.0], [0.3, 0.5, 0.7, 0.8]);
        });

        let screen = Screen::new(100.0, 100.0);
        let clip_ndc = screen.rect_to_ndc(clip);
        let vertices = drain(list, screen, None, None);
        assert!(!vertices.is_empty());
        assert!(vertices.iter().all(|vertex| {
            vertex.position[0] >= clip_ndc[0]
                && vertex.position[0] <= clip_ndc[1]
                && vertex.position[1] <= clip_ndc[2]
                && vertex.position[1] >= clip_ndc[3]
        }));
        assert!(vertices.iter().any(|vertex| vertex.color[3] > 0.0 && vertex.color[3] < 0.8));
    }

    fn clipping_test_vertex(position: [f32; 2], alpha: f32) -> GlyphVertex {
        GlyphVertex { position, tex_coords: position, color: [0.3, 0.5, 0.7, alpha] }
    }

    #[test]
    fn tapered_mesh_triangle_clip_relation_distinguishes_all_three_paths() {
        let clip_ndc = Screen::new(100.0, 100.0).rect_to_ndc(Rect::new(25.0, 25.0, 50.0, 50.0));
        let inside = [
            clipping_test_vertex([-0.25, 0.25], 1.0),
            clipping_test_vertex([0.25, 0.25], 0.5),
            clipping_test_vertex([0.0, -0.25], 0.0),
        ];
        let outside = [
            clipping_test_vertex([0.75, 0.25], 1.0),
            clipping_test_vertex([0.9, 0.0], 1.0),
            clipping_test_vertex([0.75, -0.25], 1.0),
        ];
        let intersecting = [
            clipping_test_vertex([-0.75, 0.0], 0.0),
            clipping_test_vertex([0.25, 0.25], 1.0),
            clipping_test_vertex([0.25, -0.25], 1.0),
        ];

        assert_eq!(triangle_clip_relation(&inside, clip_ndc), TriangleClipRelation::Inside);
        assert_eq!(triangle_clip_relation(&outside, clip_ndc), TriangleClipRelation::Outside);
        assert_eq!(
            triangle_clip_relation(&intersecting, clip_ndc),
            TriangleClipRelation::Intersecting
        );
    }

    #[test]
    fn tapered_mesh_triangle_clipping_emits_inside_skips_outside_and_clips_boundary() {
        let screen = Screen::new(100.0, 100.0);
        let clip = Rect::new(25.0, 25.0, 50.0, 50.0);
        let clip_ndc = screen.rect_to_ndc(clip);
        let inside = [
            clipping_test_vertex([-0.25, 0.25], 1.0),
            clipping_test_vertex([0.25, 0.25], 0.5),
            clipping_test_vertex([0.0, -0.25], 0.0),
        ];
        let outside = [
            clipping_test_vertex([0.75, 0.25], 1.0),
            clipping_test_vertex([0.9, 0.0], 1.0),
            clipping_test_vertex([0.75, -0.25], 1.0),
        ];
        let intersecting = [
            clipping_test_vertex([-0.75, 0.0], 0.0),
            clipping_test_vertex([0.25, 0.25], 1.0),
            clipping_test_vertex([0.25, -0.25], 1.0),
        ];
        let degenerate_inside = [
            clipping_test_vertex([-0.25, 0.0], 1.0),
            clipping_test_vertex([0.0, 0.0], 1.0),
            clipping_test_vertex([0.25, 0.0], 1.0),
        ];
        let tessellated = [inside, outside, intersecting, degenerate_inside].concat();
        let mut vertices = Vec::new();

        append_clipped_triangles(&mut vertices, &tessellated, clip, &screen);

        assert_eq!(vertices.len(), 9);
        for (actual, expected) in vertices.iter().zip(inside) {
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.tex_coords, expected.tex_coords);
            assert_eq!(actual.color, expected.color);
        }
        assert!(vertices.iter().all(|vertex| {
            vertex.position[0] >= clip_ndc[0]
                && vertex.position[0] <= clip_ndc[1]
                && vertex.position[1] <= clip_ndc[2]
                && vertex.position[1] >= clip_ndc[3]
        }));
        let expected_boundary_alpha = 0.25;
        assert!(vertices.iter().any(|vertex| {
            (vertex.position[0] - clip_ndc[0]).abs() < f32::EPSILON
                && (vertex.color[3] - expected_boundary_alpha).abs() < f32::EPSILON
        }));
    }

    fn point_is_covered(vertices: &[GlyphVertex], point: [f32; 2]) -> bool {
        vertices.chunks_exact(3).any(|triangle| {
            let edge_sign = |start: [f32; 2], end: [f32; 2]| {
                (point[0] - end[0]) * (start[1] - end[1])
                    - (start[0] - end[0]) * (point[1] - end[1])
            };
            let first = edge_sign(triangle[0].position, triangle[1].position);
            let second = edge_sign(triangle[1].position, triangle[2].position);
            let third = edge_sign(triangle[2].position, triangle[0].position);
            let has_negative =
                first < -f32::EPSILON || second < -f32::EPSILON || third < -f32::EPSILON;
            let has_positive =
                first > f32::EPSILON || second > f32::EPSILON || third > f32::EPSILON;

            !has_negative || !has_positive
        })
    }

    // ── FillRect 测试（不依赖 TextState）──

    #[test]
    fn empty_list_returns_empty() {
        let list = DrawList::new();
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        assert!(v.is_empty());
    }

    #[test]
    fn fill_rect_generates_6_vertices() {
        let mut list = DrawList::new();
        list.fill(Rect::new(100.0, 200.0, 300.0, 150.0), [1.0, 0.0, 0.0, 1.0]);
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        assert_eq!(v.len(), 6, "FillRect 应生成 6 个顶点");
    }

    #[test]
    fn fill_rect_vertices_in_ndc() {
        let mut list = DrawList::new();
        list.fill(Rect::new(0.0, 0.0, 800.0, 600.0), [1.0; 4]);
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        for vert in &v {
            assert!(vert.position[0] >= -1.0 && vert.position[0] <= 1.0);
            assert!(vert.position[1] >= -1.0 && vert.position[1] <= 1.0);
        }
    }

    #[test]
    fn fill_rect_preserves_color() {
        let mut list = DrawList::new();
        let color = [0.2, 0.4, 0.6, 0.8];
        list.fill(Rect::new(10.0, 10.0, 50.0, 50.0), color);
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        for vert in &v {
            assert_eq!(vert.color, color);
        }
    }

    #[test]
    fn push_pop_clip_constrains_fill() {
        let mut list = DrawList::new();
        list.clip(Rect::new(100.0, 100.0, 100.0, 100.0), |inner| {
            inner.fill(Rect::new(50.0, 50.0, 200.0, 200.0), [1.0; 4]);
        });
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        assert_eq!(v.len(), 6);
        let clip_ndc = screen.rect_to_ndc(Rect::new(100.0, 100.0, 100.0, 100.0));
        for vert in &v {
            assert!(vert.position[0] >= clip_ndc[0] - 0.001);
            assert!(vert.position[0] <= clip_ndc[1] + 0.001);
            assert!(vert.position[1] <= clip_ndc[2] + 0.001);
            assert!(vert.position[1] >= clip_ndc[3] - 0.001);
        }
    }

    #[test]
    fn clip_outside_produces_zero_vertices() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 10.0, 10.0), |inner| {
            inner.fill(Rect::new(100.0, 100.0, 50.0, 50.0), [1.0; 4]);
        });
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        assert!(v.is_empty(), "完全在裁剪外应生成 0 个顶点");
    }

    #[test]
    fn nested_clip_multiplies_constraints() {
        let mut list = DrawList::new();
        list.clip(Rect::new(100.0, 100.0, 200.0, 200.0), |outer| {
            outer.clip(Rect::new(150.0, 150.0, 100.0, 100.0), |inner| {
                inner.fill(Rect::new(0.0, 0.0, 500.0, 500.0), [1.0; 4]);
            });
        });
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        assert_eq!(v.len(), 6);
        let expected = Rect::new(150.0, 150.0, 100.0, 100.0);
        let ndc = screen.rect_to_ndc(expected);
        for vert in &v {
            assert!(vert.position[0] >= ndc[0] - 0.001);
            assert!(vert.position[0] <= ndc[1] + 0.001);
            assert!(vert.position[1] <= ndc[2] + 0.001);
            assert!(vert.position[1] >= ndc[3] - 0.001);
        }
    }

    // ── TextLayout 路径测试（不依赖真实 GPU 资源）──

    #[test]
    fn text_layout_no_text_state_produces_zero_vertices() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let layout = ui::core::text_layout::UiTextLayout::new(
            "hello",
            10.0,
            None,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            false,
            &mut shaper,
        )
        .unwrap();
        let mut list = DrawList::new();
        list.text_layout(std::sync::Arc::new(layout), 32.0, 791.5, [1.0; 4]);
        let screen = Screen::new(800.0, 600.0);
        let v = drain(list, screen, None, None);
        assert_eq!(v.len(), 0, "无 TextState 时 TextLayout 应跳过");
    }

    #[test]
    fn text_layout_no_text_state_mixed_with_fill() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let layout = ui::core::text_layout::UiTextLayout::new(
            "5,10",
            10.0,
            None,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            false,
            &mut shaper,
        )
        .unwrap();
        let mut list = DrawList::new();
        let color = [0.8, 0.8, 0.8, 1.0];
        list.text_layout(std::sync::Arc::new(layout), 32.0, 791.5, color);
        list.fill(Rect::new(0.0, 0.0, 100.0, 100.0), [0.5; 4]);
        let screen = Screen::new(1200.0, 800.0);
        let v = drain(list, screen, None, None);
        // FillRect 生成 6 顶点，TextLayout 跳过（无 TextState）
        assert_eq!(v.len(), 6);
    }

    #[test]
    fn text_layout_fill_mixed_in_same_list() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let layout = ui::core::text_layout::UiTextLayout::new(
            "1,1",
            10.0,
            None,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            false,
            &mut shaper,
        )
        .unwrap();
        let mut list = DrawList::new();
        list.fill(Rect::new(0.0, 776.0, 1200.0, 24.0), [0.1; 4]); // 背景
        list.text_layout(std::sync::Arc::new(layout), 32.0, 791.5, [0.8; 4]);
        let screen = Screen::new(1200.0, 800.0);
        let v = drain(list, screen, None, None);
        // 只有 FillRect 产生顶点
        assert_eq!(v.len(), 6);
    }
    #[test]
    fn fillrect_with_radius_emits_more_vertices_than_direct_quad() {
        let mut list = DrawList::new();
        list.fill_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 8.0);
        let v = drain(list, Screen::new(1000.0, 1000.0), None, None);
        assert!(v.len() > 6, "圆角应产生比直角更多顶点");
    }

    #[test]
    fn clipped_rounded_fill_keeps_a_straight_viewport_cut() {
        let mut list = DrawList::new();
        list.clip(Rect::new(20.0, 0.0, 80.0, 100.0), |inner| {
            inner.fill_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 10.0);
        });
        let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

        assert!(vertices.iter().any(|vertex| { vertex.position == [-0.6, 1.0] }));
        assert!(vertices.iter().any(|vertex| { vertex.position == [-0.6, -1.0] }));
    }

    #[test]
    fn rounded_fill_keeps_feather_geometry_when_clip_misses_base_rect() {
        let mut list = DrawList::new();
        list.clip(Rect::new(9.5, 19.9, 0.4, 0.2), |inner| {
            inner.fill_rounded(Rect::new(10.0, 10.0, 20.0, 20.0), [1.0; 4], 10.0);
        });
        let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

        assert!(!vertices.is_empty(), "裁剪区与外侧 0.5px 羽化环相交时必须保留几何");
    }

    #[test]
    fn rounded_fill_inside_empty_cumulative_clip_produces_no_vertices() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 10.0, 10.0), |outer| {
            outer.clip(Rect::new(20.0, 20.0, 10.0, 10.0), |inner| {
                inner.fill_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 10.0);
            });
        });
        let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

        assert!(vertices.is_empty(), "空的累计裁剪区不应生成退化三角形");
    }

    #[test]
    fn clipped_rounded_stroke_does_not_create_a_new_left_border() {
        let mut list = DrawList::new();
        list.clip(Rect::new(20.0, 0.0, 80.0, 100.0), |inner| {
            inner.stroke_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 10.0, 2.0);
        });
        let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

        assert!(!point_is_covered(&vertices, [-0.6, 0.0]), "裁剪边缘中部不应被合成的左侧描边覆盖");
    }

    #[test]
    fn zero_dimension_rounded_stroke_without_clip_produces_no_vertices() {
        for rect in [Rect::new(10.0, 10.0, 0.0, 20.0), Rect::new(10.0, 10.0, 20.0, 0.0)] {
            let mut list = DrawList::new();
            list.stroke_rounded(rect, [1.0; 4], 2.0, 4.0);
            let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

            assert!(vertices.is_empty(), "零宽或零高的圆角描边不应生成顶点: {rect:?}");
        }
    }

    #[test]
    fn nonempty_rounded_stroke_inside_empty_cumulative_clip_produces_no_vertices() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 100.0, 100.0), |outer| {
            outer.clip(Rect::new(10.0, 0.0, 0.0, 100.0), |inner| {
                inner.stroke_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 10.0, 2.0);
            });
        });
        let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

        assert!(vertices.is_empty(), "空的累计裁剪区不应生成退化描边三角形");
    }

    #[test]
    fn clipped_collinear_triangle_does_not_emit_degenerate_fan_triangle() {
        let collinear_triangle = [
            GlyphVertex { position: [-2.0, 0.0], tex_coords: [0.0; 2], color: [1.0; 4] },
            GlyphVertex { position: [0.0, 0.0], tex_coords: [0.5, 0.0], color: [1.0; 4] },
            GlyphVertex { position: [2.0, 0.0], tex_coords: [1.0, 0.0], color: [1.0; 4] },
        ];
        let mut vertices = Vec::new();

        append_clipped_triangles(
            &mut vertices,
            &collinear_triangle,
            Rect::new(25.0, 25.0, 50.0, 50.0),
            &Screen::new(100.0, 100.0),
        );

        assert!(vertices.is_empty(), "裁剪后的共线多边形不应生成退化扇形三角形");
    }

    #[test]
    fn wide_rounded_stroke_keeps_non_degenerate_outer_extension_inside_clip() {
        let mut list = DrawList::new();
        list.clip(Rect::new(11.0, 4.0, 2.0, 1.0), |inner| {
            inner.stroke_rounded(Rect::new(10.0, 10.0, 4.0, 4.0), [1.0; 4], 1.0, 10.0);
        });
        let vertices = drain(list, Screen::new(100.0, 100.0), None, None);

        assert!(
            vertices.chunks_exact(3).any(|triangle| {
                let signed_double_area = (triangle[1].position[0] - triangle[0].position[0])
                    * (triangle[2].position[1] - triangle[0].position[1])
                    - (triangle[2].position[0] - triangle[0].position[0])
                        * (triangle[1].position[1] - triangle[0].position[1]);
                signed_double_area.abs() * 0.5 > CLIPPED_TRIANGLE_AREA_EPSILON
            }),
            "仅与外伸部分相交的宽描边必须保留显著非零面积的三角形"
        );
    }

    #[test]
    fn rounded_stroke_bounds_expand_each_axis_by_excess_line_width() {
        let bounds = rounded_stroke_bounds(Rect::new(10.0, 20.0, 4.0, 8.0), 10.0);

        assert_eq!(bounds.x, 4.0);
        assert_eq!(bounds.y, 18.0);
        assert_eq!(bounds.w, 16.0);
        assert_eq!(bounds.h, 12.0);
    }

    #[test]
    fn clip_intersection_snaps_the_clipped_axis_to_the_boundary() {
        let boundary = -1.094_4;
        let start = GlyphVertex { position: [-2.0, 0.0], tex_coords: [0.0, 0.0], color: [1.0; 4] };
        let end = GlyphVertex { position: [-0.868, 1.0], tex_coords: [1.0, 1.0], color: [0.0; 4] };

        let intersection =
            interpolate_at_clip_edge(start, end, ClipEdge::Left, [boundary, 1.0, 1.0, -1.0]);

        assert_eq!(intersection.position[0], boundary);
    }

    #[test]
    fn clipped_rounded_vertices_stay_inside_the_cumulative_clip() {
        let cumulative_clip = Rect::new(19.37, 17.21, 53.19, 61.73);
        let mut list = DrawList::new();
        list.clip(Rect::new(5.0, 3.0, 90.0, 94.0), |outer| {
            outer.clip(cumulative_clip, |inner| {
                inner.fill_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 13.0);
            });
        });
        let screen = Screen::new(100.0, 100.0);
        let clip_ndc = screen.rect_to_ndc(cumulative_clip);
        let vertices = drain(list, screen, None, None);
        let tolerance = f32::EPSILON * 4.0;

        assert!(!vertices.is_empty());
        for vertex in vertices {
            assert!(vertex.position[0] >= clip_ndc[0] - tolerance);
            assert!(vertex.position[0] <= clip_ndc[1] + tolerance);
            assert!(vertex.position[1] <= clip_ndc[2] + tolerance);
            assert!(vertex.position[1] >= clip_ndc[3] - tolerance);
        }
    }

    // ── FillTriangle 测试 ──

    #[test]
    fn fill_triangle_generates_3_vertices() {
        let mut list = DrawList::new();
        list.fill_triangle([100.0, 100.0], [200.0, 100.0], [150.0, 200.0], [1.0; 4]);
        let v = drain(list, Screen::new(800.0, 600.0), None, None);
        assert_eq!(v.len(), 3, "FillTriangle 应生成 3 个顶点");
    }

    #[test]
    fn fill_triangle_skipped_when_all_points_outside_same_edge() {
        let mut list = DrawList::new();
        list.clip(Rect::new(100.0, 100.0, 200.0, 200.0), |inner| {
            // All 3 points to the left of clip
            inner.fill_triangle([10.0, 150.0], [20.0, 150.0], [15.0, 200.0], [1.0; 4]);
        });
        let v = drain(list, Screen::new(800.0, 600.0), None, None);
        assert!(v.is_empty(), "all points left of clip should skip triangle");
    }

    #[test]
    fn fill_triangle_not_skipped_when_points_on_different_edges() {
        let mut list = DrawList::new();
        list.clip(Rect::new(100.0, 100.0, 200.0, 200.0), |inner| {
            // p0 left of clip, p1 right of clip, p2 above clip
            // but triangle spans the clip region → should render
            inner.fill_triangle([50.0, 150.0], [350.0, 150.0], [200.0, 50.0], [1.0; 4]);
        });
        let v = drain(list, Screen::new(800.0, 600.0), None, None);
        assert_eq!(v.len(), 3, "triangle spanning clip should NOT be skipped");
    }

    #[test]
    fn fill_triangle_inside_clip_renders() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 800.0, 600.0), |inner| {
            inner.fill_triangle([100.0, 100.0], [200.0, 100.0], [150.0, 200.0], [1.0; 4]);
        });
        let v = drain(list, Screen::new(800.0, 600.0), None, None);
        assert_eq!(v.len(), 3, "triangle fully inside clip should render");
    }
}
