//! Content hash for incremental rendering / reshape cache validation.
//!
//! Computes a deterministic u64 hash from a line's content and layout inputs.

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Compute a content hash for cache consistency checks.
///
/// Combines the line bytes and byte offset with viewport width and font size,
/// which affect word-wrap / visual-line breaks.
/// Viewport width is rounded to the nearest integer to avoid hash drift from
/// sub-pixel width changes (< 1 px) that do not affect word-wrap layout.
/// The same input always produces the same hash; different layout parameters
/// produce different hashes, forcing cache invalidation.
#[inline]
pub fn content_hash(
    line_bytes: &[u8],
    byte_offset: usize,
    viewport_width: f32,
    font_size: f32,
) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, line_bytes);
    hash_bytes(&mut hash, &(byte_offset as u64).to_le_bytes());
    hash_bytes(&mut hash, &(viewport_width.round() as u64).to_le_bytes());
    hash_bytes(&mut hash, &font_size.to_bits().to_le_bytes());
    hash
}

#[inline]
fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[cfg(test)]
mod tests {
    use super::content_hash;

    #[test]
    fn equal_length_ascii_and_cjk_content_have_distinct_hashes() {
        let viewport_width = 800.0;
        let font_size = 14.0;
        let ascii_heading = "# tokens";
        let cjk_heading = "# 企业";
        assert_eq!(ascii_heading.len(), cjk_heading.len(), "回归样例必须保持等字节长度");

        let ascii_hash = content_hash(ascii_heading.as_bytes(), 0, viewport_width, font_size);
        let cjk_hash = content_hash(cjk_heading.as_bytes(), 0, viewport_width, font_size);

        assert_ne!(ascii_hash, cjk_hash, "不同内容即使 UTF-8 字节长度相同，也不能复用同一渲染缓存");
    }
}
