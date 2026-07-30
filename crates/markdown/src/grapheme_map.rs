//! Visual grapheme map for WYSIWYG editor coordinates.
//!
//! Converts char-indexed source-byte maps (produced by [`super::edit::materialize_line`])
//! into grapheme-cluster-indexed maps so that cursor, hit-test, and selection
//! never split a UAX#29 grapheme cluster (combining marks, ZWJ emoji, etc.).

use core::unicode::{
    ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done, ucd_grapheme_cluster_lookup,
};

/// Maps visual grapheme indices to absolute source byte offsets.
///
/// Index `g` holds the source byte at the *start* of the `g`-th grapheme
/// cluster.  One-past-end sentinel is included so that `.len()` returns the
/// grapheme count and `source_byte_at(grapheme_count)` returns the total byte
/// length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisualGraphemeMap {
    source_bytes_by_grapheme: Vec<usize>,
}

impl VisualGraphemeMap {
    /// Number of grapheme clusters (excluding the one-past-end sentinel).
    pub(crate) fn len(&self) -> usize {
        self.source_bytes_by_grapheme.len().saturating_sub(1)
    }

    /// Source byte at the start of grapheme `grapheme_index`.
    pub(crate) fn source_byte_at(&self, grapheme_index: usize) -> Option<usize> {
        self.source_bytes_by_grapheme.get(grapheme_index).copied()
    }

    /// Grapheme index whose byte range contains `source_byte`.
    ///
    /// Returns `None` only when the map is empty.  Bytes before the first
    /// grapheme snap to 0; bytes at or past the end snap to `len()-1`.
    pub(crate) fn byte_to_grapheme(&self, source_byte: usize) -> Option<usize> {
        if self.source_bytes_by_grapheme.len() <= 1 {
            return None;
        }
        // Find the largest grapheme boundary ≤ source_byte.
        match self.source_bytes_by_grapheme.binary_search(&source_byte) {
            Ok(i) => Some(i.min(self.len().saturating_sub(1))),
            Err(i) => Some(i.saturating_sub(1).min(self.len().saturating_sub(1))),
        }
    }

    /// Raw slice including the one-past-end sentinel.
    pub(crate) fn as_slice(&self) -> &[usize] {
        &self.source_bytes_by_grapheme
    }
}

/// Count grapheme clusters in `text` up to (but not including) `target_byte`.
///
/// Returns the grapheme index at the given byte position.  If `target_byte`
/// falls inside a multi-char grapheme cluster, returns the index of that
/// cluster's start.
pub(crate) fn grapheme_index_at_byte(text: &str, target_byte: usize) -> usize {
    let target_byte = target_byte.min(text.len());
    let mut grapheme_idx = 0usize;
    let mut chars = text.char_indices().peekable();

    while let Some((byte_start, ch)) = chars.next() {
        // Target is at or before this cluster's start.
        if target_byte <= byte_start {
            return grapheme_idx;
        }

        // Determine where this grapheme cluster ends.
        let mut prev_props = ucd_grapheme_cluster_lookup(ch);
        let mut state = 0u32;
        let mut cluster_end = byte_start + ch.len_utf8();

        while let Some(&(next_byte, next_ch)) = chars.peek() {
            let next_props = ucd_grapheme_cluster_lookup(next_ch);
            state = ucd_grapheme_cluster_joins(state, prev_props, next_props);
            if ucd_grapheme_cluster_joins_done(state) {
                break;
            }
            prev_props = next_props;
            chars.next();
            cluster_end = next_byte + next_ch.len_utf8();
        }

        // Target falls inside this cluster.
        if target_byte < cluster_end {
            return grapheme_idx;
        }

        grapheme_idx += 1;
    }

    grapheme_idx
}

/// Build a [`VisualGraphemeMap`] from a visual text string and a parallel
/// char-indexed source-byte array (as produced by materialized-line source
/// maps).
///
/// `source_bytes_by_char` must have `text.chars().count() + 1` entries
/// (the last entry is the one-past-end sentinel).
pub(crate) fn build_visual_grapheme_map(
    text: &str,
    source_bytes_by_char: &[usize],
) -> VisualGraphemeMap {
    let mut source_bytes_by_grapheme: Vec<usize> = Vec::new();

    let mut char_indices = text.char_indices().peekable();
    let mut ci = 0; // index into source_bytes_by_char

    while let Some((byte_start, ch)) = char_indices.next() {
        // Record the start of this grapheme cluster.
        let source_byte = source_bytes_by_char.get(ci).copied().unwrap_or(byte_start);
        source_bytes_by_grapheme.push(source_byte);

        // Advance through chars that extend the current grapheme cluster.
        let mut state = 0u32;
        let mut prev_props = ucd_grapheme_cluster_lookup(ch);

        while let Some(&(_, next_ch)) = char_indices.peek() {
            let next_props = ucd_grapheme_cluster_lookup(next_ch);
            state = ucd_grapheme_cluster_joins(state, prev_props, next_props);
            if ucd_grapheme_cluster_joins_done(state) {
                break;
            }
            prev_props = next_props;
            char_indices.next();
            ci += 1;
        }

        ci += 1;
    }

    // One-past-end sentinel: total byte length (or last entry in source_bytes_by_char).
    let total_bytes = source_bytes_by_char.last().copied().unwrap_or(text.len());
    source_bytes_by_grapheme.push(total_bytes);

    VisualGraphemeMap { source_bytes_by_grapheme }
}

/// Return the UTF-8 byte boundaries of every extended grapheme in `text`.
///
/// The returned vector always includes the one-past-end sentinel. This makes
/// it suitable for pairing each byte range with a corresponding visual edge.
pub(crate) fn grapheme_byte_boundaries(text: &str) -> Vec<usize> {
    let source_bytes_by_char: Vec<usize> = text
        .char_indices()
        .map(|(byte_offset, _)| byte_offset)
        .chain(std::iter::once(text.len()))
        .collect();

    build_visual_grapheme_map(text, &source_bytes_by_char).as_slice().to_vec()
}

/// Count grapheme clusters in `text` using UAX#29 extended grapheme cluster rules.
pub(crate) fn grapheme_count(text: &str) -> usize {
    let mut count = 0;
    let mut state = 0u32;
    let mut prev_props = 0usize;
    for ch in text.chars() {
        let props = ucd_grapheme_cluster_lookup(ch);
        let joins = ucd_grapheme_cluster_joins(state, prev_props, props);
        if ucd_grapheme_cluster_joins_done(joins) {
            count += 1;
            state = 0;
        } else {
            state = joins;
        }
        prev_props = props;
    }
    count
}

/// Find the byte position at the start of grapheme `target_grapheme` within `text`.
///
/// Returns `text.len()` if `target_grapheme` is past the end.
pub(crate) fn byte_at_grapheme_index(text: &str, target_grapheme: usize) -> usize {
    if target_grapheme == 0 {
        return 0;
    }
    let mut gi = 0usize;
    let mut state = 0u32;
    let mut prev_props = 0usize;
    for (byte, ch) in text.char_indices() {
        let props = ucd_grapheme_cluster_lookup(ch);
        let joins = ucd_grapheme_cluster_joins(state, prev_props, props);
        let is_boundary = ucd_grapheme_cluster_joins_done(joins);
        if is_boundary {
            if gi == target_grapheme {
                return byte;
            }
            gi += 1;
            state = 0;
        } else {
            state = joins;
        }
        prev_props = props;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a source_bytes_by_char array from a string for testing.
    fn char_sentinel_map(text: &str) -> Vec<usize> {
        let mut v: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        v.push(text.len());
        v
    }

    // ── ASCII ──────────────────────────────────────────────────────────

    #[test]
    fn grapheme_map_ascii_one_grapheme_per_char() {
        let text = "abc";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        assert_eq!(map.len(), 3);
        assert_eq!(map.source_byte_at(0), Some(0));
        assert_eq!(map.source_byte_at(1), Some(1));
        assert_eq!(map.source_byte_at(2), Some(2));
        assert_eq!(map.source_byte_at(3), Some(3)); // sentinel
    }

    #[test]
    fn grapheme_map_byte_to_grapheme_ascii() {
        let text = "abc";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        assert_eq!(map.byte_to_grapheme(0), Some(0));
        assert_eq!(map.byte_to_grapheme(1), Some(1));
        assert_eq!(map.byte_to_grapheme(2), Some(2));
    }

    // ── NFD combining mark ─────────────────────────────────────────────

    #[test]
    fn grapheme_map_treats_nfd_combining_as_one_position() {
        let text = "xe\u{0301}y";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        // "x" (1 char), "é" (2 chars: e + combining acute), "y" (1 char) = 3 graphemes
        // combining acute \u{0301} is 2 bytes (0xCC 0x81), so 'y' starts at byte 4
        assert_eq!(map.len(), 3);
        assert_eq!(map.source_byte_at(0), Some(0)); // 'x'
        assert_eq!(map.source_byte_at(1), Some(1)); // 'e' + combining
        assert_eq!(map.source_byte_at(2), Some(4)); // 'y'
        assert_eq!(map.source_byte_at(3), Some(5)); // sentinel
        assert_eq!(map.byte_to_grapheme(2), Some(1)); // combining mark → same grapheme as 'e'
        assert_eq!(map.byte_to_grapheme(3), Some(1)); // byte inside combining mark
    }

    // ── ZWJ emoji ──────────────────────────────────────────────────────

    #[test]
    fn grapheme_map_treats_zwj_emoji_as_one_position() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let text = format!("x{emoji}y");
        let source_by_char = char_sentinel_map(&text);
        let map = build_visual_grapheme_map(&text, &source_by_char);

        // "x" (1), ZWJ emoji (1), "y" (1) = 3 graphemes
        assert_eq!(map.len(), 3);
        assert_eq!(map.source_byte_at(0), Some(0)); // 'x'
        assert_eq!(map.source_byte_at(1), Some(1)); // ZWJ emoji start
        assert_eq!(map.source_byte_at(2), Some(1 + emoji.len())); // 'y'
    }

    #[test]
    fn grapheme_map_byte_to_grapheme_inside_zwj_returns_cluster_start() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let text = format!("x{emoji}y");
        let source_by_char = char_sentinel_map(&text);
        let map = build_visual_grapheme_map(&text, &source_by_char);

        // Any byte within the emoji cluster should belong to grapheme 1.
        let emoji_start = 1;
        let emoji_end = emoji_start + emoji.len();
        assert_eq!(map.byte_to_grapheme(emoji_start), Some(1));
        assert_eq!(map.byte_to_grapheme(emoji_start + 3), Some(1));
        assert_eq!(map.byte_to_grapheme(emoji_end - 1), Some(1));
    }

    // ── Variation selector ─────────────────────────────────────────────

    #[test]
    fn grapheme_map_treats_variation_selector_as_one_position() {
        let text = format!("x{}\u{FE0F}y", '\u{2708}'); // ✈ + VS16
        let source_by_char = char_sentinel_map(&text);
        let map = build_visual_grapheme_map(&text, &source_by_char);

        // "x", "✈️" (2 chars: airplane + VS16), "y" = 3 graphemes
        assert_eq!(map.len(), 3);
        assert_eq!(map.source_byte_at(0), Some(0));
        assert_eq!(map.source_byte_at(1), Some(1));
        assert_eq!(map.source_byte_at(2), Some(1 + '\u{2708}'.len_utf8() + '\u{FE0F}'.len_utf8()));
    }

    // ── CJK (should be 1:1 char→grapheme still) ────────────────────────

    #[test]
    fn grapheme_map_cjk_each_char_is_own_grapheme() {
        let text = "你好世界";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        assert_eq!(map.len(), 4);
        assert_eq!(map.source_byte_at(0), Some(0));
        assert_eq!(map.source_byte_at(1), Some(3));
        assert_eq!(map.source_byte_at(2), Some(6));
        assert_eq!(map.source_byte_at(3), Some(9));
    }

    #[test]
    fn grapheme_index_at_byte_snaps_to_cluster_start() {
        // "e\u{0301}" → 1 grapheme cluster (e + combining acute), 3 bytes
        let text = "xe\u{0301}y";
        // byte 0: x, bytes 1-3: é, byte 4: y
        assert_eq!(grapheme_index_at_byte(text, 0), 0); // 'x' start
        assert_eq!(grapheme_index_at_byte(text, 1), 1); // 'é' start
        assert_eq!(grapheme_index_at_byte(text, 2), 1); // inside combining acute
        assert_eq!(grapheme_index_at_byte(text, 3), 1); // still inside combining acute
        assert_eq!(grapheme_index_at_byte(text, 4), 2); // 'y' start
        assert_eq!(grapheme_index_at_byte(text, 5), 3); // past end → grapheme_count
    }

    #[test]
    fn grapheme_index_at_byte_ascii() {
        assert_eq!(grapheme_index_at_byte("abc", 0), 0);
        assert_eq!(grapheme_index_at_byte("abc", 1), 1);
        assert_eq!(grapheme_index_at_byte("abc", 2), 2);
        assert_eq!(grapheme_index_at_byte("abc", 3), 3); // past end → grapheme_count
    }

    #[test]
    fn grapheme_index_at_byte_empty() {
        assert_eq!(grapheme_index_at_byte("", 0), 0);
        assert_eq!(grapheme_index_at_byte("", 5), 0);
    }

    // ── Empty string ───────────────────────────────────────────────────

    #[test]
    fn grapheme_map_empty_string() {
        let text = "";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        assert_eq!(map.len(), 0);
        assert_eq!(map.as_slice(), &[0]);
    }

    // ── grapheme_count ───────────────────────────────────────────────────

    #[test]
    fn grapheme_count_ascii() {
        assert_eq!(grapheme_count("abc"), 3);
    }

    #[test]
    fn grapheme_count_nfd_combining() {
        assert_eq!(grapheme_count("xe\u{0301}y"), 3); // x, é, y
    }

    #[test]
    fn grapheme_count_zwj_emoji() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(grapheme_count(&format!("x{emoji}y")), 3); // x, emoji, y
    }

    #[test]
    fn grapheme_count_cjk() {
        assert_eq!(grapheme_count("你好世界"), 4);
    }

    #[test]
    fn grapheme_count_empty() {
        assert_eq!(grapheme_count(""), 0);
    }

    // ── byte_at_grapheme_index ───────────────────────────────────────────

    #[test]
    fn byte_at_grapheme_ascii() {
        assert_eq!(byte_at_grapheme_index("abc", 0), 0);
        assert_eq!(byte_at_grapheme_index("abc", 1), 1);
        assert_eq!(byte_at_grapheme_index("abc", 2), 2);
        assert_eq!(byte_at_grapheme_index("abc", 3), 3); // sentinel
    }

    #[test]
    fn byte_at_grapheme_skips_combining() {
        let text = "xe\u{0301}y";
        assert_eq!(byte_at_grapheme_index(text, 0), 0); // 'x'
        assert_eq!(byte_at_grapheme_index(text, 1), 1); // 'e' + combining
        assert_eq!(byte_at_grapheme_index(text, 2), 4); // 'y'
        assert_eq!(byte_at_grapheme_index(text, 3), 5); // sentinel
    }

    #[test]
    fn byte_at_grapheme_skips_zwj() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let text = format!("x{emoji}y");
        assert_eq!(byte_at_grapheme_index(&text, 0), 0); // 'x'
        assert_eq!(byte_at_grapheme_index(&text, 1), 1); // emoji start
        assert_eq!(byte_at_grapheme_index(&text, 2), 1 + emoji.len()); // 'y'
    }

    #[test]
    fn byte_at_grapheme_cjk() {
        assert_eq!(byte_at_grapheme_index("你好", 0), 0);
        assert_eq!(byte_at_grapheme_index("你好", 1), 3);
        assert_eq!(byte_at_grapheme_index("你好", 2), 6); // sentinel
    }

    // ── Wrap segment boundary tests ──────────────────────────────────────

    /// A segment's source map has exactly `grapheme_count + 1` entries,
    /// where the +1 is the one-past-end sentinel (byte of next segment's start).
    #[test]
    fn segment_source_map_has_grapheme_count_plus_one() {
        // Simulate a line "abcde" split at wrap point byte 3 → ["abc", "de"]
        let text = "abcde";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        // Segment "abc": byte range [0, 3)
        let g_start = grapheme_index_at_byte(text, 0); // byte 0 → grapheme 0
        let g_end = grapheme_index_at_byte(text, 3); // byte 3 → grapheme 3
        let seg = &map.as_slice()[g_start..=g_end];
        // "abc" has 3 graphemes, map should have 3 + 1 = 4 entries
        assert_eq!(seg.len(), 4, "segment slice should have grapheme_count + 1 entries");
        assert_eq!(seg[0], 0); // 'a'
        assert_eq!(seg[1], 1); // 'b'
        assert_eq!(seg[2], 2); // 'c'
        assert_eq!(seg[3], 3); // sentinel = byte of 'd' (start of next segment)
    }

    /// The sentinel of one segment equals the first entry of the next segment,
    /// providing forward affinity at wrap boundaries.
    #[test]
    fn sentinel_equals_next_segment_first_byte() {
        let text = "abcdef";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        // Split at byte 3 → seg0 = "abc" [0,3), seg1 = "def" [3,6)
        let g0_end = grapheme_index_at_byte(text, 3); // byte 3 → grapheme 3
        let seg0_last = map.as_slice()[g0_end]; // sentinel of seg0
        // Seg1 starts at the same grapheme index as seg0's sentinel position.
        let seg1_start = grapheme_index_at_byte(text, 3); // byte 3 → grapheme 3
        let seg1_first = map.source_byte_at(seg1_start).unwrap();
        assert_eq!(seg1_first, 3); // first grapheme of "def" is at byte 3
        // The sentinel of seg0 equals the first entry of seg1.
        assert_eq!(seg0_last, seg1_first);
    }

    /// `grapheme_index_at_byte()` at segment boundaries returns the correct
    /// grapheme index: a byte at the boundary belongs to the next grapheme.
    #[test]
    fn grapheme_index_at_byte_at_segment_boundary() {
        let text = "abcde";

        // byte 0 → grapheme 0 ('a')
        assert_eq!(grapheme_index_at_byte(text, 0), 0);
        // byte 1 → grapheme 1 ('b')
        assert_eq!(grapheme_index_at_byte(text, 1), 1);
        // byte 2 → grapheme 2 ('c')
        assert_eq!(grapheme_index_at_byte(text, 2), 2);
        // byte 3 → grapheme 3 ('d') — this is the boundary
        assert_eq!(grapheme_index_at_byte(text, 3), 3);
        // byte 4 → grapheme 4 ('e')
        assert_eq!(grapheme_index_at_byte(text, 4), 4);
        // byte 5 → grapheme 5 (one-past-end sentinel)
        assert_eq!(grapheme_index_at_byte(text, 5), 5);
    }

    /// Grapehme index at byte for multi-byte CJK at wrap boundary.
    /// If text "hello你好world" wraps between "你好" and "world",
    /// the byte at the boundary maps to the correct grapheme.
    #[test]
    fn grapheme_index_at_byte_cjk_wrap_boundary() {
        let text = "hello你好world";
        // byte positions (你=U+4F60=3bytes, 好=U+597D=3bytes):
        // h(0) e(1) l(2) l(3) o(4) 你(5..7) 好(8..10) w(11) o(12) r(13) l(14) d(15)
        // grapheme positions: 0:h 1:e 2:l 3:l 4:o 5:你 6:好 7:w 8:o 9:r 10:l 11:d

        // Boundary at byte 11 (start of 'w', between "好" and "world")
        assert_eq!(grapheme_index_at_byte(text, 11), 7); // 'w' is grapheme 7

        // Inside 你 (bytes 5-7): all snap to grapheme 5
        assert_eq!(grapheme_index_at_byte(text, 5), 5); // start of 你
        assert_eq!(grapheme_index_at_byte(text, 6), 5); // mid-cluster → snaps to start
        assert_eq!(grapheme_index_at_byte(text, 7), 5); // last byte of 你 → snaps to start

        // Byte 8: start of 好 → grapheme 6
        assert_eq!(grapheme_index_at_byte(text, 8), 6);

        // Inside 好 (bytes 9-10): snap to grapheme 6
        assert_eq!(grapheme_index_at_byte(text, 9), 6); // mid-cluster → snaps to start
        assert_eq!(grapheme_index_at_byte(text, 10), 6); // last byte of 好 → snaps to start
    }

    /// Segment source map with CJK: each CJK char is its own grapheme,
    /// so the segment [你, 好] has 2 graphemes + 1 sentinel = 3 entries.
    #[test]
    fn segment_source_map_cjk_has_correct_sentinel() {
        let text = "hello你好world";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        // Segment "你好": byte range [5, 11)
        let g_start = grapheme_index_at_byte(text, 5); // byte 5 → grapheme 5 (你)
        let g_end = grapheme_index_at_byte(text, 11); // byte 11 → grapheme 7 (w)
        let seg = &map.as_slice()[g_start..=g_end];

        // 2 graphemes (你, 好) + 1 sentinel = 3 entries
        assert_eq!(seg.len(), 3);
        assert_eq!(seg[0], 5); // 你 at source byte 5
        assert_eq!(seg[1], 8); // 好 at source byte 8
        assert_eq!(seg[2], 11); // sentinel = 'w' at source byte 11
    }

    /// When the entire text wraps and a segment ends at text.len(), the
    /// sentinel is the total byte length (from the full source map's sentinel).
    #[test]
    fn segment_sentinel_at_end_of_full_text() {
        let text = "abc";
        let source_by_char = char_sentinel_map(text);
        let map = build_visual_grapheme_map(text, &source_by_char);

        // Segment covering the whole text: byte range [0, 3)
        let g_start = grapheme_index_at_byte(text, 0);
        let g_end = grapheme_index_at_byte(text, 3);
        let seg = &map.as_slice()[g_start..=g_end];

        // 3 graphemes + 1 sentinel = 4 entries
        assert_eq!(seg.len(), 4);
        assert_eq!(seg[0], 0);
        assert_eq!(seg[1], 1);
        assert_eq!(seg[2], 2);
        assert_eq!(seg[3], 3); // sentinel = text.len()
    }

    /// `grapheme_index_at_byte` for a byte beyond text length returns
    /// the grapheme count (one-past-end sentinel position).
    #[test]
    fn grapheme_index_at_byte_past_end_cjk() {
        let text = "你好";
        // 2 graphemes, byte positions: 你(0..3) 好(3..6)
        assert_eq!(grapheme_index_at_byte(text, 0), 0);
        assert_eq!(grapheme_index_at_byte(text, 3), 1);
        assert_eq!(grapheme_index_at_byte(text, 6), 2); // past end → grapheme count
        assert_eq!(grapheme_index_at_byte(text, 100), 2); // way past end → grapheme count
    }
}
