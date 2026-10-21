/// Physical-pixel geometry shared by the swapchain and its desktop composition clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowClip {
    pub width: f32,
    pub height: f32,
    pub radius: f32,
}

impl WindowClip {
    pub fn new(width: u32, height: u32, radius: Option<f32>) -> Self {
        let width = width as f32;
        let height = height as f32;
        Self {
            width,
            height,
            radius: radius.unwrap_or_default().max(0.0).min(width / 2.0).min(height / 2.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_clip_preserves_sixteen_logical_pixels_at_fractional_dpi() {
        for dpi in [1.0, 1.25, 1.28125, 1.5, 2.0] {
            let radius = 16.0 * dpi;
            assert_eq!(WindowClip::new(1200, 800, Some(radius)).radius, radius);
        }
    }

    #[test]
    fn removing_rounding_and_resizing_update_the_same_clip_bounds() {
        assert_eq!(
            WindowClip::new(1600, 900, None),
            WindowClip { width: 1600.0, height: 900.0, radius: 0.0 }
        );
        assert_eq!(WindowClip::new(30, 20, Some(32.0)).radius, 10.0);
    }
}
