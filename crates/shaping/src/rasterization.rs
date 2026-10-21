use super::{FontId, Shaper};

const GASP_HEADER_SIZE: usize = 4;
const GASP_RANGE_SIZE: usize = 4;
const GASP_VERSION_LEGACY: u16 = 0;
const GASP_VERSION_SYMMETRIC: u16 = 1;
const GASP_GRIDFIT: u16 = 0x0001;
const GASP_SYMMETRIC_GRIDFIT: u16 = 0x0004;

/// Smallest em size handed to the hinter, which cannot work with a zero grid.
const MIN_HINTED_PPEM: f32 = 1.0;

/// Rendering policy belongs to the resolved face, including fallback fonts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphRasterization {
    IntegerUnhinted,
    /// Snap glyph origins to whole pixels and let the hinter align stems to the
    /// pixel grid, matching Windows GridFit rendering.
    IntegerGridFitted,
    SubpixelFontRecommended,
}

impl GlyphRasterization {
    /// Align only the drawing position; shaping advances remain unchanged.
    pub fn align_x(self, x: f32) -> f32 {
        match self {
            Self::IntegerUnhinted | Self::IntegerGridFitted => x.round(),
            Self::SubpixelFontRecommended => x,
        }
    }

    /// Em size handed to the scaler.
    ///
    /// Grid-fitted faces run TrueType instructions on a whole-pixel ppem while
    /// skrifa scales the hinted outline with the requested size. A fractional
    /// request therefore snaps stems to a grid the outline was not drawn for:
    /// dense CJK glyphs drop strokes at Windows 125% scaling (15 px × 1.25 =
    /// 18.75 px), so those faces rasterize on the nearest whole pixel.
    /// Unhinted and CoreText-style faces keep the exact fractional size.
    pub(crate) fn raster_em(self, font_size: f32) -> f32 {
        match self {
            Self::IntegerGridFitted => font_size.round().max(MIN_HINTED_PPEM),
            Self::IntegerUnhinted | Self::SubpixelFontRecommended => font_size,
        }
    }

    pub(crate) fn uses_hinting(self, font: swash::FontRef<'_>, font_size: f32) -> bool {
        if self == Self::IntegerUnhinted {
            return false;
        }
        let ppem = self.raster_em(font_size).clamp(0.0, u16::MAX as f32) as u16;
        // Swash 0.1.19 fixes its hinter to horizontal LCD with symmetric rendering,
        // independently of the output mask format. Match that target's gasp flags.
        font.table(swash::tag_from_bytes(b"gasp"))
            .and_then(|table| lcd_hinting_recommendation(table, ppem))
            .unwrap_or(true)
    }

    pub(crate) fn raster_offset(self, offset: (f32, f32)) -> (f32, f32) {
        match self {
            Self::IntegerUnhinted | Self::IntegerGridFitted => (0.0, 0.0),
            Self::SubpixelFontRecommended => offset,
        }
    }
}

impl Shaper {
    pub fn glyph_rasterization(&mut self, font_id: FontId) -> GlyphRasterization {
        if let Some(&policy) = self.rasterization_policies.get(&font_id) {
            return policy;
        }
        let font_system = self.font_system.lock().expect("font database must not be poisoned");
        let Some(face) = font_system.db().face(font_id) else {
            return GlyphRasterization::SubpixelFontRecommended;
        };
        let policy = if face.monospaced {
            GlyphRasterization::IntegerUnhinted
        } else if cfg!(target_os = "macos") {
            // CoreText-based platforms rely on subpixel positioning for even
            // tracking; grid fitting there reads as jittery letter spacing.
            GlyphRasterization::SubpixelFontRecommended
        } else {
            GlyphRasterization::IntegerGridFitted
        };
        self.rasterization_policies.insert(font_id, policy);
        policy
    }
}

fn lcd_hinting_recommendation(table: &[u8], ppem: u16) -> Option<bool> {
    let header = table.get(..GASP_HEADER_SIZE)?;
    let version = u16::from_be_bytes([header[0], header[1]]);
    let gridfit_mask = match version {
        GASP_VERSION_LEGACY => GASP_GRIDFIT,
        GASP_VERSION_SYMMETRIC => GASP_SYMMETRIC_GRIDFIT,
        _ => return None,
    };
    let range_count = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let ranges = table.get(GASP_HEADER_SIZE..GASP_HEADER_SIZE + range_count * GASP_RANGE_SIZE)?;
    for range in ranges.chunks_exact(GASP_RANGE_SIZE) {
        let max_ppem = u16::from_be_bytes([range[0], range[1]]);
        if ppem <= max_ppem {
            let behavior = u16::from_be_bytes([range[2], range[3]]);
            return Some(behavior & gridfit_mask != 0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gasp_table(version: u16, ranges: &[(u16, u16)]) -> Vec<u8> {
        let mut table = Vec::new();
        table.extend(version.to_be_bytes());
        table.extend(
            u16::try_from(ranges.len()).expect("fixture range count fits u16").to_be_bytes(),
        );
        for &(max_ppem, behavior) in ranges {
            table.extend(max_ppem.to_be_bytes());
            table.extend(behavior.to_be_bytes());
        }
        table
    }

    #[test]
    fn lcd_hinting_follows_symmetric_flags_and_inclusive_size_boundaries() {
        let table = gasp_table(
            GASP_VERSION_SYMMETRIC,
            &[(21, GASP_SYMMETRIC_GRIDFIT), (u16::MAX, GASP_GRIDFIT)],
        );
        for (ppem, hinted) in [(16, true), (21, true), (22, false), (24, false), (16, true)] {
            assert_eq!(lcd_hinting_recommendation(&table, ppem), Some(hinted));
        }
    }

    #[test]
    fn legacy_gasp_uses_gridfit_flags() {
        let table = gasp_table(GASP_VERSION_LEGACY, &[(8, 0), (u16::MAX, GASP_GRIDFIT)]);
        assert_eq!(lcd_hinting_recommendation(&table, 8), Some(false));
        assert_eq!(lcd_hinting_recommendation(&table, 9), Some(true));
    }

    #[test]
    fn grid_fitted_faces_rasterize_on_whole_pixel_ems() {
        let grid_fitted = GlyphRasterization::IntegerGridFitted;
        for (requested, whole_pixels) in [(14.0, 14.0), (17.5, 18.0), (18.75, 19.0), (22.5, 23.0)] {
            assert_eq!(grid_fitted.raster_em(requested), whole_pixels, "size {requested}");
        }
        assert_eq!(grid_fitted.raster_em(0.25), MIN_HINTED_PPEM, "sub-pixel sizes keep a grid");
        for policy in
            [GlyphRasterization::IntegerUnhinted, GlyphRasterization::SubpixelFontRecommended]
        {
            for requested in [14.0, 17.5, 18.75, 22.5] {
                assert_eq!(
                    policy.raster_em(requested),
                    requested,
                    "{policy:?} must keep the exact em size"
                );
            }
        }
    }

    #[test]
    fn unavailable_gasp_recommendations_preserve_the_default() {
        let unsupported = gasp_table(GASP_VERSION_SYMMETRIC + 1, &[(u16::MAX, GASP_GRIDFIT)]);
        let empty = gasp_table(GASP_VERSION_SYMMETRIC, &[]);
        let incomplete = gasp_table(GASP_VERSION_SYMMETRIC, &[(8, GASP_SYMMETRIC_GRIDFIT)]);
        for table in [&[][..], &unsupported, &empty, &incomplete[..GASP_HEADER_SIZE], &incomplete] {
            assert_eq!(lcd_hinting_recommendation(table, 24), None);
        }
    }
}
