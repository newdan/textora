use crate::render_geom::{AdvanceCacheEntry, compute_selection_highlight_quads};
use crate::settings::UiMetrics;
use crate::theme::Theme;
use core::highlight::HighlightKind;
use render::GlyphVertex;

/// Generate selection highlight vertices.
pub fn is_cursor_visible(
    cursor_blink_instant: std::time::Instant,
    dim_factor: Option<f32>,
) -> bool {
    dim_factor.is_some() || (cursor_blink_instant.elapsed().as_millis() / 500).rem_euclid(2) == 0
}

/// Push a cursor fill rect command to a DrawList using standard visibility and dimming rules.
pub fn draw_caret(
    dl: &mut crate::core::paint::DrawList,
    rect: crate::core::geom::Rect,
    theme: &Theme,
    cursor_blink_instant: std::time::Instant,
    dim_factor: Option<f32>,
) {
    if !is_cursor_visible(cursor_blink_instant, dim_factor) {
        return;
    }
    let mut color = theme.editor.cursor;
    if let Some(dim) = dim_factor {
        color[3] *= dim;
    }
    dl.fill(rect, color);
}

/// Generate selection highlight vertices.
pub fn selection_vertices(
    selection_range: Option<(usize, usize)>,
    advance_cache: &[AdvanceCacheEntry],
    metrics: &UiMetrics,
    screen_w: f32,
    screen_h: f32,
    left_margin: f32,
    theme: &Theme,
    tab_bar_height: f32,
    sub_line_offset: f32,
    line_offsets: &[usize],
) -> Vec<GlyphVertex> {
    let Some(range) = selection_range else { return vec![] };
    if range.0 >= range.1 {
        return vec![];
    }

    compute_selection_highlight_quads(
        advance_cache,
        range,
        line_offsets,
        screen_w,
        screen_h,
        metrics.line_height,
        sub_line_offset,
        left_margin,
        theme.editor.selection,
        tab_bar_height,
    )
}

/// Generate cursor vertices.
pub fn cursor_vertices(
    theme: &Theme,
    cursor_visual_line: Option<usize>,
    tab_bar_height: f32,
    cursor_pixel_x: f32,
    cursor_blink_instant: std::time::Instant,
    metrics: &UiMetrics,
    screen_w: f32,
    screen_h: f32,
    sub_line_offset: f32,
    dim_factor: Option<f32>,
) -> Vec<GlyphVertex> {
    if cursor_visual_line.is_none() {
        return vec![];
    }

    let visible = if let Some(_dim) = dim_factor {
        true // always visible when dimmed
    } else {
        let blink_ms = cursor_blink_instant.elapsed().as_millis();
        (blink_ms / 500).is_multiple_of(2)
    };
    if !visible {
        return vec![];
    }

    let cursor_width = 2.0 * metrics.dpi;
    let cursor_left = cursor_pixel_x - cursor_width * 0.5;

    let line_y = cursor_visual_line.unwrap() as f32 * metrics.line_height + sub_line_offset;
    let cursor_height = metrics.font_size;

    let y_base = line_y + tab_bar_height + metrics.line_height * 0.8;
    let cursor_top_y = y_base - cursor_height * 0.8;
    let cursor_bottom_y = y_base + cursor_height * 0.2;

    let left = cursor_left / screen_w * 2.0 - 1.0;
    let top = 1.0 - cursor_top_y / screen_h * 2.0;
    let right = (cursor_left + cursor_width) / screen_w * 2.0 - 1.0;
    let bottom = 1.0 - cursor_bottom_y / screen_h * 2.0;

    let uv = 0.0;
    let mut color = theme.editor.cursor;
    if let Some(dim) = dim_factor {
        color[3] *= dim;
    }

    vec![
        GlyphVertex { position: [left, top], tex_coords: [uv, uv], color },
        GlyphVertex { position: [right, top], tex_coords: [uv, uv], color },
        GlyphVertex { position: [left, bottom], tex_coords: [uv, uv], color },
        GlyphVertex { position: [right, top], tex_coords: [uv, uv], color },
        GlyphVertex { position: [right, bottom], tex_coords: [uv, uv], color },
        GlyphVertex { position: [left, bottom], tex_coords: [uv, uv], color },
    ]
}

/// Look up the highlight color for a byte offset within a line.
pub fn highlight_color_for_offset(
    spans: &[(usize, HighlightKind)],
    offset: usize,
    theme: &Theme,
) -> [f32; 4] {
    use core::highlight::highlight_kind_scope;
    let idx = spans.partition_point(|(start, _)| *start <= offset).saturating_sub(1);
    if let Some((_, kind)) = spans.get(idx) {
        let scope = highlight_kind_scope(*kind);
        theme.scope_color(scope)
    } else {
        theme.editor.foreground
    }
}

/// Generate search match highlight vertices — only for visible viewport.
pub fn search_match_vertices(
    matches: &[(usize, usize)],
    active_match_idx: usize,
    is_active: bool,
    advance_cache: &[AdvanceCacheEntry],
    metrics: &UiMetrics,
    screen_w: f32,
    screen_h: f32,
    left_margin: f32,
    theme: &Theme,
    tab_bar_height: f32,
    sub_line_offset: f32,
    line_offsets: &[usize],
) -> Vec<GlyphVertex> {
    if !is_active || matches.is_empty() || advance_cache.is_empty() {
        return vec![];
    }

    let first_line_abs = advance_cache
        .first()
        .and_then(|e| line_offsets.get(e.doc_line))
        .copied()
        .map(|lo| lo + advance_cache.first().unwrap().vl_byte_start)
        .unwrap_or(0);
    let last_entry = advance_cache.last().unwrap();
    let last_line_abs = line_offsets.get(last_entry.doc_line).copied().unwrap_or(0);
    // clusters store vl-local end offsets; add vl_byte_start to get line-local end
    let visible_end = last_line_abs
        + last_entry.vl_byte_start
        + last_entry.clusters.last().map(|&(end, _, _)| end).unwrap_or(0);

    let active_color = theme.palette.highlight;
    let inactive_color = theme.palette.inactive_highlight;

    let mut all_vertices = Vec::new();

    for (idx, range) in matches.iter().enumerate() {
        if range.1 <= first_line_abs || range.0 >= visible_end {
            continue;
        }

        let color = if idx == active_match_idx { active_color } else { inactive_color };

        let quads = compute_selection_highlight_quads(
            advance_cache,
            (range.0, range.1),
            line_offsets,
            screen_w,
            screen_h,
            metrics.line_height,
            sub_line_offset,
            left_margin,
            color,
            tab_bar_height,
        );
        all_vertices.extend(quads);
    }

    all_vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use core::highlight::HighlightKind;

    #[test]
    fn highlight_color_for_offset_foreground_fallback() {
        let theme = crate::theme::test_theme();
        let spans: Vec<(usize, HighlightKind)> = vec![];
        let color = highlight_color_for_offset(&spans, 0, &theme);
        assert_eq!(color, theme.editor.foreground);
    }

    #[test]
    fn highlight_color_for_offset_before_first_span() {
        let theme = crate::theme::test_theme();
        let spans = vec![(10usize, HighlightKind::Comment)];
        let color = highlight_color_for_offset(&spans, 5, &theme);
        assert_eq!(color, theme.scope_color("comment"));
    }

    #[test]
    fn highlight_color_for_offset_within_span() {
        let theme = crate::theme::test_theme();
        let spans = vec![(10usize, HighlightKind::Comment)];
        let color = highlight_color_for_offset(&spans, 15, &theme);
        assert_eq!(color, theme.scope_color("comment"));
    }

    #[test]
    fn cursor_vertices_empty_when_none() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        let verts = cursor_vertices(
            &theme,
            None,
            32.0,
            100.0,
            std::time::Instant::now(),
            &metrics,
            800.0,
            600.0,
            0.0,
            None,
        );
        assert!(verts.is_empty());
    }

    #[test]
    fn selection_vertices_empty_when_no_range() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        let verts =
            selection_vertices(None, &[], &metrics, 800.0, 600.0, 0.0, &theme, 32.0, 0.0, &[]);
        assert!(verts.is_empty());
    }

    // ── Dimmed cursor ──

    #[test]
    fn cursor_dimmed_always_visible() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        // Use a very old instant to ensure normal blink would be invisible
        let old_instant = std::time::Instant::now() - std::time::Duration::from_millis(600);
        let verts = cursor_vertices(
            &theme,
            Some(5),
            32.0,
            100.0,
            old_instant,
            &metrics,
            800.0,
            600.0,
            0.0,
            Some(0.4), // dimmed
        );
        // Should produce 6 vertices (2 triangles) even though blink says invisible
        assert_eq!(verts.len(), 6, "dimmed cursor should always be visible (6 vertices)");
    }

    #[test]
    fn cursor_dimmed_color_alpha_reduced() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        let instant = std::time::Instant::now();
        let dim_factor = 0.4;
        let verts = cursor_vertices(
            &theme,
            Some(5),
            32.0,
            100.0,
            instant,
            &metrics,
            800.0,
            600.0,
            0.0,
            Some(dim_factor),
        );
        assert!(!verts.is_empty());
        let original_alpha = theme.editor.cursor[3];
        let expected_alpha = original_alpha * dim_factor;
        for v in &verts {
            assert!(
                (v.color[3] - expected_alpha).abs() < 0.001,
                "dimmed cursor alpha should be {} * {} = {}, got {}",
                original_alpha,
                dim_factor,
                expected_alpha,
                v.color[3]
            );
        }
    }

    #[test]
    fn cursor_normal_blinks() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        // At time 0 (now), blink phase is visible
        let now = std::time::Instant::now();
        let verts =
            cursor_vertices(&theme, Some(5), 32.0, 100.0, now, &metrics, 800.0, 600.0, 0.0, None);
        // Now is instant, blink_ms is small, visible depends on timing
        // (blink_ms / 500).is_multiple_of(2) → very likely true for < 500ms
        // We can't assert exact vertex count due to timing, but we can assert
        // that dim_factor=None preserves original alpha
        if !verts.is_empty() {
            let original_alpha = theme.editor.cursor[3];
            for v in &verts {
                assert!(
                    (v.color[3] - original_alpha).abs() < 0.001,
                    "normal cursor alpha should match theme"
                );
            }
        }
    }

    #[test]
    fn cursor_vertices_physical_width_at_2x() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = crate::settings::UiMetrics::from_settings(&settings, 2.0);
        let cursor_pixel_x = 100.0;
        let screen_w = 800.0;
        let verts = cursor_vertices(
            &theme,
            Some(0),
            0.0,
            cursor_pixel_x,
            std::time::Instant::now(),
            &metrics,
            screen_w,
            600.0,
            0.0,
            None,
        );
        let left_ndc = verts[0].position[0];
        let right_ndc = verts[1].position[0];

        let left_px = (left_ndc + 1.0) / 2.0 * screen_w;
        let right_px = (right_ndc + 1.0) / 2.0 * screen_w;

        // Cursor width is 4.0 at 2.0 DPI (2.0 * 2.0)
        // Center should be 100.0, so left is 98.0, right is 102.0
        assert!((left_px - 98.0).abs() < 0.01, "Expected left to be 98.0, got {}", left_px);
        assert!((right_px - 102.0).abs() < 0.01, "Expected right to be 102.0, got {}", right_px);

        let cursor_width_px = (right_ndc - left_ndc) * screen_w / 2.0;
        assert!((cursor_width_px - 4.0).abs() < 0.01);
    }

    #[test]
    fn cursor_dimmed_does_not_blink() {
        let theme = crate::theme::test_theme();
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        // At 600ms offset, normal blink: 600/500 = 1, not multiple of 2 → invisible
        let old = std::time::Instant::now() - std::time::Duration::from_millis(600);
        let verts_normal =
            cursor_vertices(&theme, Some(5), 32.0, 100.0, old, &metrics, 800.0, 600.0, 0.0, None);
        assert!(verts_normal.is_empty(), "normal cursor should be invisible at 600ms blink");

        // Dimmed cursor at same time should still be visible
        let verts_dimmed = cursor_vertices(
            &theme,
            Some(5),
            32.0,
            100.0,
            old,
            &metrics,
            800.0,
            600.0,
            0.0,
            Some(0.4),
        );
        assert_eq!(verts_dimmed.len(), 6, "dimmed cursor should ignore blink and always show");
    }
}
