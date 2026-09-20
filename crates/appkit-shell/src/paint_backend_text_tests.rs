use super::*;
use shaping::{GlyphCluster, ShapedRun};
use ui::core::text_layout::UiTextLayout;
use ui::typography::{TextSpacingMode, TypographyCluster, TypographyInput};

const FONT_SIZE: f32 = 32.0;
const LATIN_ADVANCE: f32 = 16.0;
const POSITION_TOLERANCE: f32 = 0.001;

fn layout(text: &str, shaped: ShapedRun) -> UiTextLayout {
    UiTextLayout::from_shaped(
        text,
        FONT_SIZE,
        None,
        shaping::Weight::NORMAL,
        shaping::Style::Normal,
        false,
        shaped,
    )
}

fn raw_run(text: &str) -> ShapedRun {
    let clusters: Vec<_> = text
        .char_indices()
        .map(|(start, character)| GlyphCluster {
            byte_range: start..start + character.len_utf8(),
            glyph_id: 1,
            font_id: Default::default(),
            advance: if character.is_ascii() { LATIN_ADVANCE } else { FONT_SIZE },
            x_offset: 0.0,
            y_offset: 0.0,
        })
        .collect();
    ShapedRun { width: clusters.iter().map(|cluster| cluster.advance).sum(), clusters }
}

fn glyph_slot() -> render::GlyphSlot {
    render::GlyphSlot { x: 0, y: 0, width: 8, height: 12, page: 0, bearing_x: 0.0, bearing_y: 10.0 }
}

fn proportional_slot(_: &GlyphCluster, x: f32) -> Option<(f32, render::GlyphSlot)> {
    Some((render::split_subpixel(x).0, glyph_slot()))
}

#[test]
fn natural_spacing_preserves_every_letter_position_inside_agent() {
    let text = "从做工具到做工作流agent";
    let raw = raw_run(text);
    let clusters: Vec<_> = raw
        .clusters
        .iter()
        .map(|cluster| TypographyCluster::new(cluster.byte_range.clone(), FONT_SIZE))
        .collect();
    let gaps = ui::typography::calculate_gaps(&TypographyInput::new(
        text,
        TextSpacingMode::Natural,
        &clusters,
        &[],
    ));
    assert_eq!(gaps.len(), 1);
    let natural = ui::layout::typography::apply_gaps_to_existing_visual_lines(
        &raw,
        &gaps,
        &[(0, raw.clusters.len(), raw.width + gaps[0].advance)],
    );
    let raw_cache = build_cached_text_line(&layout(text, raw), 0.0, proportional_slot);
    let natural_layout = layout(text, natural);
    let natural_cache = build_cached_text_line(&natural_layout, 0.0, proportional_slot);
    let latin_start = natural_layout.shaped.clusters.len() - "agent".len();
    for index in latin_start..natural_cache.instances.len() {
        let shift = natural_cache.instances[index].x - raw_cache.instances[index].x;
        assert!(
            (shift - gaps[0].advance).abs() < POSITION_TOLERANCE,
            "every letter in agent must shift equally; letter {index} shifted {shift}"
        );
    }
    assert_eq!(natural_cache.visual_lines[0].2, natural_layout.shaped.width);
}

#[test]
fn raster_phase_includes_offset_without_applying_fraction_twice() {
    let mut shaped = raw_run("a");
    shaped.clusters[0].x_offset = 2.5;
    let mut phases = Vec::new();
    let cached = build_cached_text_line(&layout("a", shaped), 10.0, |_, x| {
        let (glyph_x, phase) = render::split_subpixel(x);
        phases.push(phase);
        Some((glyph_x, glyph_slot()))
    });
    assert_eq!(phases, vec![2], "rasterization must use the displaced glyph position");
    assert_eq!(cached.instances[0].x, 2.0, "fraction is already in the rasterized bitmap");
}

#[test]
fn zero_and_fractional_advances_survive_preview_cache() {
    let mut shaped = raw_run("abc");
    shaped.clusters[0].advance = 0.0;
    shaped.clusters[1].advance = 0.25;
    shaped.width = LATIN_ADVANCE + 0.25;
    let cached = build_cached_text_line(&layout("abc", shaped.clone()), 0.0, proportional_slot);
    assert_eq!(cached.cluster_data[0].2, 0.0);
    assert_eq!(cached.cluster_data[1].2, 0.25);
    assert_eq!(cached.instances[1].x, 0.0);
    assert_eq!(cached.visual_lines[0].2, shaped.width);
}

#[test]
fn vertical_offset_survives_cache_and_vertex_emission() {
    let mut shaped = raw_run("a");
    shaped.clusters[0].y_offset = 3.0;
    let cached = build_cached_text_line(&layout("a", shaped), 0.0, proportional_slot);
    let vertices =
        emit_from_instances(&cached.instances, 0.0, 40.0, 100.0, 100.0, [1.0; 4], &[], false);
    let top = (1.0 - vertices[0].position[1]) * 50.0;
    assert!(
        (top - 27.0).abs() < POSITION_TOLERANCE,
        "positive shaping y offset must raise glyph above baseline; top={top}"
    );
}

#[test]
fn cached_text_preserves_spacing_after_integer_translation() {
    let mut shaped = raw_run("ab");
    shaped.clusters[0].x_offset = 2.0;
    let text_layout = layout("ab", shaped);
    let cached = build_cached_text_line(&text_layout, 10.0, proportional_slot);
    let cold =
        emit_from_instances(&cached.instances, 10.0, 40.0, 200.0, 100.0, [1.0; 4], &[], false);
    let warm =
        emit_from_instances(&cached.instances, 30.0, 40.0, 200.0, 100.0, [1.0; 4], &[], false);
    for (first, translated) in cold.iter().zip(&warm) {
        assert!((translated.position[0] - first.position[0] - 0.2).abs() < POSITION_TOLERANCE);
        assert_eq!(translated.position[1], first.position[1]);
    }
}

#[test]
fn monospace_preview_preserves_advances_but_uses_integer_glyph_positions() {
    let mut shaper = shaping::Shaper::new()
        .expect("system fonts must load")
        .with_font_family("monospace")
        .with_font_size(FONT_SIZE);
    let shaped = shaper.shape("agent").expect("fixture must shape");
    let text_layout = layout("agent", shaped.clone());
    let origin = 10.25;
    let mut phases = Vec::new();
    let cached = build_cached_text_line(&text_layout, origin, |cluster, x| {
        let (glyph_x, phase) =
            crate::text_rasterize::glyph_position(&mut shaper, cluster.font_id, x);
        phases.push(phase);
        Some((glyph_x, glyph_slot()))
    });
    assert!(
        phases.iter().all(|&phase| phase == 0),
        "monospace glyphs must resolve zero-phase bitmaps: {phases:?}"
    );
    let mut x = origin;
    for (cluster, instance) in shaped.clusters.iter().zip(&cached.instances) {
        assert!((instance.x + origin - (x + cluster.x_offset).round()).abs() < POSITION_TOLERANCE);
        x += cluster.advance;
    }
    for (cluster, cached_cluster) in shaped.clusters.iter().zip(&cached.cluster_data) {
        assert_eq!(cluster.advance, cached_cluster.2);
    }
    assert!((cached.visual_lines[0].2 - shaped.width).abs() < POSITION_TOLERANCE);
}

#[test]
fn preview_cache_distinguishes_origins_that_change_pixel_alignment() {
    for (first, second) in [(10.0, 10.25), (10.25, 10.5), (0.5, -0.5)] {
        assert_ne!(preview_text_cache_key(42, first), preview_text_cache_key(42, second));
    }
    assert_eq!(preview_text_cache_key(42, 10.25), preview_text_cache_key(42, 10.25));
    assert_ne!(preview_text_cache_key(42, 10.25), preview_text_cache_key(43, 10.25));
}

#[test]
fn monospace_preview_cache_matches_fresh_geometry_after_origin_changes() {
    let mut shaper = shaping::Shaper::new()
        .expect("system fonts must load")
        .with_font_family("monospace")
        .with_font_size(FONT_SIZE);
    let text_layout = layout("agent", shaper.shape("agent").expect("fixture must shape"));
    let mut cache = crate::render_cache::PreviewRenderCache::new();
    let mut hits = 0;
    for origin in [0.5, -0.5, 10.25, 10.75, 10.25] {
        let fresh = build_cached_text_line(&text_layout, origin, |cluster, x| {
            let (glyph_x, phase) =
                crate::text_rasterize::glyph_position(&mut shaper, cluster.font_id, x);
            assert_eq!(phase, 0);
            Some((glyph_x, glyph_slot()))
        });
        let key = preview_text_cache_key(text_layout.id, origin);
        if cache.get(key).is_some() {
            hits += 1;
        } else {
            cache.insert(key, fresh.clone());
        }
        let cached = cache.get(key).expect("just inserted cache entry must be available");
        let actual = emit_from_instances(
            &cached.instances,
            origin,
            40.0,
            200.0,
            100.0,
            [1.0; 4],
            &[],
            false,
        );
        let expected =
            emit_from_instances(&fresh.instances, origin, 40.0, 200.0, 100.0, [1.0; 4], &[], false);
        for (actual_vertex, expected_vertex) in actual.iter().zip(&expected) {
            assert_eq!(actual_vertex.position, expected_vertex.position);
            assert_eq!(actual_vertex.tex_coords, expected_vertex.tex_coords);
        }
    }
    assert_eq!(hits, 1, "only the exact repeated origin should reuse pixel placement");
}
