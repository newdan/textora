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
fn proportional_glyphs_use_grid_fit_and_ignore_subpixel_offsets() {
    let mut shaper =
        Shaper::new().expect("system font database must load").with_font_family("sans-serif");
    shaper.set_font_size(32.0);
    let shaped = shaper.shape("eg").expect("ASCII fixture must shape");
    let expected = if cfg!(target_os = "macos") {
        GlyphRasterization::SubpixelFontRecommended
    } else {
        GlyphRasterization::IntegerGridFitted
    };
    for cluster in &shaped.clusters {
        assert!(
            !shaper
                .font_system()
                .db()
                .face(cluster.font_id)
                .expect("fixture face must exist")
                .monospaced
        );
        assert_eq!(shaper.glyph_rasterization(cluster.font_id), expected);
        for offset in [(0.0, 0.0), (0.5, 0.25)] {
            // macOS keeps subpixel positioning; Windows grid-fits and ignores
            // the requested phase.
            let reference_offset = if cfg!(target_os = "macos") { offset } else { (0.0, 0.0) };
            let reference = reference_bitmap(&shaper, cluster, 32.0, true, reference_offset);
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

#[cfg(target_os = "windows")]
fn assert_yahei_reference(shaper: &mut Shaper, shaped: &ShapedRun, family: &str, hinted: bool) {
    let size = shaper.font_size();
    for cluster in &shaped.clusters {
        assert!(
            shaper
                .font_system()
                .db()
                .face(cluster.font_id)
                .expect("fixture face must exist")
                .families
                .iter()
                .any(|(name, _)| name == family),
            "fixture must use the requested YaHei face"
        );
        assert_eq!(
            shaper.glyph_rasterization(cluster.font_id),
            GlyphRasterization::IntegerGridFitted
        );
        for offset in [(0.0, 0.0), (0.25, 0.0), (0.75, 0.5)] {
            let reference = reference_bitmap(shaper, cluster, size, hinted, (0.0, 0.0));
            let actual = shaper
                .rasterize_glyph(
                    cluster.font_id,
                    u16::try_from(cluster.glyph_id).expect("font glyph ID fits u16"),
                    size,
                    offset,
                )
                .expect("visible Chinese glyph must rasterize");
            assert_bitmap_matches(&actual, &reference);
        }
    }
}

#[cfg(target_os = "windows")]
#[test]
fn yahei_chinese_glyphs_follow_size_specific_hinting_at_requested_phase() {
    let mut shaper = Shaper::new().expect("system font database must load");
    for family in ["Microsoft YaHei", "Microsoft YaHei UI"] {
        shaper.set_font_family(Some(family));
        for weight in [Weight::NORMAL, Weight::BOLD] {
            shaper.set_font_weight(weight);
            for (size, hinted) in [(16.0, true), (24.0, false), (32.0, false), (16.0, true)] {
                shaper.set_font_size(size);
                let shaped = shaper.shape("请选择笔记").expect("Chinese fixture must shape");
                assert_yahei_reference(&mut shaper, &shaped, family, hinted);
                assert_eq!(shaper.shape("请选择笔记").expect("fixture must still shape"), shaped);
            }
        }
    }
}

#[cfg(target_os = "windows")]
#[test]
fn hinted_fractional_sizes_rasterize_on_the_whole_pixel_grid() {
    // Windows 125% scaling requests 15 * 1.25 = 18.75 px. Hinting that fraction
    // snapped stems for one ppem while the outline scaled for another, which
    // merged the two interior horizontals of 看 into a single thick bar.
    let mut shaper = Shaper::new().expect("system font database must load");
    shaper.set_font_family(Some("Microsoft YaHei UI"));
    for size in [16.75_f32, 17.5, 18.75] {
        let whole = size.round();
        shaper.set_font_size(size);
        let shaped = shaper.shape("看").expect("Chinese fixture must shape");
        let cluster = &shaped.clusters[0];
        let glyph_id = u16::try_from(cluster.glyph_id).expect("font glyph ID fits u16");
        let fractional = shaper
            .rasterize_glyph(cluster.font_id, glyph_id, size, (0.0, 0.0))
            .expect("visible Chinese glyph must rasterize");
        let reference = shaper
            .rasterize_glyph(cluster.font_id, glyph_id, whole, (0.0, 0.0))
            .expect("visible Chinese glyph must rasterize");
        assert_eq!(
            (fractional.width, fractional.height, fractional.left, fractional.top),
            (reference.width, reference.height, reference.left, reference.top),
            "size {size} must share the {whole} px placement"
        );
        assert_eq!(
            fractional.data, reference.data,
            "size {size} must hint on the {whole} px grid, not a fractional one"
        );
    }
}

#[test]
fn font_recommended_policy_preserves_fractional_positions_and_offsets() {
    let policy = GlyphRasterization::SubpixelFontRecommended;
    for coordinate in [10.25, 10.5, 10.75, -0.25, -0.5, -0.75] {
        assert_eq!(policy.align_x(coordinate), coordinate);
    }
    for offset in [(0.0, 0.0), (0.25, 0.0), (0.75, 0.5)] {
        assert_eq!(policy.raster_offset(offset), offset);
    }
}

#[cfg(target_os = "windows")]
#[test]
fn yahei_fallback_uses_resolved_face_policy_after_requested_family_changes() {
    let mut shaper =
        Shaper::new().expect("system font database must load").with_font_family("Consolas");
    shaper.set_font_size(24.0);
    let shaped = shaper.shape("A请").expect("mixed fixture must shape");
    let latin_face = shaped.clusters[0].font_id;
    let chinese = &shaped.clusters[1];
    assert_ne!(latin_face, chinese.font_id, "Chinese must exercise font fallback");
    assert_eq!(shaper.glyph_rasterization(latin_face), GlyphRasterization::IntegerUnhinted);
    assert_eq!(
        shaper.glyph_rasterization(chinese.font_id),
        GlyphRasterization::IntegerGridFitted,
        "resolved proportional fallback must follow its own font recommendations"
    );
    shaper.set_font_family(Some("Arial"));
    assert_eq!(shaper.glyph_rasterization(latin_face), GlyphRasterization::IntegerUnhinted);
    assert_eq!(shaper.glyph_rasterization(chinese.font_id), GlyphRasterization::IntegerGridFitted);
    let offset = (0.5, 0.0);
    let reference = reference_bitmap(&shaper, chinese, shaper.font_size(), false, (0.0, 0.0));
    let actual = shaper
        .rasterize_glyph(
            chinese.font_id,
            u16::try_from(chinese.glyph_id).expect("font glyph ID fits u16"),
            shaper.font_size(),
            offset,
        )
        .expect("fallback Chinese glyph must rasterize");
    assert_bitmap_matches(&actual, &reference);
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
    let proportional_expected = if cfg!(target_os = "macos") {
        GlyphRasterization::SubpixelFontRecommended
    } else {
        GlyphRasterization::IntegerGridFitted
    };
    assert_eq!(shaper.glyph_rasterization(proportional), proportional_expected);
    shaper.set_font_family(Some("monospace"));
    assert_eq!(shaper.glyph_rasterization(proportional), proportional_expected);
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
                GlyphRasterization::IntegerGridFitted
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
