//! 直接验证生产光标与选区顶点落在正确的视觉行内，无需 GPU。

use std::time::Instant;

use render::GlyphVertex;
use ui::decorations::{cursor_vertices, selection_vertices};
use ui::render_geom::AdvanceCacheEntry;
use ui::settings::{Settings, UiMetrics};

const VIEWPORT_WIDTH: f32 = 800.0;
const VIEWPORT_HEIGHT: f32 = 600.0;
const HEADER_HEIGHT: f32 = 32.0;
const SCROLL_OFFSET: f32 = 5.0;
const GEOMETRY_TOLERANCE: f32 = 0.001;
const VERTICES_PER_QUAD: usize = 6;

fn vertical_pixel_bounds(vertices: &[GlyphVertex]) -> (f32, f32) {
    vertices.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(top, bottom), vertex| {
        let y = (1.0 - vertex.position[1]) * VIEWPORT_HEIGHT * 0.5;
        (top.min(y), bottom.max(y))
    })
}

fn selected_line_quads(selection: (usize, usize), metrics: &UiMetrics) -> Vec<GlyphVertex> {
    let line_offsets = [0, 4, 8, 12];
    let advance_cache: Vec<_> = (0..line_offsets.len())
        .map(|doc_line| AdvanceCacheEntry {
            doc_line,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: vec![(1, 10.0, 0), (2, 20.0, 1), (3, 30.0, 2)],
        })
        .collect();
    selection_vertices(
        Some(selection),
        &advance_cache,
        metrics,
        VIEWPORT_WIDTH,
        VIEWPORT_HEIGHT,
        0.0,
        &ui::theme::test_theme(),
        HEADER_HEIGHT,
        SCROLL_OFFSET,
        &line_offsets,
    )
}

#[test]
fn caret_vertices_stay_within_the_requested_visual_line() {
    let theme = ui::theme::test_theme();
    let metrics = UiMetrics::from_settings(&Settings::new(), 1.0);
    for line_index in [0, 2, 5] {
        let vertices = cursor_vertices(
            &theme,
            Some(line_index),
            HEADER_HEIGHT,
            100.0,
            Instant::now(),
            &metrics,
            VIEWPORT_WIDTH,
            VIEWPORT_HEIGHT,
            SCROLL_OFFSET,
            Some(1.0),
        );
        assert_eq!(vertices.len(), VERTICES_PER_QUAD);
        let (top, bottom) = vertical_pixel_bounds(&vertices);
        let line_top = HEADER_HEIGHT + SCROLL_OFFSET + line_index as f32 * metrics.line_height;
        assert!(top >= line_top - GEOMETRY_TOLERANCE);
        assert!(bottom <= line_top + metrics.line_height + GEOMETRY_TOLERANCE);
        assert!((bottom - top - metrics.font_size).abs() < GEOMETRY_TOLERANCE);
    }
}

#[test]
fn multiline_selection_vertices_cover_each_selected_visual_line() {
    let metrics = UiMetrics::from_settings(&Settings::new(), 1.0);
    let vertices = selected_line_quads((5, 14), &metrics);
    let selected_lines = 1..=3;
    assert_eq!(vertices.len(), selected_lines.clone().count() * VERTICES_PER_QUAD);
    for (line_index, quad) in selected_lines.zip(vertices.chunks_exact(VERTICES_PER_QUAD)) {
        let (top, bottom) = vertical_pixel_bounds(quad);
        let line_top = HEADER_HEIGHT + SCROLL_OFFSET + line_index as f32 * metrics.line_height;
        assert!((top - line_top).abs() < GEOMETRY_TOLERANCE);
        assert!((bottom - line_top - metrics.line_height).abs() < GEOMETRY_TOLERANCE);
    }
}

#[test]
fn single_line_selection_vertices_stay_on_the_selected_visual_line() {
    let metrics = UiMetrics::from_settings(&Settings::new(), 1.0);
    let vertices = selected_line_quads((5, 6), &metrics);
    assert_eq!(vertices.len(), VERTICES_PER_QUAD);
    let (top, bottom) = vertical_pixel_bounds(&vertices);
    let line_top = HEADER_HEIGHT + SCROLL_OFFSET + metrics.line_height;
    assert!((top - line_top).abs() < GEOMETRY_TOLERANCE);
    assert!((bottom - line_top - metrics.line_height).abs() < GEOMETRY_TOLERANCE);
}
