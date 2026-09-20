use super::*;

fn reference_bitmap(
    shaper: &Shaper,
    cluster: &GlyphCluster,
    font_size: f32,
    hinted: bool,
    offset: (f32, f32),
) -> swash::scale::image::Image {
    let font = shaper.font_system().get_font(cluster.font_id).expect("fixture font must resolve");
    let mut context = swash::scale::ScaleContext::new();
    let mut scaler = context.builder(font.as_swash()).size(font_size).hint(hinted).build();
    swash::scale::Render::new(&[swash::scale::Source::Outline])
        .format(swash::zeno::Format::Alpha)
        .offset(swash::zeno::Vector::new(offset.0, offset.1))
        .render(&mut scaler, u16::try_from(cluster.glyph_id).expect("font glyph ID fits u16"))
        .expect("visible fixture glyph must rasterize")
}

fn assert_bitmap_matches(actual: &GlyphBitmap, reference: &swash::scale::image::Image) {
    assert_eq!(actual.width, reference.placement.width);
    assert_eq!(actual.height, reference.placement.height);
    assert_eq!(actual.left, reference.placement.left);
    assert_eq!(actual.top, reference.placement.top);
    assert_eq!(
        actual.data, reference.data,
        "rasterization must use the expected hinting and phase"
    );
}

#[test]
fn monospace_glyphs_use_unhinted_zero_phase_without_changing_shaping() {
    let mut shaper =
        Shaper::new().expect("system font database must load").with_font_family("monospace");
    for size in [16.0, 28.0, 32.0, 36.0] {
        shaper.set_font_size(size);
        let shaped = shaper.shape("agent Skill").expect("ASCII fixture must shape");
        for cluster in &shaped.clusters {
            if &"agent Skill"[cluster.byte_range.clone()] == " " {
                continue;
            }
            assert!(
                shaper
                    .font_system()
                    .db()
                    .face(cluster.font_id)
                    .expect("fixture face must exist")
                    .monospaced
            );
            let reference = reference_bitmap(&shaper, cluster, size, false, (0.0, 0.0));
            for offset in [(0.0, 0.0), (0.25, 0.0), (0.75, 0.5)] {
                let actual = shaper
                    .rasterize_glyph(
                        cluster.font_id,
                        u16::try_from(cluster.glyph_id).expect("font glyph ID fits u16"),
                        size,
                        offset,
                    )
                    .expect("visible glyph must rasterize");
                assert_bitmap_matches(&actual, &reference);
            }
        }
        assert_eq!(shaper.shape("agent Skill").expect("fixture must still shape"), shaped);
    }
}

#[test]
fn proportional_glyphs_keep_hinting_and_requested_phase() {
    let mut shaper =
        Shaper::new().expect("system font database must load").with_font_family("sans-serif");
    shaper.set_font_size(32.0);
    let shaped = shaper.shape("eg").expect("ASCII fixture must shape");
    for cluster in &shaped.clusters {
        assert!(
            !shaper
                .font_system()
                .db()
                .face(cluster.font_id)
                .expect("fixture face must exist")
                .monospaced
        );
        for offset in [(0.0, 0.0), (0.5, 0.25)] {
            let reference = reference_bitmap(&shaper, cluster, 32.0, true, offset);
            let actual = shaper
                .rasterize_glyph(
                    cluster.font_id,
                    u16::try_from(cluster.glyph_id).expect("font glyph ID fits u16"),
                    32.0,
                    offset,
                )
                .expect("visible glyph must rasterize");
            assert_bitmap_matches(&actual, &reference);
        }
    }
}

#[test]
fn raster_policy_uses_resolved_face_across_family_and_style_changes() {
    let mut shaper = Shaper::new().expect("system fonts must load");
    let mut mono_faces = Vec::new();
    for (weight, style) in [
        (Weight::NORMAL, Style::Normal),
        (Weight::BOLD, Style::Normal),
        (Weight::NORMAL, Style::Italic),
    ] {
        shaper.set_font_family(Some("monospace"));
        shaper.set_font_weight(weight);
        shaper.set_font_style(style);
        let shaped = shaper.shape("e").expect("styled fixture must shape");
        mono_faces.push(shaped.clusters[0].font_id);
    }
    shaper.set_font_family(Some("sans-serif"));
    shaper.set_font_weight(Weight::NORMAL);
    shaper.set_font_style(Style::Normal);
    let proportional =
        shaper.shape("e").expect("proportional fixture must shape").clusters[0].font_id;
    for font_id in mono_faces {
        assert_eq!(shaper.glyph_rasterization(font_id), GlyphRasterization::IntegerUnhinted);
    }
    assert_eq!(shaper.glyph_rasterization(proportional), GlyphRasterization::SubpixelHinted);
    shaper.set_font_family(Some("monospace"));
    assert_eq!(shaper.glyph_rasterization(proportional), GlyphRasterization::SubpixelHinted);
}

#[cfg(target_os = "macos")]
#[test]
fn menlo_chinese_fallback_preserves_each_faces_raster_policy() {
    let mut shaper = Shaper::new().expect("system fonts must load").with_font_family("Menlo");
    let shaped = shaper.shape("中文agent").expect("mixed fixture must shape");
    let mut found_mono = false;
    let mut found_proportional = false;
    for cluster in shaped.clusters {
        let monospaced = shaper
            .font_system()
            .db()
            .face(cluster.font_id)
            .expect("resolved face must exist")
            .monospaced;
        let policy = shaper.glyph_rasterization(cluster.font_id);
        assert_eq!(
            policy,
            if monospaced {
                GlyphRasterization::IntegerUnhinted
            } else {
                GlyphRasterization::SubpixelHinted
            }
        );
        found_mono |= monospaced;
        found_proportional |= !monospaced;
    }
    assert!(
        found_mono && found_proportional,
        "fixture must exercise both Menlo and proportional CJK fallback"
    );
}
