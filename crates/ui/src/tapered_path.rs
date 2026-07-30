use crate::core::geom::Rect;

pub const TAPERED_PATH_FEATHER_PX: f32 = 0.5;

const ZERO_LENGTH_EPSILON: f32 = 0.01;
const MIN_CAP_SEGMENTS: usize = 8;
const MAX_CAP_SEGMENTS: usize = 32;
const MAX_MITER_RATIO: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaperedMeshVertex {
    pub position: [f32; 2],
    pub alpha_multiplier: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaperedMesh {
    pub vertices: Box<[TaperedMeshVertex]>,
    pub bounds: Rect,
}

#[derive(Clone, Copy, Debug)]
pub struct TaperedPathInput<'a> {
    pub centerline: &'a [[f32; 2]],
    pub head_width: f32,
    pub tail_width: f32,
    pub scale: f32,
    pub feather_width: f32,
}

#[derive(Clone, Copy)]
struct PathSample {
    center: [f32; 2],
    tangent: [f32; 2],
    half_width: f32,
}

pub fn tessellate_tapered_path(input: TaperedPathInput<'_>) -> Option<TaperedMesh> {
    let scaled_head_width = input.head_width * input.scale;
    let scaled_tail_width = input.tail_width * input.scale;
    if !valid_input(input, scaled_head_width, scaled_tail_width) {
        return None;
    }

    let scaled_centerline = normalized_centerline(input)?;
    let samples = path_samples(&scaled_centerline, scaled_head_width, scaled_tail_width);
    if samples.len() < 2 {
        return None;
    }

    let mut vertices = Vec::new();
    append_body_triangles(&mut vertices, &samples);
    append_round_join_triangles(&mut vertices, &samples);
    append_round_cap_triangles(&mut vertices, samples[0], negate(samples[0].tangent));
    let tail_sample = samples[samples.len() - 1];
    append_round_cap_triangles(&mut vertices, tail_sample, tail_sample.tangent);
    append_feather_triangles(&mut vertices, &samples, input.feather_width);

    let bounds = mesh_bounds(&vertices)?;
    Some(TaperedMesh { vertices: vertices.into_boxed_slice(), bounds })
}

fn valid_input(input: TaperedPathInput<'_>, head_width: f32, tail_width: f32) -> bool {
    input.scale.is_finite()
        && input.scale > 0.0
        && input.feather_width.is_finite()
        && input.feather_width >= 0.0
        && head_width.is_finite()
        && tail_width.is_finite()
        && head_width > 0.0
        && tail_width > 0.0
}

fn normalized_centerline(input: TaperedPathInput<'_>) -> Option<Vec<[f32; 2]>> {
    let mut centerline = Vec::with_capacity(input.centerline.len());
    for &point in input.centerline {
        let scaled_point = [point[0] * input.scale, point[1] * input.scale];
        if !scaled_point[0].is_finite() || !scaled_point[1].is_finite() {
            return None;
        }
        if centerline.last().is_none_or(|last| distance(*last, scaled_point) >= ZERO_LENGTH_EPSILON)
        {
            centerline.push(scaled_point);
        }
    }

    (centerline.len() >= 2).then_some(centerline)
}

fn path_samples(centerline: &[[f32; 2]], head_width: f32, tail_width: f32) -> Vec<PathSample> {
    let segment_lengths: Vec<f32> =
        centerline.windows(2).map(|pair| distance(pair[0], pair[1])).collect();
    let total_length: f32 = segment_lengths.iter().sum();
    let mut accumulated_length = 0.0;

    centerline
        .iter()
        .enumerate()
        .map(|(index, &center)| {
            if index > 0 {
                accumulated_length += segment_lengths[index - 1];
            }
            let progress = accumulated_length / total_length;
            PathSample {
                center,
                tangent: sample_tangent(centerline, index),
                half_width: interpolated_half_width(head_width, tail_width, progress),
            }
        })
        .collect()
}

fn sample_tangent(centerline: &[[f32; 2]], index: usize) -> [f32; 2] {
    if index == 0 {
        return direction(centerline[0], centerline[1]);
    }
    if index + 1 == centerline.len() {
        return direction(centerline[index - 1], centerline[index]);
    }

    let incoming = direction(centerline[index - 1], centerline[index]);
    let outgoing = direction(centerline[index], centerline[index + 1]);
    normalize([incoming[0] + outgoing[0], incoming[1] + outgoing[1]]).unwrap_or(outgoing)
}

fn interpolated_half_width(head_width: f32, tail_width: f32, progress: f32) -> f32 {
    (head_width + (tail_width - head_width) * progress) * 0.5
}

fn append_body_triangles(vertices: &mut Vec<TaperedMeshVertex>, samples: &[PathSample]) {
    for pair in samples.windows(2) {
        let segment_direction = direction(pair[0].center, pair[1].center);
        let [start_left, start_right] =
            edge_points_for_direction(pair[0], segment_direction, pair[0].half_width);
        let [end_left, end_right] =
            edge_points_for_direction(pair[1], segment_direction, pair[1].half_width);
        append_triangle(vertices, start_left, start_right, end_left, 1.0);
        append_triangle(vertices, start_right, end_right, end_left, 1.0);
    }
}

fn append_round_join_triangles(vertices: &mut Vec<TaperedMeshVertex>, samples: &[PathSample]) {
    for index in 1..samples.len().saturating_sub(1) {
        let previous_direction = direction(samples[index - 1].center, samples[index].center);
        let next_direction = direction(samples[index].center, samples[index + 1].center);
        let turn = cross(previous_direction, next_direction);
        if turn.abs() < ZERO_LENGTH_EPSILON {
            if dot(previous_direction, next_direction) < 0.0 {
                append_full_circle_join(vertices, samples[index]);
            }
            continue;
        }
        if miter_within_limit(previous_direction, next_direction) {
            append_outer_turn_body_triangle(
                vertices,
                samples[index],
                previous_direction,
                next_direction,
                turn,
            );
            append_miter_join(vertices, samples[index], previous_direction, next_direction);
        } else {
            append_inner_miter_triangle(
                vertices,
                samples[index],
                previous_direction,
                next_direction,
                turn,
            );
            append_round_join(vertices, samples[index], previous_direction, next_direction);
        }
    }
}

fn append_full_circle_join(vertices: &mut Vec<TaperedMeshVertex>, sample: PathSample) {
    let start_offset = [sample.half_width, 0.0];
    append_arc_fan(
        vertices,
        sample.center,
        start_offset,
        start_offset,
        -1.0,
        sample.half_width,
        1.0,
    );
}

fn miter_within_limit(previous_direction: [f32; 2], next_direction: [f32; 2]) -> bool {
    let cosine = dot(previous_direction, next_direction).clamp(-1.0, 1.0);
    let half_angle_cosine = ((1.0 + cosine) * 0.5).sqrt();
    half_angle_cosine > 0.0 && half_angle_cosine.recip() <= MAX_MITER_RATIO
}

fn append_round_join(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
) {
    let turn = cross(previous_direction, next_direction);
    if turn.abs() < ZERO_LENGTH_EPSILON {
        return;
    }

    let side = -turn.signum();
    let start = scale(perpendicular(previous_direction), side * sample.half_width);
    let end = scale(perpendicular(next_direction), side * sample.half_width);
    append_arc_fan(vertices, sample.center, start, end, turn.signum(), sample.half_width, 1.0);
}

fn append_outer_turn_body_triangle(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    turn: f32,
) {
    let outer_side = -turn.signum();
    let previous_outer =
        offset_join_point(sample, previous_direction, outer_side, sample.half_width);
    let next_outer = offset_join_point(sample, next_direction, outer_side, sample.half_width);
    append_triangle(vertices, sample.center, previous_outer, next_outer, 1.0);
}

fn append_miter_join(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
) {
    append_miter_triangle(vertices, sample, previous_direction, next_direction, -1.0);
    append_miter_triangle(vertices, sample, previous_direction, next_direction, 1.0);
}

fn append_inner_miter_triangle(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    turn: f32,
) {
    append_miter_triangle(vertices, sample, previous_direction, next_direction, turn.signum());
}

fn append_miter_triangle(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    side: f32,
) {
    let previous_offset = scale(perpendicular(previous_direction), side * sample.half_width);
    let next_offset = scale(perpendicular(next_direction), side * sample.half_width);
    let Some(intersection) = offset_line_intersection(
        add(sample.center, previous_offset),
        previous_direction,
        add(sample.center, next_offset),
        next_direction,
    ) else {
        return;
    };
    append_triangle(
        vertices,
        add(sample.center, previous_offset),
        intersection,
        add(sample.center, next_offset),
        1.0,
    );
}

fn append_round_cap_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    outward_direction: [f32; 2],
) {
    let start = scale(perpendicular(outward_direction), -sample.half_width);
    let end = scale(perpendicular(outward_direction), sample.half_width);
    append_arc_fan(vertices, sample.center, start, end, 1.0, sample.half_width, 1.0);
}

fn append_feather_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    samples: &[PathSample],
    feather_width: f32,
) {
    if feather_width == 0.0 {
        return;
    }

    append_side_feather_triangles(vertices, samples, feather_width);
    append_join_feather_triangles(vertices, samples, feather_width);
    append_cap_feather_triangles(vertices, samples[0], negate(samples[0].tangent), feather_width);
    let tail_sample = samples[samples.len() - 1];
    append_cap_feather_triangles(vertices, tail_sample, tail_sample.tangent, feather_width);
}

fn append_side_feather_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    samples: &[PathSample],
    feather_width: f32,
) {
    for pair in samples.windows(2) {
        let segment_direction = direction(pair[0].center, pair[1].center);
        let [inner_start_left, inner_start_right] =
            edge_points_for_direction(pair[0], segment_direction, pair[0].half_width);
        let [inner_end_left, inner_end_right] =
            edge_points_for_direction(pair[1], segment_direction, pair[1].half_width);
        let [outer_start_left, outer_start_right] = edge_points_for_direction(
            pair[0],
            segment_direction,
            pair[0].half_width + feather_width,
        );
        let [outer_end_left, outer_end_right] = edge_points_for_direction(
            pair[1],
            segment_direction,
            pair[1].half_width + feather_width,
        );
        append_feather_quad(
            vertices,
            inner_start_left,
            inner_end_left,
            outer_start_left,
            outer_end_left,
        );
        append_feather_quad(
            vertices,
            inner_start_right,
            inner_end_right,
            outer_start_right,
            outer_end_right,
        );
    }
}

fn append_join_feather_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    samples: &[PathSample],
    feather_width: f32,
) {
    for index in 1..samples.len().saturating_sub(1) {
        let previous_direction = direction(samples[index - 1].center, samples[index].center);
        let next_direction = direction(samples[index].center, samples[index + 1].center);
        let turn = cross(previous_direction, next_direction);
        if turn.abs() < ZERO_LENGTH_EPSILON {
            if dot(previous_direction, next_direction) < 0.0 {
                append_full_circle_join_feather(vertices, samples[index], feather_width);
            }
            continue;
        }
        if miter_within_limit(previous_direction, next_direction) {
            append_miter_feather_join(
                vertices,
                samples[index],
                previous_direction,
                next_direction,
                feather_width,
            );
        } else {
            append_miter_feather_side(
                vertices,
                samples[index],
                previous_direction,
                next_direction,
                turn.signum(),
                feather_width,
            );
            append_round_join_feather(
                vertices,
                samples[index],
                previous_direction,
                next_direction,
                feather_width,
            );
        }
    }
}

fn append_full_circle_join_feather(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    feather_width: f32,
) {
    let start_offset = [sample.half_width, 0.0];
    append_arc_feather_ring(
        vertices,
        sample.center,
        start_offset,
        start_offset,
        -1.0,
        sample.half_width,
        sample.half_width + feather_width,
    );
}

fn append_miter_feather_join(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    feather_width: f32,
) {
    append_miter_feather_side(
        vertices,
        sample,
        previous_direction,
        next_direction,
        -1.0,
        feather_width,
    );
    append_miter_feather_side(
        vertices,
        sample,
        previous_direction,
        next_direction,
        1.0,
        feather_width,
    );
}

fn append_miter_feather_side(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    side: f32,
    feather_width: f32,
) {
    let inner_start = offset_join_point(sample, previous_direction, side, sample.half_width);
    let inner_end = offset_join_point(sample, next_direction, side, sample.half_width);
    let outer_width = sample.half_width + feather_width;
    let outer_start = offset_join_point(sample, previous_direction, side, outer_width);
    let outer_end = offset_join_point(sample, next_direction, side, outer_width);
    let Some(inner_intersection) =
        offset_line_intersection(inner_start, previous_direction, inner_end, next_direction)
    else {
        return;
    };
    let Some(outer_intersection) =
        offset_line_intersection(outer_start, previous_direction, outer_end, next_direction)
    else {
        return;
    };
    append_feather_quad(vertices, inner_start, inner_intersection, outer_start, outer_intersection);
    append_feather_quad(vertices, inner_intersection, inner_end, outer_intersection, outer_end);
}

fn append_round_join_feather(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    feather_width: f32,
) {
    let turn = cross(previous_direction, next_direction);
    let side = -turn.signum();
    let start_offset = scale(perpendicular(previous_direction), side * sample.half_width);
    let end_offset = scale(perpendicular(next_direction), side * sample.half_width);
    append_arc_feather_ring(
        vertices,
        sample.center,
        start_offset,
        end_offset,
        turn.signum(),
        sample.half_width,
        sample.half_width + feather_width,
    );
}

fn append_cap_feather_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    outward_direction: [f32; 2],
    feather_width: f32,
) {
    let segment_count = cap_segment_count(sample.half_width + feather_width);
    let outward_angle = outward_direction[1].atan2(outward_direction[0]);
    for segment in 0..segment_count {
        let start_angle = outward_angle - std::f32::consts::FRAC_PI_2
            + std::f32::consts::PI * segment as f32 / segment_count as f32;
        let end_angle = outward_angle - std::f32::consts::FRAC_PI_2
            + std::f32::consts::PI * (segment + 1) as f32 / segment_count as f32;
        let inner_start = point_at_angle(sample.center, sample.half_width, start_angle);
        let inner_end = point_at_angle(sample.center, sample.half_width, end_angle);
        let outer_start =
            point_at_angle(sample.center, sample.half_width + feather_width, start_angle);
        let outer_end = point_at_angle(sample.center, sample.half_width + feather_width, end_angle);
        append_feather_quad(vertices, inner_start, inner_end, outer_start, outer_end);
    }
}

fn append_feather_quad(
    vertices: &mut Vec<TaperedMeshVertex>,
    inner_start: [f32; 2],
    inner_end: [f32; 2],
    outer_start: [f32; 2],
    outer_end: [f32; 2],
) {
    append_triangle_with_alphas(vertices, inner_start, 1.0, inner_end, 1.0, outer_start, 0.0);
    append_triangle_with_alphas(vertices, inner_end, 1.0, outer_end, 0.0, outer_start, 0.0);
}

fn append_arc_fan(
    vertices: &mut Vec<TaperedMeshVertex>,
    center: [f32; 2],
    start_offset: [f32; 2],
    end_offset: [f32; 2],
    direction_sign: f32,
    radius: f32,
    alpha_multiplier: f32,
) {
    let segment_count = cap_segment_count(radius);
    let start_angle = start_offset[1].atan2(start_offset[0]);
    let end_angle = end_offset[1].atan2(end_offset[0]);
    let sweep = signed_sweep(start_angle, end_angle, direction_sign);
    let mut previous = point_at_angle(center, radius, start_angle);
    for segment in 1..=segment_count {
        let angle = start_angle + sweep * segment as f32 / segment_count as f32;
        let next = point_at_angle(center, radius, angle);
        append_triangle(vertices, center, previous, next, alpha_multiplier);
        previous = next;
    }
}

fn append_arc_feather_ring(
    vertices: &mut Vec<TaperedMeshVertex>,
    center: [f32; 2],
    start_offset: [f32; 2],
    end_offset: [f32; 2],
    direction_sign: f32,
    inner_radius: f32,
    outer_radius: f32,
) {
    let segment_count = cap_segment_count(outer_radius);
    let base_start_angle = start_offset[1].atan2(start_offset[0]);
    let base_end_angle = end_offset[1].atan2(end_offset[0]);
    let sweep = signed_sweep(base_start_angle, base_end_angle, direction_sign);
    for segment in 0..segment_count {
        let start_fraction = segment as f32 / segment_count as f32;
        let end_fraction = (segment + 1) as f32 / segment_count as f32;
        let segment_start_angle = base_start_angle + sweep * start_fraction;
        let segment_end_angle = base_start_angle + sweep * end_fraction;
        append_feather_quad(
            vertices,
            point_at_angle(center, inner_radius, segment_start_angle),
            point_at_angle(center, inner_radius, segment_end_angle),
            point_at_angle(center, outer_radius, segment_start_angle),
            point_at_angle(center, outer_radius, segment_end_angle),
        );
    }
}

fn mesh_bounds(vertices: &[TaperedMeshVertex]) -> Option<Rect> {
    let first = vertices.first()?;
    let mut minimum = first.position;
    let mut maximum = first.position;
    for vertex in vertices {
        if !vertex.position[0].is_finite()
            || !vertex.position[1].is_finite()
            || !vertex.alpha_multiplier.is_finite()
        {
            return None;
        }
        minimum[0] = minimum[0].min(vertex.position[0]);
        minimum[1] = minimum[1].min(vertex.position[1]);
        maximum[0] = maximum[0].max(vertex.position[0]);
        maximum[1] = maximum[1].max(vertex.position[1]);
    }
    Some(Rect::new(minimum[0], minimum[1], maximum[0] - minimum[0], maximum[1] - minimum[1]))
}

fn edge_points_for_direction(
    sample: PathSample,
    direction: [f32; 2],
    half_width: f32,
) -> [[f32; 2]; 2] {
    let offset = scale(perpendicular(direction), half_width);
    [add(sample.center, offset), subtract(sample.center, offset)]
}

fn offset_join_point(
    sample: PathSample,
    direction: [f32; 2],
    side: f32,
    half_width: f32,
) -> [f32; 2] {
    add(sample.center, scale(perpendicular(direction), side * half_width))
}

fn offset_line_intersection(
    first_point: [f32; 2],
    first_direction: [f32; 2],
    second_point: [f32; 2],
    second_direction: [f32; 2],
) -> Option<[f32; 2]> {
    let denominator = cross(first_direction, second_direction);
    if denominator.abs() < ZERO_LENGTH_EPSILON {
        return None;
    }
    let travel = cross(subtract(second_point, first_point), second_direction) / denominator;
    Some(add(first_point, scale(first_direction, travel)))
}

fn append_triangle(
    vertices: &mut Vec<TaperedMeshVertex>,
    first: [f32; 2],
    second: [f32; 2],
    third: [f32; 2],
    alpha_multiplier: f32,
) {
    append_triangle_with_alphas(
        vertices,
        first,
        alpha_multiplier,
        second,
        alpha_multiplier,
        third,
        alpha_multiplier,
    );
}

fn append_triangle_with_alphas(
    vertices: &mut Vec<TaperedMeshVertex>,
    first: [f32; 2],
    first_alpha: f32,
    second: [f32; 2],
    second_alpha: f32,
    third: [f32; 2],
    third_alpha: f32,
) {
    vertices.extend([
        TaperedMeshVertex { position: first, alpha_multiplier: first_alpha },
        TaperedMeshVertex { position: second, alpha_multiplier: second_alpha },
        TaperedMeshVertex { position: third, alpha_multiplier: third_alpha },
    ]);
}

fn cap_segment_count(radius: f32) -> usize {
    (radius.ceil() as usize).clamp(MIN_CAP_SEGMENTS, MAX_CAP_SEGMENTS)
}

fn signed_sweep(start_angle: f32, end_angle: f32, direction_sign: f32) -> f32 {
    let positive_sweep = (end_angle - start_angle).rem_euclid(std::f32::consts::TAU);
    if direction_sign >= 0.0 { positive_sweep } else { positive_sweep - std::f32::consts::TAU }
}

fn point_at_angle(center: [f32; 2], radius: f32, angle: f32) -> [f32; 2] {
    [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]
}

fn direction(start: [f32; 2], end: [f32; 2]) -> [f32; 2] {
    normalize(subtract(end, start)).expect("normalized centerline contains no zero-length segments")
}

fn normalize(vector: [f32; 2]) -> Option<[f32; 2]> {
    let length = (vector[0] * vector[0] + vector[1] * vector[1]).sqrt();
    (length >= ZERO_LENGTH_EPSILON).then_some([vector[0] / length, vector[1] / length])
}

fn distance(start: [f32; 2], end: [f32; 2]) -> f32 {
    let delta = subtract(end, start);
    (delta[0] * delta[0] + delta[1] * delta[1]).sqrt()
}

fn perpendicular(vector: [f32; 2]) -> [f32; 2] {
    [-vector[1], vector[0]]
}

fn add(left: [f32; 2], right: [f32; 2]) -> [f32; 2] {
    [left[0] + right[0], left[1] + right[1]]
}

fn subtract(left: [f32; 2], right: [f32; 2]) -> [f32; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn scale(vector: [f32; 2], factor: f32) -> [f32; 2] {
    [vector[0] * factor, vector[1] * factor]
}

fn negate(vector: [f32; 2]) -> [f32; 2] {
    [-vector[0], -vector[1]]
}

fn dot(left: [f32; 2], right: [f32; 2]) -> f32 {
    left[0] * right[0] + left[1] * right[1]
}

fn cross(left: [f32; 2], right: [f32; 2]) -> f32 {
    left[0] * right[1] - left[1] * right[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight_path(length: f32) -> [[f32; 2]; 2] {
        [[0.0, 0.0], [length, 0.0]]
    }

    fn mesh_for(centerline: &[[f32; 2]]) -> TaperedMesh {
        tessellate_tapered_path(TaperedPathInput {
            centerline,
            head_width: 10.0,
            tail_width: 2.0,
            scale: 1.0,
            feather_width: TAPERED_PATH_FEATHER_PX,
        })
        .expect("a finite two-point centerline must tessellate")
    }

    fn solid_mesh_covers_point(mesh: &TaperedMesh, point: [f32; 2]) -> bool {
        solid_triangle_count_containing_point(mesh, point) > 0
    }

    fn solid_triangle_count_containing_point(mesh: &TaperedMesh, point: [f32; 2]) -> usize {
        mesh.vertices
            .chunks_exact(3)
            .filter(|triangle| {
                if !triangle.iter().all(|vertex| vertex.alpha_multiplier == 1.0) {
                    return false;
                }

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
            .count()
    }

    fn outer_turn_probe(centerline: &[[f32; 2]; 3]) -> [f32; 2] {
        let samples = path_samples(centerline, 10.0, 2.0);
        let joint = samples[1];
        let previous_direction = direction(centerline[0], centerline[1]);
        let next_direction = direction(centerline[1], centerline[2]);
        let outer_side = -cross(previous_direction, next_direction).signum();
        let previous_outer =
            offset_join_point(joint, previous_direction, outer_side, joint.half_width);
        let next_outer = offset_join_point(joint, next_direction, outer_side, joint.half_width);

        [
            (joint.center[0] + previous_outer[0] + next_outer[0]) / 3.0,
            (joint.center[1] + previous_outer[1] + next_outer[1]) / 3.0,
        ]
    }

    #[test]
    fn positive_shallow_turn_has_solid_outer_body_coverage() {
        let centerline = [[0.0, 0.0], [100.0, 0.0], [119.615_71, 3.901_806_4]];
        let mesh = mesh_for(&centerline);
        let probe = outer_turn_probe(&centerline);

        assert!(
            solid_mesh_covers_point(&mesh, probe),
            "正向浅弯的外侧主体不得在分段接缝处露出背景"
        );
    }

    #[test]
    fn negative_shallow_turn_has_solid_outer_body_coverage() {
        let centerline = [[0.0, 0.0], [100.0, 0.0], [119.615_71, -3.901_806_4]];
        let mesh = mesh_for(&centerline);
        let probe = outer_turn_probe(&centerline);

        assert!(
            solid_mesh_covers_point(&mesh, probe),
            "反向浅弯的外侧主体不得在分段接缝处露出背景"
        );
    }

    #[test]
    fn straight_path_vertex_count_does_not_depend_on_pixel_length() {
        let short = mesh_for(&straight_path(100.0));
        let long = mesh_for(&straight_path(1_000.0));

        assert_eq!(short.vertices.len(), long.vertices.len());
        assert!(long.vertices.len() < 512);
    }

    #[test]
    fn straight_path_keeps_round_caps_taper_and_feather() {
        let mesh = mesh_for(&straight_path(1_000.0));

        assert!((mesh.bounds.x + 5.5).abs() < 0.01);
        assert!((mesh.bounds.right() - 1_001.5).abs() < 0.01);
        assert!((mesh.bounds.y + 5.5).abs() < 0.01);
        assert!((mesh.bounds.bottom() - 5.5).abs() < 0.01);
        assert!(mesh.vertices.iter().any(|vertex| vertex.alpha_multiplier == 0.0));
        assert!(mesh.vertices.iter().any(|vertex| vertex.alpha_multiplier == 1.0));
    }

    #[test]
    fn curved_and_reversing_paths_emit_only_finite_vertices() {
        for centerline in [
            vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]],
            vec![[0.0, 0.0], [50.0, 0.0], [0.0, 0.0]],
            vec![[0.0, 0.0], [0.0, 0.0], [50.0, 0.0]],
        ] {
            let mesh = mesh_for(&centerline);
            assert!(mesh.vertices.iter().all(|vertex| {
                vertex.position[0].is_finite()
                    && vertex.position[1].is_finite()
                    && vertex.alpha_multiplier.is_finite()
            }));
        }
    }

    #[test]
    fn right_angle_join_reaches_the_outer_miter_intersection() {
        let mesh = mesh_for(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]);

        assert!(mesh.vertices.iter().any(|vertex| {
            vertex.alpha_multiplier == 1.0
                && (vertex.position[0] - 103.0).abs() < 0.01
                && (vertex.position[1] + 3.0).abs() < 0.01
        }));
    }

    #[test]
    fn right_angle_miter_join_has_a_feathered_outer_intersection() {
        let mesh = mesh_for(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]);

        assert!(mesh.vertices.iter().any(|vertex| {
            vertex.alpha_multiplier == 0.0
                && (vertex.position[0] - 103.5).abs() < 0.01
                && (vertex.position[1] + 3.5).abs() < 0.01
        }));
    }

    #[test]
    fn sharp_round_join_has_a_feathered_outer_arc() {
        let mesh = mesh_for(&[[0.0, 0.0], [100.0, 0.0], [13.397_46, 50.0]]);

        assert!(mesh.vertices.iter().any(|vertex| {
            let offset_x = vertex.position[0] - 100.0;
            let offset_y = vertex.position[1];
            vertex.alpha_multiplier == 0.0
                && (offset_x * offset_x + offset_y * offset_y - 12.25).abs() < 0.01
                && offset_x > 0.0
                && offset_y < -3.0
        }));
    }

    #[test]
    fn sharp_round_join_does_not_overlap_solid_center_with_outer_body_triangle() {
        let centerline = [[0.0, 0.0], [100.0, 0.0], [13.397_46, 50.0]];
        let mesh = mesh_for(&centerline);
        let probe = outer_turn_probe(&centerline);

        assert_eq!(
            solid_triangle_count_containing_point(&mesh, probe),
            1,
            "锐弯 round 扇形内部不得与外侧主体三角形重复覆盖"
        );
    }

    #[test]
    fn reversing_join_emits_a_full_circle_and_feather_envelope() {
        let mesh = mesh_for(&[[0.0, 0.0], [100.0, 0.0], [0.0, 0.0]]);

        for (radius, alpha_multiplier) in [(3.0, 1.0), (3.5, 0.0)] {
            for expected_position in
                [[100.0 + radius, 0.0], [100.0 - radius, 0.0], [100.0, radius], [100.0, -radius]]
            {
                assert!(
                    mesh.vertices.iter().any(|vertex| {
                        vertex.alpha_multiplier == alpha_multiplier
                            && (vertex.position[0] - expected_position[0]).abs() < 0.01
                            && (vertex.position[1] - expected_position[1]).abs() < 0.01
                    }),
                    "折返点缺少半径 {radius}、alpha {alpha_multiplier} 的圆形顶点 {expected_position:?}"
                );
            }
        }

        assert!((mesh.bounds.right() - 103.5).abs() < 0.01);
    }

    #[test]
    fn invalid_path_inputs_do_not_emit_meshes() {
        for input in [
            TaperedPathInput {
                centerline: &[[0.0, 0.0]],
                head_width: 10.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            },
            TaperedPathInput {
                centerline: &[[0.0, 0.0], [f32::NAN, 1.0]],
                head_width: 10.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            },
            TaperedPathInput {
                centerline: &[[0.0, 0.0], [10.0, 0.0]],
                head_width: 0.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            },
        ] {
            assert!(tessellate_tapered_path(input).is_none());
        }
    }
}
