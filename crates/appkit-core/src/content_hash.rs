//! Content hash for incremental rendering / reshape cache validation.
//!
//! Computes a deterministic u64 hash from a line's layout inputs.

/// Compute a content hash for cache consistency checks.
///
/// Combines line length with viewport width and font size, which affect
/// word-wrap / visual-line breaks.
/// Viewport width is rounded to the nearest integer to avoid hash drift from
/// sub-pixel width changes (< 1 px) that do not affect word-wrap layout.
/// The same input always produces the same hash; different layout parameters
/// produce different hashes, forcing cache invalidation.
#[inline]
pub fn content_hash(
    _byte_offset: usize,
    byte_length: u32,
    viewport_width: f32,
    font_size: f32,
) -> u64 {
    (byte_length as u64)
        .wrapping_mul(31)
        .wrapping_add(viewport_width.round() as u64)
        .wrapping_mul(31)
        .wrapping_add(font_size.to_bits() as u64)
}
