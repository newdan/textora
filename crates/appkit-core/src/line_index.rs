//! Line index: byte offsets and lengths for each document line.
//!
//! Used by DocumentView for efficient line-level access during rendering,
//! cursor movement, and incremental edits.

use core::buffer::text_buffer::TextBuffer;
use core::types::{UniCharOffset, UnicharLineLookup};
use core::unicode::{
    ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done, ucd_grapheme_cluster_lookup,
};

/// Cached line index over a TextBuffer.
///
/// `offsets[i]` is the byte offset of line `i` in the buffer.
/// `lengths[i]` is the byte length of line `i` (excluding trailing line ending).
/// `unichar_offsets[i]` is the absolute unichar offset (excluding newlines) at the start of line `i`.
pub struct LineIndex {
    pub offsets: Vec<usize>,
    pub lengths: Vec<usize>,
    pub unichar_offsets: Vec<usize>,
}

impl Default for LineIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl LineIndex {
    /// Create an empty line index.
    pub fn new() -> Self {
        Self { offsets: vec![0], lengths: vec![0], unichar_offsets: vec![0] }
    }

    /// Full rebuild from TextBuffer content.
    ///
    /// Iterates through TextBuffer using read_forward() to handle chunk boundaries.
    pub fn rebuild_from(tb: &TextBuffer) -> Self {
        let total = tb.text_length();
        if total == 0 {
            return Self::new();
        }

        let mut offsets = Vec::new();
        let mut lengths = Vec::new();
        let mut pos = 0;

        let mut unichar_offsets = Vec::new();
        let mut cumulative_unichar = 0;
        let mut unichar_start = 0;

        let mut grapheme_state: u32 = 0;
        let mut first_char = true;
        let mut prev_props: usize = 0;
        let mut last_char = '\0';
        let mut line_start: usize = 0;

        while pos < total {
            let chunk = tb.read_forward(pos);
            if chunk.is_empty() {
                break;
            }

            // Assume valid UTF-8, but handle lossy safely.
            let text = String::from_utf8_lossy(chunk);
            for ch in text.chars() {
                let bytes_len = ch.len_utf8();

                // --- Line counting logic ---
                if ch == '\n' {
                    if last_char == '\r' {
                        line_start = pos + bytes_len;
                        unichar_start = cumulative_unichar;
                    } else {
                        offsets.push(line_start);
                        lengths.push(pos - line_start);
                        unichar_offsets.push(unichar_start);
                        line_start = pos + bytes_len;
                        unichar_start = cumulative_unichar;
                    }
                } else if ch == '\r' {
                    offsets.push(line_start);
                    lengths.push(pos - line_start);
                    unichar_offsets.push(unichar_start);
                    line_start = pos + bytes_len;
                    unichar_start = cumulative_unichar;
                } else {
                    // --- Grapheme counting logic (only for non-newline chars) ---
                    let props = ucd_grapheme_cluster_lookup(ch);
                    if first_char {
                        cumulative_unichar += 1;
                        first_char = false;
                    } else {
                        grapheme_state =
                            ucd_grapheme_cluster_joins(grapheme_state, prev_props, props);
                        if ucd_grapheme_cluster_joins_done(grapheme_state) {
                            cumulative_unichar += 1;
                            grapheme_state = 0;
                        }
                    }
                    prev_props = props;
                }

                last_char = ch;
                pos += bytes_len;
            }
        }

        // Handle last line (may not end with newline)
        if line_start < total {
            offsets.push(line_start);
            lengths.push(total - line_start);
            unichar_offsets.push(unichar_start);
        } else if line_start == total && total > 0 {
            // Document ends with a newline, add a final empty line
            offsets.push(line_start);
            lengths.push(0);
            unichar_offsets.push(unichar_start);
        }

        Self { offsets, lengths, unichar_offsets }
    }

    // TODO(dry): extract shared grapheme-counting loop used by both rebuild_from and rescan_from.
    /// Rescan line offsets starting from byte offset `start`.
    /// Replaces all line entries from the line containing `start` onward.
    pub fn rescan_from(&mut self, tb: &TextBuffer, start: usize) {
        let total = tb.text_length();
        if total == 0 {
            self.offsets.clear();
            self.lengths.clear();
            self.unichar_offsets.clear();
            self.offsets.push(0);
            self.lengths.push(0);
            self.unichar_offsets.push(0);
            return;
        }

        // Find the line index that starts at or before `start`
        let line_idx = match self.offsets.binary_search(&start) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };

        let mut cumulative_unichar =
            if line_idx < self.unichar_offsets.len() { self.unichar_offsets[line_idx] } else { 0 };

        let resume_pos = if line_idx < self.offsets.len() { self.offsets[line_idx] } else { total };

        // Truncate from that line onward
        self.offsets.truncate(line_idx);
        self.lengths.truncate(line_idx);
        self.unichar_offsets.truncate(line_idx);

        // Rescan from `resume_pos` to end of buffer
        let mut pos = resume_pos;
        let mut grapheme_state: u32 = 0;
        let mut first_char = true;
        let mut prev_props: usize = 0;
        let mut last_char = '\0';
        let mut line_start: usize = pos;
        let mut unichar_start = cumulative_unichar;

        while pos < total {
            let chunk = tb.read_forward(pos);
            if chunk.is_empty() {
                break;
            }

            // Assume valid UTF-8, but handle lossy safely.
            let text = String::from_utf8_lossy(chunk);
            for ch in text.chars() {
                let bytes_len = ch.len_utf8();

                // --- Line counting logic ---
                if ch == '\n' {
                    if last_char == '\r' {
                        line_start = pos + bytes_len;
                        unichar_start = cumulative_unichar;
                    } else {
                        self.offsets.push(line_start);
                        self.lengths.push(pos - line_start);
                        self.unichar_offsets.push(unichar_start);
                        line_start = pos + bytes_len;
                        unichar_start = cumulative_unichar;
                    }
                } else if ch == '\r' {
                    self.offsets.push(line_start);
                    self.lengths.push(pos - line_start);
                    self.unichar_offsets.push(unichar_start);
                    line_start = pos + bytes_len;
                    unichar_start = cumulative_unichar;
                } else {
                    // --- Grapheme counting logic (only for non-newline chars) ---
                    let props = ucd_grapheme_cluster_lookup(ch);
                    if first_char {
                        cumulative_unichar += 1;
                        first_char = false;
                    } else {
                        grapheme_state =
                            ucd_grapheme_cluster_joins(grapheme_state, prev_props, props);
                        if ucd_grapheme_cluster_joins_done(grapheme_state) {
                            cumulative_unichar += 1;
                            grapheme_state = 0;
                        }
                    }
                    prev_props = props;
                }

                last_char = ch;
                pos += bytes_len;
            }
        }
        // Handle last line
        if line_start < total {
            self.offsets.push(line_start);
            self.lengths.push(total - line_start);
            self.unichar_offsets.push(cumulative_unichar);
        } else if line_start == total && total > 0 {
            let last_byte_chunk = tb.read_forward(total - 1);
            if !last_byte_chunk.is_empty()
                && (last_byte_chunk[0] == b'\n' || last_byte_chunk[0] == b'\r')
            {
                self.offsets.push(total);
                self.lengths.push(0);
                self.unichar_offsets.push(cumulative_unichar);
            }
        }
    }

    /// Number of document lines.
    pub fn line_count(&self) -> usize {
        self.offsets.len()
    }

    /// Returns the cumulative grapheme offset at the start of the given line.
    ///
    /// `unichar_of_line(0)` is always `UniCharOffset::ZERO`.
    /// For line `n`, it equals the unichar offset at the start of the line.
    pub fn unichar_of_line(&self, line: usize) -> UniCharOffset {
        let line = line.min(self.unichar_offsets.len().saturating_sub(1));
        UniCharOffset(self.unichar_offsets[line])
    }

    /// Binary search to find which line a given unichar offset falls on.
    ///
    /// Returns `(line_number, line_local_unichar)` where `line_local_unichar`
    /// is the offset within that line.
    pub fn line_at_unichar(&self, offset: UniCharOffset) -> (usize, usize) {
        let target = offset.to_usize();
        // Use partition_point to deterministically find the FIRST line starting
        // at `target`.  std's binary_search may return any matching index when
        // duplicates exist (consecutive empty lines share the same unichar
        // offset), while partition_point always returns the first occurrence.
        let idx = self.unichar_offsets.partition_point(|&x| x < target);
        if idx < self.unichar_offsets.len() && self.unichar_offsets[idx] == target {
            (idx, 0)
        } else {
            let line = idx.saturating_sub(1);
            (line, target - self.unichar_offsets[line])
        }
    }
}

impl UnicharLineLookup for LineIndex {
    fn line_at_unichar(&self, offset: UniCharOffset) -> (usize, usize) {
        self.line_at_unichar(offset)
    }
}

/// Find the byte offset of the `target`-th grapheme cluster in `[start, end)`.
/// Returns `end` if `target` is beyond the last grapheme.
pub fn grapheme_to_byte(tb: &TextBuffer, start: usize, end: usize, target: usize) -> usize {
    if start >= end {
        return end;
    }
    let mut grapheme_index: usize = 0;
    let mut byte_offset: usize = 0;
    let mut state: u32 = 0;
    let mut first_char = true;
    let mut prev_props: usize = 0;

    let mut pos = start;
    while pos < end {
        let chunk = tb.read_forward(pos);
        if chunk.is_empty() {
            break;
        }
        let take = (end - pos).min(chunk.len());
        let slice = &chunk[..take];
        let s = String::from_utf8_lossy(slice);

        for ch in s.chars() {
            let char_start = byte_offset;
            let props = ucd_grapheme_cluster_lookup(ch);

            if first_char {
                if grapheme_index == target {
                    return start + char_start;
                }
                prev_props = props;
                first_char = false;
            } else {
                state = ucd_grapheme_cluster_joins(state, prev_props, props);
                if ucd_grapheme_cluster_joins_done(state) {
                    grapheme_index += 1;
                    state = 0;
                    if grapheme_index == target {
                        return start + char_start;
                    }
                }
                prev_props = props;
            }
            byte_offset += ch.len_utf8();
        }
        pos += take;
    }
    end
}

/// Count grapheme clusters whose start byte is strictly before `target_byte` in `[start, end)`.
pub fn count_graphemes_before(
    tb: &TextBuffer,
    start: usize,
    end: usize,
    target_byte: usize,
) -> usize {
    if start >= end {
        return 0;
    }
    let mut count: usize = 0;
    let mut byte_offset: usize = 0;
    let mut state: u32 = 0;
    let mut first_char = true;
    let mut prev_props: usize = 0;

    let mut pos = start;
    while pos < end {
        let chunk = tb.read_forward(pos);
        if chunk.is_empty() {
            break;
        }
        let take = (end - pos).min(chunk.len());
        let slice = &chunk[..take];
        let s = String::from_utf8_lossy(slice);

        for ch in s.chars() {
            let char_byte_pos = start + byte_offset;
            if char_byte_pos >= target_byte {
                return count;
            }
            let props = ucd_grapheme_cluster_lookup(ch);

            if first_char {
                count += 1;
                prev_props = props;
                first_char = false;
            } else {
                state = ucd_grapheme_cluster_joins(state, prev_props, props);
                if ucd_grapheme_cluster_joins_done(state) {
                    count += 1;
                    state = 0;
                }
                prev_props = props;
            }
            byte_offset += ch.len_utf8();
        }
        pos += take;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a TextBuffer from raw bytes.
    fn make_tb(content: &[u8]) -> TextBuffer {
        let mut tb = TextBuffer::new(false).expect("TextBuffer creation failed");
        if !content.is_empty() {
            tb.write_raw(content);
        }
        tb
    }

    #[test]
    fn unichar_of_line_ascii() {
        let tb = make_tb(b"hello\nworld\n");
        let li = LineIndex::rebuild_from(&tb);
        // "hello" = 5 graphemes, "world" = 5 graphemes, "" = 0
        assert_eq!(li.unichar_of_line(0), UniCharOffset(0));
        assert_eq!(li.unichar_of_line(1), UniCharOffset(5));
        assert_eq!(li.unichar_of_line(2), UniCharOffset(10));
    }

    #[test]
    fn unichar_of_line_multibyte_utf8() {
        // "你好" = 2 CJK codepoints = 2 graphemes (each 3 bytes)
        // "🌍🌎" = 2 emoji codepoints = 2 graphemes (each 4 bytes)
        let content = "你好\n🌍🌎\n".as_bytes();
        let tb = make_tb(content);
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_of_line(0), UniCharOffset(0));
        assert_eq!(li.unichar_of_line(1), UniCharOffset(2));
        assert_eq!(li.unichar_of_line(2), UniCharOffset(4));
    }

    #[test]
    fn line_at_unichar_roundtrip_ascii() {
        let tb = make_tb(b"abc\ndef\nghi\n");
        let li = LineIndex::rebuild_from(&tb);
        // Line 0 starts at unichar 0, line 1 at 3, line 2 at 6
        for line in 0..li.line_count() {
            let offset = li.unichar_of_line(line);
            let (found_line, local) = li.line_at_unichar(offset);
            assert_eq!(found_line, line, "roundtrip failed for line {line}");
            assert_eq!(local, 0, "local offset should be 0 at line start");
        }
    }

    #[test]
    fn line_at_unichar_roundtrip_multibyte() {
        let content = "你好\n世界\n".as_bytes();
        let tb = make_tb(content);
        let li = LineIndex::rebuild_from(&tb);
        for line in 0..li.line_count() {
            let offset = li.unichar_of_line(line);
            let (found_line, local) = li.line_at_unichar(offset);
            assert_eq!(found_line, line, "roundtrip failed for line {line}");
            assert_eq!(local, 0, "local offset should be 0 at line start");
        }
    }

    #[test]
    fn line_at_unichar_midline() {
        let tb = make_tb(b"abcde\nfghij\n");
        let li = LineIndex::rebuild_from(&tb);
        // Offset 7 should be line 1, local offset 2 (within "fghij")
        let (line, local) = li.line_at_unichar(UniCharOffset(7));
        assert_eq!(line, 1);
        assert_eq!(local, 2);
    }

    #[test]
    fn empty_document() {
        let tb = make_tb(b"");
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.line_count(), 1);
        assert_eq!(li.unichar_offsets, vec![0]);
        assert_eq!(li.unichar_of_line(0), UniCharOffset(0));
        let (line, local) = li.line_at_unichar(UniCharOffset(0));
        assert_eq!(line, 0);
        assert_eq!(local, 0);
    }

    #[test]
    fn single_line_no_newline() {
        let tb = make_tb(b"hello");
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.line_count(), 1);
        assert_eq!(li.unichar_offsets, vec![0]);
        assert_eq!(li.unichar_of_line(0), UniCharOffset(0));
    }

    #[test]
    fn grapheme_counts_after_rescan() {
        let tb = make_tb(b"abc\ndef\n");
        let mut li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_offsets, vec![0, 3, 6]);
        // rescan_from should preserve consistency
        li.rescan_from(&tb, 0);
        assert_eq!(li.unichar_offsets, vec![0, 3, 6]);
    }

    #[test]
    fn composed_accent_is_one_grapheme() {
        // "é" = U+0065 (e) + U+0301 (combining acute accent) = 1 grapheme, 2 codepoints
        // Test with NFD (decomposed) form
        let nfd = "caf\u{0065}\u{0301}\n";
        let tb = make_tb(nfd.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        // "caf" = 3 ASCII graphemes, then e+combining = 1 grapheme → line has 4
        assert_eq!(
            li.unichar_of_line(1).to_usize(),
            4,
            "e + combining accent should be 1 grapheme"
        );
    }

    #[test]
    fn emoji_skin_tone_is_one_grapheme() {
        // "👍🏽" = thumbs up + skin tone modifier = 1 grapheme
        let content = "👍🏽\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_of_line(1).to_usize(), 1, "emoji + skin tone should be 1 grapheme");
    }

    #[test]
    fn flag_emoji_count() {
        // Regional indicators: the UAX #29 state machine decides the count.
        // Just verify it's consistent and doesn't panic.
        let content = "🇨🇳\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        let unichar_next = li.unichar_of_line(1).to_usize();
        // UAX #29 GB12/13: two RI → 1 flag grapheme
        assert_eq!(unichar_next, 1, "two regional indicators should form 1 flag grapheme");
    }

    #[test]
    fn grapheme_to_byte_ascii() {
        let tb = make_tb(b"abcde");
        // 'b' is at grapheme index 1, byte offset 1
        assert_eq!(grapheme_to_byte(&tb, 0, 5, 1), 1);
        // 'e' at grapheme index 4
        assert_eq!(grapheme_to_byte(&tb, 0, 5, 4), 4);
        // Past end
        assert_eq!(grapheme_to_byte(&tb, 0, 5, 5), 5);
    }

    #[test]
    fn grapheme_to_byte_composed() {
        // "aé" where é = e + combining (NFD): a(1 byte) + e(1 byte) + combining(2 bytes) = 4 bytes
        let content = "a\u{0065}\u{0301}";
        let tb = make_tb(content.as_bytes());
        // grapheme 0 = 'a' at byte 0
        assert_eq!(grapheme_to_byte(&tb, 0, 4, 0), 0);
        // grapheme 1 = 'é' at byte 1
        assert_eq!(grapheme_to_byte(&tb, 0, 4, 1), 1);
        // past end
        assert_eq!(grapheme_to_byte(&tb, 0, 4, 2), 4);
    }

    #[test]
    fn count_graphemes_before_ascii() {
        let tb = make_tb(b"abcde");
        // graphemes before byte 0 → 0
        assert_eq!(count_graphemes_before(&tb, 0, 5, 0), 0);
        // graphemes before byte 3 → 3
        assert_eq!(count_graphemes_before(&tb, 0, 5, 3), 3);
        // graphemes before byte 5 → 5
        assert_eq!(count_graphemes_before(&tb, 0, 5, 5), 5);
    }

    #[test]
    fn count_graphemes_before_composed() {
        // "aé" where é = e + combining (NFD): 4 bytes, 2 graphemes
        let content = "a\u{0065}\u{0301}";
        let tb = make_tb(content.as_bytes());
        // graphemes before byte 0 → 0
        assert_eq!(count_graphemes_before(&tb, 0, 4, 0), 0);
        // graphemes before byte 1 → 1 ('a')
        assert_eq!(count_graphemes_before(&tb, 0, 4, 1), 1);
        // graphemes before byte 2 (in middle of combining accent in é) → 2
        assert_eq!(count_graphemes_before(&tb, 0, 4, 2), 2);
        // graphemes before byte 4 (end) → 2
        assert_eq!(count_graphemes_before(&tb, 0, 4, 4), 2);
    }

    // ── count_graphemes_from_bytes: additional complex grapheme clusters ──

    #[test]
    fn zwj_family_emoji_is_one_grapheme() {
        // "👨\u{200D}👩\u{200D}👧" (family = man ZWJ woman ZWJ girl) = 1 grapheme cluster
        let content = "👨\u{200D}👩\u{200D}👧\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_of_line(1).to_usize(), 1, "ZWJ family emoji should be 1 grapheme");
    }

    #[test]
    fn combining_mark_chain_is_one_grapheme() {
        // "e\u{0301}\u{0300}" = e + acute + grave = 1 grapheme cluster
        let content = "e\u{0301}\u{0300}\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(
            li.unichar_of_line(1).to_usize(),
            1,
            "e + two combining marks should be 1 grapheme"
        );
    }

    #[test]
    fn regional_indicator_us_flag() {
        // U+1F1FA U+1F1F8 = US flag = 1 grapheme per UAX #29 GB12/13
        let content = "🇺🇸\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_of_line(1).to_usize(), 1, "US flag should be 1 grapheme");
    }

    #[test]
    fn skin_tone_modifier_emoji() {
        // "👍🏽" = thumbs up + medium skin tone modifier = 1 grapheme
        let content = "👍🏽\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(
            li.unichar_of_line(1).to_usize(),
            1,
            "emoji + skin tone modifier should be 1 grapheme"
        );
    }

    #[test]
    fn mixed_line_grapheme_count() {
        // "Aé汉🇨🇳" = 4 grapheme clusters:
        //   A (1), é = e+acute (1), 汉 (1), 🇨🇳 (1)
        let content = "Ae\u{0301}汉🇨🇳\n";
        let tb = make_tb(content.as_bytes());
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_of_line(1).to_usize(), 4, "A+é+汉+🇨🇳 = 4 graphemes");
    }

    #[test]
    fn line_at_unichar_single_empty_line() {
        // "aaa\n\nbbb\n" → unichar_offsets = [0, 3, 3, 6]
        // The empty line (line 1) shares unichar 3 with "bbb" (line 2).
        // line_at_unichar must return the first match, not the text line.
        let tb = make_tb(b"aaa\n\nbbb\n");
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_offsets, vec![0, 3, 3, 6]);
        let (line, local) = li.line_at_unichar(UniCharOffset(3));
        assert_eq!(line, 1, "should return the empty line (first match), not bbb line");
        assert_eq!(local, 0);
    }

    #[test]
    fn line_at_unichar_consecutive_empty_lines() {
        // "aaa\n\n\n\nbbb\n" → unichar_offsets = [0,3,3,3,3,6]
        let tb = make_tb(b"aaa\n\n\n\nbbb\n");
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_offsets, vec![0, 3, 3, 3, 3, 6]);
        let (line, local) = li.line_at_unichar(UniCharOffset(3));
        assert_eq!(line, 1, "should return the first empty line");
        assert_eq!(local, 0);
    }

    #[test]
    fn line_at_unichar_midline_after_empty_lines() {
        // "a\n\nbc\n" → offsets=[0,2,3,6], unichar_offsets=[0,1,1,3]
        // Line 0: unichar 0 ("a"), line 1: unichar 1 (empty), line 2: unichar 1 ("bc")
        // Grapheme 0='a', 1=... (empty line shares unichar 1), 2='b', 3='c'
        // Unichar 2 should land in line 2 (the "bc" line), local grapheme 1 ('b')
        let tb = make_tb(b"a\n\nbc\n");
        let li = LineIndex::rebuild_from(&tb);
        assert_eq!(li.unichar_offsets, vec![0, 1, 1, 3]);
        let (line, local) = li.line_at_unichar(UniCharOffset(2));
        assert_eq!(line, 2, "unichar 2 ('b') should be line 2, not empty line 1");
        assert_eq!(local, 1, "'b' is at local offset 1 within 'bc'");
    }
}
