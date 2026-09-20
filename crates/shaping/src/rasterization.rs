use super::{FontId, Shaper};

/// Rendering policy belongs to the resolved face, including fallback fonts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphRasterization {
    IntegerUnhinted,
    SubpixelHinted,
}

impl GlyphRasterization {
    /// Align only the drawing position; shaping advances remain unchanged.
    pub fn align_x(self, x: f32) -> f32 {
        match self {
            Self::IntegerUnhinted => x.round(),
            Self::SubpixelHinted => x,
        }
    }

    pub(crate) fn uses_hinting(self) -> bool {
        matches!(self, Self::SubpixelHinted)
    }

    pub(crate) fn raster_offset(self, offset: (f32, f32)) -> (f32, f32) {
        match self {
            Self::IntegerUnhinted => (0.0, 0.0),
            Self::SubpixelHinted => offset,
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
            return GlyphRasterization::SubpixelHinted;
        };
        let policy = if face.monospaced {
            GlyphRasterization::IntegerUnhinted
        } else {
            GlyphRasterization::SubpixelHinted
        };
        self.rasterization_policies.insert(font_id, policy);
        policy
    }
}
