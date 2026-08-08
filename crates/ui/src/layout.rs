use crate::render_geom::AdvanceCacheEntry;
use shaping;
use unicode_categories::UnicodeCategories;

/// Build advance cache entries for a single doc line's visible visual lines.
#[allow(
    clippy::too_many_arguments,
    reason = "advance cache construction keeps shaped text and viewport coordinates explicit"
)]
pub fn build_advance_cache_entries(
    visual_lines: &[(usize, usize, f32)],
    skip_visual: usize,
    shaped: &shaping::ShapedRun,
    line_bytes: &[u8],
    char_width: f32,
    doc_line_idx: usize,
    cluster_pool: &mut Vec<Vec<(usize, f32, u32)>>,
    left_margin: f32,
) -> Vec<AdvanceCacheEntry> {
    if shaped.clusters.is_empty() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    let mut vl_grapheme_start: usize = visual_lines[..skip_visual]
        .iter()
        .map(|(vl_start, vl_end, _)| vl_end.saturating_sub(*vl_start))
        .sum();
    for &(vl_start, vl_end, _) in &visual_lines[skip_visual..] {
        let mut cluster_advances = cluster_pool.pop().unwrap_or_default();
        cluster_advances.clear();
        let mut x = left_margin;
        let vl_byte_start = shaped.clusters[vl_start].byte_range.start;
        let mut grapheme_idx: u32 = 0;
        for c in &shaped.clusters[vl_start..vl_end] {
            let is_ws = is_whitespace_cluster(&line_bytes[c.byte_range.clone()]);
            x += if is_ws {
                ws_cluster_advance(&line_bytes[c.byte_range.clone()], char_width)
            } else {
                c.advance.max(1.0)
            };
            // vl-local byte offset: subtract vl_byte_start so byte_to_x's prev_end=0 works for all VLs.
            // saturating_sub guards against unexpected shaper output.
            cluster_advances.push((
                c.byte_range.end.saturating_sub(vl_byte_start),
                x,
                grapheme_idx,
            ));
            grapheme_idx += 1;
        }
        entries.push(AdvanceCacheEntry {
            doc_line: doc_line_idx,
            vl_byte_start,
            vl_grapheme_start,
            clusters: cluster_advances,
        });
        vl_grapheme_start += grapheme_idx as usize;
    }
    entries
}
/// Whether all chars in `bytes` are whitespace (ASCII + Unicode).
/// Recognizes ASCII whitespace, NBSP (U+00A0), and ideographic space (U+3000).
/// Returns false for invalid UTF-8 bytes (falls through to ASCII check).
pub fn is_whitespace_cluster(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.chars().all(|c| c.is_whitespace()),
        Err(_) => bytes.iter().all(|b| b.is_ascii_whitespace()),
    }
}

/// Default tab width in columns (matches Settings::tab_width default).
pub const DEFAULT_TAB_WIDTH: usize = 4;

/// Compute effective advance for a whitespace cluster.
/// Tab clusters use `tab_width * char_width`; ideographic space (U+3000)
/// uses `2 * char_width`; other whitespace uses `char_width`.
pub fn ws_cluster_advance(bytes: &[u8], char_width: f32) -> f32 {
    if bytes == b"\t" {
        return char_width * DEFAULT_TAB_WIDTH as f32;
    }
    // Ideographic space (U+3000) is fullwidth: 2 columns
    if bytes == b"\xe3\x80\x80" {
        return char_width * 2.0;
    }
    char_width
}

/// Check if a UTF-8 character is a CJK ideograph, Kana, or Hangul syllable (not punctuation/symbols).

/// Returns `(is_whitespace, advance)` for a glyph cluster.
/// Combines `is_whitespace_cluster` + `ws_cluster_advance` / fallback advance.
pub fn cluster_advance(bytes: &[u8], fallback_advance: f32, char_width: f32) -> (bool, f32) {
    let ws = is_whitespace_cluster(bytes);
    let adv = if ws { ws_cluster_advance(bytes, char_width) } else { fallback_advance.max(1.0) };
    (ws, adv)
}
pub fn is_cjk_char(ch: char) -> bool {
    let cp = ch as u32;
    matches!(cp,
        0x4E00..=0x9FFF    |  // CJK Unified Ideographs
        0x3400..=0x4DBF    |  // CJK Unified Ideographs Extension A
        0x20000..=0x2A6DF  |  // CJK Unified Ideographs Extension B
        0xF900..=0xFAFF    |  // CJK Compatibility Ideographs
        0x2F800..=0x2FA1F  |  // CJK Compatibility Ideographs Supplement
        0x3040..=0x309F    |  // Hiragana
        0x30A0..=0x30FF    |  // Katakana
        0x31F0..=0x31FF    |  // Katakana Phonetic Extensions
        0xFF66..=0xFF9F    |  // Halfwidth Katakana
        0xAC00..=0xD7AF    |  // Hangul Syllables
        0x1100..=0x11FF    |  // Hangul Jamo
        0x3130..=0x318F    |  // Hangul Compat Jamo
        0xA960..=0xA97F    |  // Hangul Jamo Extended-A
        0xD7B0..=0xD7FF    |  // Hangul Jamo Extended-B
        0x3000..=0x303F    |  // CJK Symbols and Punctuation
        0xFF01..=0xFF5E    |  // Fullwidth Forms
        0xFF5F..=0xFF60    |  // Fullwidth Brackets
        0xFFE0..=0xFFE6       // Fullwidth Signs
    )
}

/// Classify a cluster's bytes for CJK boundary detection.
///
/// Returns `Some(true)` if the cluster contains a CJK ideograph,
/// `Some(false)` if it contains ASCII content (letters, digits, punctuation),
/// `None` for non-ASCII, non-CJK characters (CJK punctuation like ，。：).
pub fn cluster_boundary_class(bytes: &[u8]) -> Option<bool> {
    let s = std::str::from_utf8(bytes).ok()?;
    let mut result: Option<bool> = None;
    for ch in s.chars() {
        if is_cjk_char(ch) {
            return Some(true); // CJK ideograph takes priority
        } else if ch.is_ascii() && !ch.is_ascii_whitespace() {
            result = Some(false); // ASCII content (letters, digits, punctuation)
        }
    }
    result
}

/// Compute word-wrap visual lines for a shaped line.
/// Uses word-boundary-aware wrapping: prefers breaking at spaces (and before CJK
/// characters) over hard-breaking mid-word. Falls back to hard break for very
/// long lines with no word boundaries.
/// Returns Vec<(cluster_start, cluster_end, width)> for each visual line.
/// Return the monospace column width.  Always returns `fallback` so tab
/// stops are consistent across ASCII and CJK lines.  Callers pass
/// `font_size * 0.6` which matches the advance of most monospace fonts
/// (e.g. Fira Mono at 600/1000 upem).
pub fn pick_char_width(
    _clusters: &[shaping::GlyphCluster],
    _line_bytes: &[u8],
    fallback: f32,
) -> f32 {
    fallback
}

/// Check if a character should NOT start a wrapped line.
/// Uses Unicode General Category: Pe (Close), Pf (Final), Po (Other)
/// are excluded; Ps (Open), Pi (Initial), Pd (Dash) are allowed.
pub fn is_punct_char(ch: char) -> bool {
    // Ps (Open) and Pi (Initial quote): explicitly allowed at line start
    if ch.is_punctuation_open() || ch.is_punctuation_initial_quote() {
        return false;
    }
    // Pe (Close), Pf (Final quote), Po (Other): should not start a line
    if ch.is_punctuation_close() || ch.is_punctuation_final_quote() || ch.is_punctuation_other() {
        return true;
    }
    // Pd (Dash), symbols, etc.: allowed at line start
    false
}
pub fn is_punctuation(bytes: &[u8]) -> bool {
    if let Ok(s) = std::str::from_utf8(bytes) {
        !s.is_empty() && s.chars().all(is_punct_char)
    } else {
        false
    }
}

fn is_ordered_list_marker_period(line_bytes: &[u8], period_range: std::ops::Range<usize>) -> bool {
    const MARKER_PERIOD: u8 = b'.';

    if period_range.len() != 1 || line_bytes.get(period_range.start) != Some(&MARKER_PERIOD) {
        return false;
    }

    if let Some(next_byte) = line_bytes.get(period_range.end)
        && !next_byte.is_ascii_whitespace()
    {
        return false;
    }

    let mut digit_seen = false;
    for byte in &line_bytes[..period_range.start] {
        if !digit_seen && byte.is_ascii_whitespace() {
            continue;
        }
        if byte.is_ascii_digit() {
            digit_seen = true;
            continue;
        }
        return false;
    }

    digit_seen
}

/// Check if a cluster is a single ASCII alphanumeric byte (letters/digits).
/// Used to prevent breaking inside long words/numbers.
fn is_ascii_alnum_cluster(bytes: &[u8]) -> bool {
    bytes.len() == 1 && bytes[0].is_ascii_alphanumeric()
}

/// Apply minimum-width padding to punctuation glyph clusters.
/// For clusters whose advance < em_width * min_ratio, expands the advance
/// and centers the glyph by adjusting x_offset.
/// Skips whitespace clusters (controlled separately via ws_cluster_advance).
pub fn apply_punctuation_padding(
    clusters: &mut [shaping::GlyphCluster],
    line_bytes: &[u8],
    em_width: f32,
    min_ratio: f32,
) {
    if min_ratio <= 0.0 {
        return;
    }
    let min_advance = em_width * min_ratio;
    for c in clusters.iter_mut() {
        let bytes = match line_bytes.get(c.byte_range.clone()) {
            Some(b) => b,
            None => continue,
        };
        // Skip whitespace clusters
        if is_whitespace_cluster(bytes) {
            continue;
        }
        if is_ordered_list_marker_period(line_bytes, c.byte_range.clone()) {
            continue;
        }
        // Check if first char is punctuation
        let is_punct = match std::str::from_utf8(bytes) {
            Ok(s) => {
                !s.is_empty()
                    && s.chars().all(|ch| ch.is_ascii_punctuation() || ch.is_punctuation())
            }
            Err(_) => false,
        };
        if !is_punct {
            continue;
        }
        if c.advance < min_advance {
            let extra = min_advance - c.advance;
            c.advance = min_advance;
            c.x_offset += extra / 2.0;
        }
    }
}

pub fn compute_visual_lines(
    clusters: &[shaping::GlyphCluster],
    line_bytes: &[u8],
    char_width: f32,
    viewport_width: f32,
    min_fill_ratio: f32,
) -> Vec<(usize, usize, f32)> {
    if clusters.is_empty() {
        return Vec::new();
    }

    // Pre-compute per-cluster advance, is_ws, and prefix sums for O(1) range queries.
    let n = clusters.len();
    let mut adv = Vec::with_capacity(n);
    let mut ws_arr = Vec::with_capacity(n);
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f32);
    for c in clusters {
        let bytes = line_bytes.get(c.byte_range.clone()).unwrap_or(&[]);
        let ws = is_whitespace_cluster(bytes);
        let a = if ws { ws_cluster_advance(bytes, char_width) } else { c.advance.max(1.0) };
        adv.push(a);
        ws_arr.push(ws);
        prefix.push(prefix.last().unwrap() + a);
    }

    let width_of = |s: usize, e: usize| prefix[e] - prefix[s];

    // Width after stripping trailing whitespace.
    let trimmed_width = |s: usize, e: usize| -> f32 {
        let mut w = width_of(s, e);
        let mut i = e;
        while i > s && ws_arr[i - 1] {
            w -= adv[i - 1];
            i -= 1;
        }
        w
    };

    let mut visual_lines: Vec<(usize, usize, f32)> = Vec::new();
    let mut start = 0usize;
    let mut last_break_after_ws: Option<usize> = None;
    let mut last_break_cjk: Option<usize> = None;
    let mut last_content_cjk: Option<bool> = None;

    let mut ci = 0usize;
    while ci < n {
        // Word-boundary detection
        if !ws_arr[ci] {
            if ci > 0 && ws_arr[ci - 1] {
                // CJK 文本不把前导空格当单词边界，让中文尽量填满行
                let bytes = line_bytes.get(clusters[ci].byte_range.clone());
                let is_cjk = bytes.and_then(cluster_boundary_class) == Some(true);
                if !is_cjk {
                    last_break_after_ws = Some(ci);
                }
            }
            if let Some(b) = line_bytes.get(clusters[ci].byte_range.clone())
                && let Some(this_cjk) = cluster_boundary_class(b)
            {
                if let Some(prev) = last_content_cjk
                    && this_cjk != prev
                {
                    last_break_cjk = Some(ci);
                }
                last_content_cjk = Some(this_cjk);
            }
        }

        // CJK 模式：空格溢出时，尝试捞回后续中文到当前行
        if ws_arr[ci] && last_content_cjk == Some(true) {
            let mut peek = ci;
            while peek < n && ws_arr[peek] {
                peek += 1;
            }
            if peek < n {
                let space_and_next = width_of(start, peek) + adv[peek];
                if space_and_next <= viewport_width {
                    ci = peek; // 跳过空格，继续纳入中文
                    continue;
                }
            }
        }

        let visual_line_x = width_of(start, ci);
        if visual_line_x + adv[ci] > viewport_width && ci > start {
            // Choose break point: pick the widest valid candidate.
            // Hard break always competes; CJK mode prefers filling the line,
            // English mode prefers word boundaries.
            let cand_ws = last_break_after_ws.filter(|&i| i > start);
            let cand_cjk = last_break_cjk.filter(|&i| i > start && i <= ci);
            let in_cjk = last_content_cjk == Some(true);
            let hard_x = width_of(start, ci);
            let mut break_at = ci; // default: hard break
            let mut best_x = hard_x;

            // Hard break inside an ASCII alphanumeric run is undesirable.
            // Only backtrack when no whitespace candidate exists (no natural
            // word boundary to prefer).
            if cand_ws.is_none()
                && let Some(b) = line_bytes.get(clusters[ci].byte_range.clone())
                && is_ascii_alnum_cluster(b)
                && ci > start
            {
                let mut run_start = ci;
                while run_start > start {
                    let prev_bytes = line_bytes.get(clusters[run_start - 1].byte_range.clone());
                    if prev_bytes.map(is_ascii_alnum_cluster).unwrap_or(false) {
                        run_start -= 1;
                    } else {
                        break;
                    }
                }
                if run_start > start {
                    break_at = run_start;
                    best_x = hard_x;
                }
            }
            // Whitespace break candidate — skip if next cluster is lone punctuation
            if let Some(i) = cand_ws {
                let next_is_punct = line_bytes
                    .get(clusters[i].byte_range.clone())
                    .map(is_punctuation)
                    .unwrap_or(false);
                if !next_is_punct {
                    let ws_x = trimmed_width(start, i);
                    // English: prefer word boundary, but only if the line fills
                    // at least 50% of the hard-break width. Avoid excessively
                    // short lines when the next token is very long (e.g. crash
                    // log backtrace lines).
                    let accept =
                        if in_cjk { ws_x >= best_x } else { ws_x >= hard_x * min_fill_ratio };
                    if accept {
                        break_at = i;
                        best_x = ws_x;
                    }
                }
            }
            // CJK boundary candidate — don't break right before a lone punctuation
            if let Some(i) = cand_cjk {
                let is_punct = line_bytes
                    .get(clusters[i].byte_range.clone())
                    .map(is_punctuation)
                    .unwrap_or(false);
                if !is_punct {
                    let cjk_x = width_of(start, i);
                    if cjk_x >= best_x {
                        break_at = i;
                    }
                }
            }
            // If break lands before punctuation, decide: swallow (if it fits)
            // or pull preceding char down (if it would overflow).
            // Loop to handle consecutive punctuation: pull-back may land on
            // another punct character (e.g. "→。) that also needs protection.
            while break_at > start {
                let mut punct_end = break_at;
                while punct_end < n {
                    let b = line_bytes.get(clusters[punct_end].byte_range.clone());
                    if b.map(is_punctuation).unwrap_or(false) {
                        punct_end += 1;
                    } else {
                        break;
                    }
                }
                if punct_end > break_at {
                    let with_punct = trimmed_width(start, punct_end);
                    if with_punct <= viewport_width {
                        // Punctuation fits without overflow — swallow it
                        break_at = punct_end;
                        break;
                    }
                    // Would overflow — pull preceding char down with punct.
                    // Never split an ASCII alphanumeric word: if the character
                    // being pulled is alnum, pull back to the word's start so
                    // the whole word moves together with the punctuation.
                    if break_at > start + 1 {
                        let prev_bytes = line_bytes.get(clusters[break_at - 1].byte_range.clone());
                        if prev_bytes.map(is_ascii_alnum_cluster).unwrap_or(false) {
                            let mut run_start = break_at - 1;
                            while run_start > start {
                                let pb = line_bytes.get(clusters[run_start - 1].byte_range.clone());
                                if pb.map(is_ascii_alnum_cluster).unwrap_or(false) {
                                    run_start -= 1;
                                } else {
                                    break;
                                }
                            }
                            // 词起点在行首：整词也装不下，放弃 pull-back，允许 punct 悬挂
                            if run_start == start {
                                break;
                            }
                            break_at = run_start;
                            break;
                        }
                        break_at -= 1;
                    } else {
                        break;
                    }
                } else {
                    break; // no punct at break_at
                }
            }
            let break_x = trimmed_width(start, break_at); // was: if break_at == ci { visual_line_x } else { trimmed_width
            visual_lines.push((start, break_at, break_x));
            start = break_at;
            // Trim leading whitespace on continuation: avoid space-only lines
            while start < n && ws_arr[start] {
                start += 1;
            }
            // Reset candidates
            last_break_after_ws = None;
            last_break_cjk = None;
            last_content_cjk = None;
            // If trimming advanced start past ci, re-sync ci
            if start > ci {
                ci = start;
            }
            if ci < n
                && !ws_arr[ci]
                && let Some(b) = line_bytes.get(clusters[ci].byte_range.clone())
            {
                last_content_cjk = cluster_boundary_class(b);
            }
            continue; // re-evaluate ci with new start
        }
        ci += 1;
    }
    if start < n {
        visual_lines.push((start, n, width_of(start, n)));
    }
    visual_lines
}

#[cfg(test)]
mod tests {

    use super::*;
    // ── is_punct_char Unicode General Category unit tests ──

    #[test]
    fn is_punct_char_pe_close_brackets() {
        // Pe (Close) should NOT start a line
        assert!(is_punct_char(')'));
        assert!(is_punct_char(']'));
        assert!(is_punct_char('}'));
        assert!(is_punct_char('\u{300D}')); // 」
        assert!(is_punct_char('\u{300F}')); // 』
        assert!(is_punct_char('\u{3011}')); // 】
        assert!(is_punct_char('\u{300B}')); // 》
        assert!(is_punct_char('\u{3009}')); // 〉
        assert!(is_punct_char('\u{FF09}')); // ）
    }

    #[test]
    fn is_punct_char_ps_open_brackets_allowed() {
        // Ps (Open) CAN start a line
        assert!(!is_punct_char('('));
        assert!(!is_punct_char('['));
        assert!(!is_punct_char('{'));
        assert!(!is_punct_char('\u{300C}')); // 「
        assert!(!is_punct_char('\u{300E}')); // 『
        assert!(!is_punct_char('\u{3010}')); // 【
        assert!(!is_punct_char('\u{300A}')); // 《
        assert!(!is_punct_char('\u{3008}')); // 〈
        assert!(!is_punct_char('\u{FF08}')); // （
    }

    #[test]
    fn is_punct_char_pf_final_quotes() {
        // Pf (Final quote) should NOT start a line
        assert!(is_punct_char('\u{201D}')); // "
        assert!(is_punct_char('\u{2019}')); // '
    }

    #[test]
    fn is_punct_char_pi_initial_quotes_allowed() {
        // Pi (Initial quote) CAN start a line
        assert!(!is_punct_char('\u{201C}')); // "
        assert!(!is_punct_char('\u{2018}')); // '
    }

    #[test]
    fn is_punct_char_po_other() {
        // Po (Other) should NOT start a line
        assert!(is_punct_char('\u{3001}')); // 、
        assert!(is_punct_char('\u{3002}')); // 。
        assert!(is_punct_char('\u{FF0C}')); // ，
        assert!(is_punct_char('\u{FF1A}')); // ：
        assert!(is_punct_char('\u{FF1B}')); // ；
        assert!(is_punct_char('\u{FF01}')); // ！
        assert!(is_punct_char('\u{FF1F}')); // ？
        assert!(is_punct_char('\u{2026}')); // …
        assert!(is_punct_char('\u{00B7}')); // ·
        assert!(is_punct_char('\u{FF05}')); // ％
        assert!(is_punct_char('\u{2030}')); // ‰
        assert!(is_punct_char('\u{2032}')); // ′
        assert!(is_punct_char('\u{2033}')); // ″
    }

    #[test]
    fn is_punct_char_pd_dash_allowed() {
        // Pd (Dash) CAN start a line
        assert!(!is_punct_char('-'));
        assert!(!is_punct_char('\u{2014}')); // —
        assert!(!is_punct_char('\u{2013}')); // –
        assert!(!is_punct_char('\u{FF5E}')); // ～
        assert!(!is_punct_char('\u{301C}')); // 〜
    }

    #[test]
    fn is_punct_char_non_punct_allowed() {
        // Letters, digits, CJK ideographs CAN start a line
        assert!(!is_punct_char('a'));
        assert!(!is_punct_char('Z'));
        assert!(!is_punct_char('0'));
        assert!(!is_punct_char('\u{4F60}')); // 你
        assert!(!is_punct_char('\u{3042}')); // あ (Hiragana)
        assert!(!is_punct_char('\u{AC00}')); // 가 (Hangul)
        assert!(!is_punct_char(' '));
    }

    #[test]
    fn long_unbreakable_token_after_space() {
        // Short prefix + space + very long token: should hard-break inside
        // the token, not at space (which would make first line too short).
        use shaping::GlyphCluster;
        let char_w = 9.0;
        let vp = 200.0;
        let mut clusters = Vec::new();
        let mut bytes = Vec::new();
        // Prefix: "aaaaa     " = 5 letters + 5 spaces = 90px
        for _ in 0..5 {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b'a');
        }
        for _ in 0..5 {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b' ');
        }
        // Long token: 30 x = 270px (exceeds 200px vp)
        for _ in 0..30 {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b'x');
        }
        let lines = compute_visual_lines(&clusters, &bytes, char_w, vp, 0.5);
        let first_w = lines[0].2;
        assert!(
            first_w > vp * 0.6,
            "first line should fill most of viewport, got width={first_w:.0} vs vp={vp:.0}"
        );
        for (i, (_, _, w)) in lines.iter().enumerate() {
            assert!(*w > vp * 0.3 || lines.len() <= 2, "line {i} too short: width={w:.0}");
        }
    }

    #[test]
    fn build_advance_cache_empty_clusters_safe() {
        let shaped = shaping::ShapedRun { clusters: vec![], width: 0.0 };
        let visual_lines = vec![(0usize, 0usize, 0.0f32)];
        let line_bytes: &[u8] = b"";
        let mut pool = Vec::new();
        let entries = build_advance_cache_entries(
            &visual_lines,
            0,
            &shaped,
            line_bytes,
            10.0,
            0,
            &mut pool,
            1.0,
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn ascii_number_no_space_backtrack() {
        // Non-alnum chars followed by long number (no spaces):
        // alnum backtrack should break at the non-alnum→alnum boundary.
        use shaping::GlyphCluster;
        let char_w = 9.0;
        let vp = 200.0;
        let mut clusters = Vec::new();
        let mut bytes = Vec::new();
        // "====" = 4 non-alnum (punctuation) chars = 36px
        for &b in b"====" {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b);
        }
        // 30 digits
        for _ in 0..30 {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b'1');
        }
        let lines = compute_visual_lines(&clusters, &bytes, char_w, vp, 0.5);
        assert!(lines.len() >= 2, "expected multiple visual lines");
        let (_, first_end, _) = lines[0];
        assert!(
            first_end <= 5,
            "first line should break at the ====/number boundary (byte 4), got first_end={first_end}"
        );
    }

    #[test]
    fn ascii_number_long_prefix_respects_space() {
        // Long prefix ending with space before number: whitespace candidate should win.
        use shaping::GlyphCluster;
        let char_w = 9.0;
        let vp = 200.0;
        let mut clusters = Vec::new();
        let mut bytes = Vec::new();
        // ~23 chars of letters + space ≈ 207px (fills >90% of vp)
        for &b in b"aaaaabbbbbcccccdddddeee " {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b);
        }
        for _ in 0..30 {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b'1');
        }
        let lines = compute_visual_lines(&clusters, &bytes, char_w, vp, 0.5);
        let first_w = lines[0].2;
        assert!(
            first_w > vp * 0.5,
            "first line should fill >50% (space boundary), got width={first_w:.0}"
        );
        let (_, first_end, _) = lines[0];
        assert!(
            first_end <= 25,
            "first line should break at space before number, got first_end={first_end}"
        );
    }

    #[test]
    fn ascii_punct_not_alone_at_line_start() {
        // Punctuation should not appear alone at the start of a wrapped line.
        use shaping::GlyphCluster;
        let char_w = 9.0;
        let vp = 100.0;
        let mut clusters = Vec::new();
        let mut bytes = Vec::new();
        for &b in b"hello, world, foo bar baz qux" {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b);
        }
        let lines = compute_visual_lines(&clusters, &bytes, char_w, vp, 0.5);
        for (i, &(start, _, _)) in lines.iter().enumerate() {
            let line_start_byte = clusters[start].byte_range.start;
            if line_start_byte < bytes.len() {
                let first_byte = bytes[line_start_byte];
                assert!(
                    !first_byte.is_ascii_punctuation(),
                    "line {i} starts with punctuation byte {first_byte} at index {line_start_byte}"
                );
            }
        }
    }

    #[test]
    fn ascii_number_short_prefix_no_backtrack() {
        // Very short prefix ("a ") before long number with space:
        // whitespace candidate rejected (prefix < 50%), hard break is correct.
        use shaping::GlyphCluster;
        let char_w = 9.0;
        let vp = 200.0;
        let mut clusters = Vec::new();
        let mut bytes = Vec::new();
        for &b in b"a " {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b);
        }
        for _ in 0..30 {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b'1');
        }
        let lines = compute_visual_lines(&clusters, &bytes, char_w, vp, 0.5);
        let first_w = lines[0].2;
        // With cand_ws = Some(2) but ws_x only 9px (< 50% of hard_x), hard break wins.
        assert!(
            first_w > vp * 0.8,
            "first line should fill most of viewport (hard break), got width={first_w:.0}"
        );
    }

    // ── apply_punctuation_padding tests ─────────────────────────────

    fn make_cluster(
        range: std::ops::Range<usize>,
        advance: f32,
        x_offset: f32,
    ) -> shaping::GlyphCluster {
        shaping::GlyphCluster {
            byte_range: range,
            advance,
            x_offset,
            glyph_id: 0,
            font_id: Default::default(),
            y_offset: 0.0,
        }
    }

    #[test]
    fn punct_padding_comma_colon() {
        // Comma and colon advances should be padded to >= em * 0.5
        let em = 15.0;
        let ratio = 0.5;
        let min_adv = em * ratio; // 7.5
        let line_bytes = b",:";
        // cluster 0: ',' at byte 0..1, advance=3.0
        // cluster 1: ':' at byte 1..2, advance=4.0
        let mut clusters = vec![make_cluster(0..1, 3.0, 0.0), make_cluster(1..2, 4.0, 0.0)];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        assert!(
            clusters[0].advance >= min_adv,
            "comma advance {} < min {}",
            clusters[0].advance,
            min_adv
        );
        assert!(
            clusters[1].advance >= min_adv,
            "colon advance {} < min {}",
            clusters[1].advance,
            min_adv
        );
    }

    #[test]
    fn punct_padding_narrow_letters_untouched() {
        // Narrow ASCII letters should not be modified
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = b"ilt";
        let mut clusters = vec![
            make_cluster(0..1, 4.0, 0.0),
            make_cluster(1..2, 4.0, 0.0),
            make_cluster(2..3, 5.0, 0.0),
        ];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        assert_eq!(clusters[0].advance, 4.0);
        assert_eq!(clusters[1].advance, 4.0);
        assert_eq!(clusters[2].advance, 5.0);
    }

    #[test]
    fn punct_padding_x_offset_centering() {
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = b",";
        let mut clusters = vec![make_cluster(0..1, 3.0, 1.0)];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        let extra = em * ratio - 3.0; // 7.5 - 3.0 = 4.5
        assert!(
            (clusters[0].x_offset - (1.0 + extra / 2.0)).abs() < 0.01,
            "x_offset expected {}, got {}",
            1.0 + extra / 2.0,
            clusters[0].x_offset
        );
    }

    #[test]
    fn punct_padding_ratio_zero_disabled() {
        let em = 15.0;
        let ratio = 0.0;
        let line_bytes = b",";
        let mut clusters = vec![make_cluster(0..1, 3.0, 0.0)];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        assert_eq!(clusters[0].advance, 3.0, "should be unchanged when ratio=0");
    }

    #[test]
    fn punct_padding_whitespace_skipped() {
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = b" ";
        let mut clusters = vec![make_cluster(0..1, 4.0, 0.0)];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        assert_eq!(clusters[0].advance, 4.0, "whitespace should not be padded");
    }

    #[test]
    fn punct_padding_already_wide_untouched() {
        // Punctuation already wider than min_advance should not be modified
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = b"!";
        let mut clusters = vec![make_cluster(0..1, 10.0, 2.0)];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        assert_eq!(clusters[0].advance, 10.0);
        assert_eq!(clusters[0].x_offset, 2.0);
    }

    #[test]
    fn punct_padding_cjk_period() {
        // CJK fullwidth period (U+3002, 0xe3 0x80 0x82) should be padded
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = [0xe3u8, 0x80, 0x82]; // '。'
        let mut clusters = vec![make_cluster(0..3, 3.0, 0.0)];
        apply_punctuation_padding(&mut clusters, &line_bytes, em, ratio);
        assert!(clusters[0].advance >= em * ratio, "CJK period should be padded");
    }

    #[test]
    fn punct_padding_markdown_ordered_list_marker_period_untouched() {
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = b"10. item";
        let mut clusters = vec![
            make_cluster(0..1, 9.0, 0.0),
            make_cluster(1..2, 9.0, 0.0),
            make_cluster(2..3, 3.0, 0.0),
            make_cluster(3..4, 9.0, 0.0),
        ];

        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);

        assert_eq!(clusters[2].advance, 3.0, "ordered-list marker period should not move cursor x");
        assert_eq!(clusters[2].x_offset, 0.0, "ordered-list marker period should not be centered");
    }

    #[test]
    fn punct_padding_high_ratio() {
        // ratio=2.0: punctuation should be padded to 2x em width
        let em = 10.0;
        let ratio = 2.0;
        let min_adv = em * ratio; // 20.0
        let line_bytes = b"!";
        let mut clusters = vec![make_cluster(0..1, 5.0, 0.0)];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        assert!(
            (clusters[0].advance - min_adv).abs() < 0.01,
            "expected advance={}, got {}",
            min_adv,
            clusters[0].advance
        );
        assert!(
            (clusters[0].x_offset - 7.5).abs() < 0.01,
            "expected x_offset=7.5, got {}",
            clusters[0].x_offset
        );
    }

    #[test]
    fn punct_padding_cjk_exclamation() {
        // CJK fullwidth exclamation (U+FF01, 0xEF 0xBC 0x81)
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = [0xEFu8, 0xBC, 0x81];
        let mut clusters = vec![make_cluster(0..3, 3.0, 1.0)];
        apply_punctuation_padding(&mut clusters, &line_bytes, em, ratio);
        let min_adv = em * ratio;
        assert!(clusters[0].advance >= min_adv, "CJK ! should be padded");
        let extra = min_adv - 3.0;
        assert!(
            (clusters[0].x_offset - (1.0 + extra / 2.0)).abs() < 0.01,
            "x_offset should be centered, got {}",
            clusters[0].x_offset
        );
    }

    #[test]
    fn punct_padding_multi_cluster_line() {
        let em = 15.0;
        let ratio = 0.5;
        let line_bytes = b"a, b!";
        let mut clusters = vec![
            make_cluster(0..1, 9.0, 0.0),
            make_cluster(1..2, 3.0, 0.0),
            make_cluster(2..3, 4.0, 0.0),
            make_cluster(3..4, 9.0, 0.0),
            make_cluster(4..5, 4.0, 0.0),
        ];
        apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
        let min_adv = em * ratio;
        assert_eq!(clusters[0].advance, 9.0, "letter 'a' unchanged");
        assert!(clusters[1].advance >= min_adv, "comma should be padded");
        assert_eq!(clusters[2].advance, 4.0, "space unchanged");
        assert_eq!(clusters[3].advance, 9.0, "letter 'b' unchanged");
        assert!(clusters[4].advance >= min_adv, "! should be padded");
    }

    #[test]
    fn visual_lines_cover_all_bytes() {
        // Verify that visual line byte ranges are CONTIGUOUS — no gaps between them.
        // Gaps would cause characters to be missing from the rendered output.
        use shaping::GlyphCluster;
        let char_w = 14.0; // CJK-width chars
        let texts: Vec<&[u8]> = vec![
            b"abc def ghi jkl mno pqr stu vwx yz",
            b"\xe4\xbd\xa0\xe5\xa5\xbd\xe4\xb8\x96\xe7\x95\x8c abc def ghi", // 你好世界 abc def ghi
            b"a  b  c  d  e  f  g  h  i  j",
            b"  leading spaces",
            b"trailing spaces  ",
        ];
        let vps = [50.0, 100.0, 200.0, 400.0];
        for &text_bytes in &texts {
            for &vp in &vps {
                let mut clusters = Vec::new();
                let mut pos = 0;
                // Simple: 1 byte per cluster for ASCII, 3 bytes for CJK
                while pos < text_bytes.len() {
                    let b = text_bytes[pos];
                    let end = if b < 0x80 { pos + 1 } else { (pos + 3).min(text_bytes.len()) };
                    let w = if b < 0x80 { char_w * 0.6 } else { char_w };
                    clusters.push(GlyphCluster {
                        byte_range: pos..end,
                        advance: w,
                        glyph_id: 0,
                        font_id: Default::default(),
                        x_offset: 0.0,
                        y_offset: 0.0,
                    });
                    pos = end;
                }
                let lines = compute_visual_lines(&clusters, text_bytes, char_w * 0.6, vp, 0.5);
                // Verify: byte ranges must cover from 0 to text_bytes.len() with no gaps
                // (allowing for trimmed whitespace between lines)
                let mut prev_end = 0usize;
                for (li, &(start, end, _)) in lines.iter().enumerate() {
                    let byte_start = clusters[start].byte_range.start;
                    let byte_end = clusters[end - 1].byte_range.end;
                    // Gaps are only allowed if the gap is all whitespace
                    if byte_start > prev_end {
                        let gap = &text_bytes[prev_end..byte_start];
                        assert!(
                            gap.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\n'),
                            "non-whitespace gap at line {}: bytes {}..{} = {:?} (text={:?}, vp={})",
                            li,
                            prev_end,
                            byte_start,
                            gap,
                            std::str::from_utf8(text_bytes),
                            vp
                        );
                    }
                    prev_end = byte_end;
                }
                // Final: last line must end at text_bytes.len()
                if !lines.is_empty() {
                    // Allow trailing whitespace to be trimmed
                    let last_end = {
                        let (_, end_idx, _) = lines.last().unwrap();
                        clusters[end_idx - 1].byte_range.end
                    };
                    let trailing = &text_bytes[last_end..];
                    assert!(
                        trailing.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\n'),
                        "non-whitespace trailing bytes: {:?} (text={:?}, vp={})",
                        trailing,
                        std::str::from_utf8(text_bytes),
                        vp
                    );
                }
            }
        }
    }

    #[test]
    fn ascii_word_not_split_by_trailing_fullwidth_close_punct() {
        // 复现：CJK 段落 + 空格 + ASCII 长词 + 紧随的全角右括号 `）`。
        // 当整段刚好溢出 viewport 时，`）` 是 Pe（Close）标点会触发 pull-back，
        // 但绝不能把前面的 ASCII 词从中间拆开（例如 `helper` 被切成 `helpe` + `r）`）。
        use shaping::GlyphCluster;
        let char_w = 10.0;
        // viewport 恰好放下 "保留其他 helper" 但装不下 "）":
        // 4*20 (CJK) + 7*10 (空格+helper) = 150；加上 20 (）) = 170 > 160。
        let vp = 160.0;

        let mut clusters = Vec::new();
        let mut bytes = Vec::new();
        for cjk in ["保", "留", "其", "他"] {
            let s = cjk.as_bytes();
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + s.len(),
                advance: 2.0 * char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.extend_from_slice(s);
        }
        for &b in b" helper" {
            clusters.push(GlyphCluster {
                byte_range: bytes.len()..bytes.len() + 1,
                advance: char_w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            bytes.push(b);
        }
        let paren = "）".as_bytes();
        clusters.push(GlyphCluster {
            byte_range: bytes.len()..bytes.len() + paren.len(),
            advance: 2.0 * char_w,
            glyph_id: 0,
            font_id: Default::default(),
            x_offset: 0.0,
            y_offset: 0.0,
        });
        bytes.extend_from_slice(paren);

        let lines = compute_visual_lines(&clusters, &bytes, char_w, vp, 0.5);

        // 定位 "helper" 在 bytes 中的字节范围，检查每条视觉行的结尾字节
        // 不落在 "helper" 内部（即不能把 helper 拆成两截）。
        let s = std::str::from_utf8(&bytes).unwrap();
        let helper_start = s.find("helper").expect("literal 'helper' present");
        let helper_end = helper_start + "helper".len();
        for (li, &(_, ce, _)) in lines.iter().enumerate() {
            let line_byte_end = clusters[ce - 1].byte_range.end;
            let ends_inside_word = line_byte_end > helper_start && line_byte_end < helper_end;
            assert!(
                !ends_inside_word,
                "line {li} ends inside \"helper\" at byte {line_byte_end}; lines={lines:?}"
            );
        }
    }

    #[test]
    fn test_compute_visual_lines_infinite_loop_regression() {
        // Test that a sequence of punctuation exceeding the viewport width doesn't cause an infinite loop.
        let text = "A.................................................."; // 'A' followed by 50 dots
        let text_bytes = text.as_bytes();

        let mut clusters = Vec::new();
        let char_w = 10.0;
        let mut pos = 0;

        while pos < text_bytes.len() {
            let b = text_bytes[pos];
            let end = if b < 0x80 { pos + 1 } else { (pos + 3).min(text_bytes.len()) };
            let w = char_w;
            clusters.push(shaping::GlyphCluster {
                byte_range: pos..end,
                advance: w,
                glyph_id: 0,
                font_id: Default::default(),
                x_offset: 0.0,
                y_offset: 0.0,
            });
            pos = end;
        }

        // Viewport width is small enough to fit 'A' and a few dots, but not all of them.
        let viewport_width = 30.0;

        // This call will loop infinitely if the bug is present.
        let lines = compute_visual_lines(&clusters, text_bytes, char_w, viewport_width, 0.5);

        assert!(!lines.is_empty(), "Lines should not be empty");
    }
}
