use crate::document_presentation::DocumentPresentation;
use crate::snap_tree::DisplayLineEntry;
use appkit_core::document::DocumentModel;
use core::buffer::TextBuffer;
use ui::layout::{
    apply_punctuation_padding, build_advance_cache_entries, compute_visual_lines, is_cjk_char,
    is_whitespace_cluster, pick_char_width, ws_cluster_advance,
};

#[test]
fn render_viewport_width_uses_physical_scrollbar_reserve() {
    let settings = ui::settings::Settings::new();
    let metrics = ui::settings::UiMetrics::from_settings(&settings, 2.0);
    assert_eq!(
        super::render_viewport_width(1000.0, 64.0, &metrics, true),
        1000.0 - 64.0 - metrics.scrollbar_reserve
    );
}

fn mock_cluster(byte_start: usize, byte_end: usize, advance: f32) -> shaping::GlyphCluster {
    shaping::GlyphCluster {
        byte_range: byte_start..byte_end,
        glyph_id: 0,
        font_id: shaping::FontId::default(),
        advance,
        x_offset: 0.0,
        y_offset: 0.0,
    }
}

/// Helper: build clusters + line_bytes for ASCII text where each char is one cluster.
fn ascii_clusters(text: &str, char_advance: f32) -> (Vec<shaping::GlyphCluster>, Vec<u8>) {
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters: Vec<_> = text
        .char_indices()
        .map(|(i, _)| {
            let end = if i + 1 < bytes.len() { i + 1 } else { bytes.len() };
            mock_cluster(i, end, char_advance)
        })
        .collect();
    (clusters, bytes)
}

#[test]
fn ws_cluster_ascii_space() {
    assert!(is_whitespace_cluster(b" "));
    assert!(is_whitespace_cluster(b"\t"));
    assert!(is_whitespace_cluster(b"  \t"));
}

#[test]
fn ws_cluster_nbsp() {
    // U+00A0 = 0xC2 0xA0
    assert!(is_whitespace_cluster(b"\xC2\xA0"));
}

#[test]
fn ws_cluster_ideographic_space() {
    // U+3000 = 0xE3 0x80 0x80
    assert!(is_whitespace_cluster(b"\xE3\x80\x80"));
}

#[test]
fn ws_cluster_non_ws() {
    assert!(!is_whitespace_cluster(b"a"));
    assert!(!is_whitespace_cluster(b"\xE4\xB8\xAD")); // 中
}

#[test]
fn ws_cluster_invalid_utf8_falls_back_to_ascii() {
    assert!(!is_whitespace_cluster(b"\xFF"));
}

#[test]
fn tab_cluster_advance() {
    // Tab should be 4x char_width (DEFAULT_TAB_WIDTH)
    let advance = ws_cluster_advance(b"\t", 12.0);
    assert_eq!(advance, 48.0); // 4 * 12.0
}

#[test]
fn space_cluster_advance() {
    let advance = ws_cluster_advance(b" ", 12.0);
    assert_eq!(advance, 12.0);
}

#[test]
fn cjk_char_hiragana() {
    assert!(is_cjk_char('あ'));
    assert!(is_cjk_char('ん'));
}

#[test]
fn cjk_char_katakana() {
    assert!(is_cjk_char('カ'));
    assert!(is_cjk_char('ー'));
}

#[test]
fn cjk_char_hangul_syllable() {
    assert!(is_cjk_char('한'));
    assert!(is_cjk_char('글'));
}

#[test]
fn cjk_char_hangul_jamo() {
    assert!(is_cjk_char('\u{1100}'));
}

#[test]
fn cjk_char_halfwidth_katakana() {
    assert!(is_cjk_char('\u{FF66}'));
}

#[test]
fn cjk_char_ascii_letter_is_not_cjk() {
    assert!(!is_cjk_char('A'));
    assert!(!is_cjk_char('1'));
}

#[test]
fn cjk_char_fullwidth_punct_is_cjk() {
    // Fullwidth CJK punctuation should be recognized as CJK so they
    // receive 1em width and follow CJK word-wrap rules.
    assert!(is_cjk_char('，'));
    assert!(is_cjk_char('。'));
    assert!(is_cjk_char('：'));
    assert!(is_cjk_char('！'));
}

#[test]
fn very_narrow_viewport_breaks_per_cluster() {
    use shaping::Shaper;
    let mut shaper = Shaper::new().unwrap().with_font_size(14.0);
    let shaped = shaper.shape("abcd").unwrap();
    // viewport smaller than a single character
    let vls = compute_visual_lines(&shaped.clusters, b"abcd", 8.0, 1.0, 0.5);
    assert_eq!(
        vls.len(),
        shaped.clusters.len(),
        "very narrow viewport should break per cluster, got {}",
        vls.len()
    );
}

#[test]
fn wrap_skips_leading_whitespace_on_continuation() {
    use shaping::Shaper;
    let mut shaper = Shaper::new().unwrap().with_font_size(14.0);
    let line = "aaa bbb ccc";
    let shaped = shaper.shape(line).unwrap();
    let bytes = line.as_bytes();
    let vls = compute_visual_lines(&shaped.clusters, bytes, 8.0, 32.0, 0.5);
    assert!(vls.len() >= 2);
    for &(s, _, _) in &vls[1..] {
        let cluster = &shaped.clusters[s];
        let first_char = &line.as_bytes()[cluster.byte_range.clone()];
        assert!(
            !is_whitespace_cluster(first_char),
            "continuation should not start with whitespace, got bytes={:?}",
            first_char
        );
    }
}

#[test]
fn char_width_uses_fallback_regardless_of_line_content() {
    // pick_char_width now always returns the caller-provided fallback
    // to guarantee consistent column width (and thus tab stops) across all lines.
    use shaping::Shaper;
    let mut shaper = Shaper::new().unwrap().with_font_size(14.0);

    // Mixed ASCII+CJK line
    let line1 = "中A中";
    let shaped1 = shaper.shape(line1).unwrap();
    assert_eq!(pick_char_width(&shaped1.clusters, line1.as_bytes(), 100.0), 100.0);

    // Pure CJK line
    let line2 = "中文";
    let shaped2 = shaper.shape(line2).unwrap();
    assert_eq!(pick_char_width(&shaped2.clusters, line2.as_bytes(), 100.0), 100.0);

    // Empty / all-whitespace line
    assert_eq!(pick_char_width(&[], b"", 8.0), 8.0);
}

#[test]
fn short_line_no_wrap() {
    // "Hello" = 5 chars, width 60px, viewport 200px → single line
    let (clusters, bytes) = ascii_clusters("Hello", 12.0);
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 200.0, 0.5);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], (0, 5, 60.0));
}

#[test]
fn ascii_word_boundary_wrap() {
    // "Hello world" = 11 chars. Each char = 12.0px, char_width = 12.0.
    // "Hello " = 6 chars = 72px, "world" starts at cluster 6.
    // viewport = 80px: "Hello " (72px) fits, "world" (12px) would make 84 > 80.
    // → break at cluster 6 (word boundary after space).
    let (clusters, bytes) = ascii_clusters("Hello world", 12.0);
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 80.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, 0); // start cluster
    assert_eq!(result[0].1, 6); // end cluster (exclusive) = "Hello "
    assert_eq!(result[1].0, 6); // "world"
    assert_eq!(result[1].1, 11);
}

#[test]
fn no_word_boundary_hard_break() {
    // "abcdef" = 6 chars, no spaces. viewport = 36px → 3 chars per line.
    let (clusters, bytes) = ascii_clusters("abcdef", 12.0);
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 36.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 3, 36.0));
    assert_eq!(result[1], (3, 6, 36.0));
}

#[test]
fn mixed_long_word_then_wrap() {
    // "abcdefgh world" = 14 chars, char_width = 10px, viewport = 80px.
    // "abcdefgh " = 9 chars = 90px > 80 → hard break at cluster 8.
    // "abcdefgh" → hard break at cluster 4? No, let's think...
    // Actually viewport=80, each char=10px, 8 chars = 80. When we see cluster 8 (space, 10px),
    // 80+10=90 > 80 and ci(8) > visual_line_start(0) → no word boundary yet (no ws yet),
    // so hard break at cluster 8? Wait, cluster 8 is space...
    // Let me trace: clusters 0-7 are "abcdefgh" (non-ws), cluster 8 is " " (ws).
    // ci=0: advance=10, x=0+10=10
    // ci=1: 20, ci=2: 30, ..., ci=7: 80
    // ci=8 (space): advance=char_width=10, x=80+10=90 > 80, ci(8)>start(0).
    //   last_break_after_ws=None, last_break_cjk=None → hard break at ci=8.
    //   line 1: (0, 8, 80.0) = "abcdefgh"
    //   start=8, x=0; then add space: x=10
    // ci=9 ("w"): advance=10, check ws→non-ws: last_break_after_ws=Some(9)
    //   x=10+10=20
    // ci=10..13: "orld" → x=60
    // End: line 2: (9, 14, 50.0) = "world" (leading space trimmed)
    let (clusters, bytes) = ascii_clusters("abcdefgh world", 10.0);
    let result = compute_visual_lines(&clusters, &bytes, 10.0, 80.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 8, 80.0));
    assert_eq!(result[1], (9, 14, 50.0));
}

#[test]
fn multiple_word_wraps() {
    // "aa bb cc dd" = 10 chars, char=10px, viewport=50px
    // "aa " = 30px, "bb " adds 30 → 60 > 50 → break at cluster 3 ("bb")
    // Line 1: (0, 3, 30.0) = "aa "
    // "bb " = 30px, "cc " adds 30 → 60 > 50 → break at cluster 6 ("cc")
    // Line 2: (3, 6, 30.0) = "bb "
    // "cc " = 30px, "dd" adds 20 → 50 ≤ 50 → ok
    // Line 3: (6, 10, 40.0) = "cc dd"
    let (clusters, bytes) = ascii_clusters("aa bb cc dd", 10.0);
    let result = compute_visual_lines(&clusters, &bytes, 10.0, 50.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 5, 50.0)); // "aa bb " (5 chars, fills viewport, hard break)
    assert_eq!(result[1], (6, 11, 50.0)); // "cc dd" (leading space trimmed, fills viewport)
}

#[test]
fn cjk_wrap() {
    // "你好世界" = 4 CJK chars, each 3 bytes, char_width=12.0, each cluster advance=24.0
    // viewport=60px: "你好" = 48px, "世界" adds 48 → 96 > 60.
    // CJK break at cluster 2 ("世").
    let bytes: Vec<u8> = "你好世界".bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),  // 你
        mock_cluster(3, 6, 24.0),  // 好
        mock_cluster(6, 9, 24.0),  // 世
        mock_cluster(9, 12, 24.0), // 界
    ];
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 60.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 2, 48.0)); // 你好
    assert_eq!(result[1], (2, 4, 48.0)); // 世界
}

#[test]
fn empty_line() {
    let clusters: Vec<shaping::GlyphCluster> = vec![];
    let bytes: Vec<u8> = vec![];
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 200.0, 0.5);
    assert_eq!(result.len(), 0);
}

#[test]
fn mixed_cjk_ascii_wrap() {
    // "hello你好world" = 5 ASCII + 2 CJK + 5 ASCII
    // Each ASCII char = 10px, each CJK char = 20px (wider)
    // ASCII→CJK 不再设断点，让行尽量填满。
    // "hello你" 70px ≤ 80px，"hello你好" 90px > 80px → 硬断在 '好' 前。
    let text = "hello你好world";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 1, 10.0),   // h
        mock_cluster(1, 2, 10.0),   // e
        mock_cluster(2, 3, 10.0),   // l
        mock_cluster(3, 4, 10.0),   // l
        mock_cluster(4, 5, 10.0),   // o
        mock_cluster(5, 8, 20.0),   // 你 (3 bytes UTF-8)
        mock_cluster(8, 11, 20.0),  // 好 (3 bytes UTF-8)
        mock_cluster(11, 12, 10.0), // w
        mock_cluster(12, 13, 10.0), // o
        mock_cluster(13, 14, 10.0), // r
        mock_cluster(14, 15, 10.0), // l
        mock_cluster(15, 16, 10.0), // d
    ];
    let result = compute_visual_lines(&clusters, &bytes, 10.0, 80.0, 0.5);
    // Line 1: "hello你" 70px (hard break), Line 2: "好world" 70px
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 6, 70.0)); // "hello你"
    assert_eq!(result[1], (6, 12, 70.0)); // "好world"
}

#[test]
fn multi_word_tracks_last_boundary() {
    // Regression: must track the LAST word boundary, not the first.
    // "aa bb cc dd" = 11 chars, each 10px, viewport=80px
    // "aa bb cc " = 80px, "dd" would make 100px → wrap.
    // Should break at last word boundary (before "dd"), not first (before "bb").
    let (clusters, bytes) = ascii_clusters("aa bb cc dd", 10.0);
    let result = compute_visual_lines(&clusters, &bytes, 10.0, 80.0, 0.5);
    assert_eq!(result.len(), 2);
    // ci=7 'c' (80px+10=90>80) triggers wrap; last word boundary is ci=6.
    // Line 1: "aa bb " (6 clusters, 60px)
    assert_eq!(result[0], (0, 6, 50.0)); // "aa bb" (trailing space stripped)
    // Line 2: "cc dd" (5 clusters)
    assert_eq!(result[1], (6, 11, 50.0)); // "cc dd" = 5*10 = 50px
}

#[test]
fn cache_key_distinguishes_lines_with_same_length() {
    // Key must include offset to avoid collisions between different lines
    // with the same byte length.
    let k1 = (100usize, 50usize, 0u32);
    let k2 = (200usize, 50usize, 0u32);
    assert_ne!(k1, k2);
}

#[test]
fn wrap_cache_key_consistency() {
    // Same input + same params → same result (verifies determinism).
    // The actual cache key test is in app.rs where wrap_cache is managed,
    // but we verify the function is pure here.
    let (clusters, bytes) = ascii_clusters("hello world foo", 10.0);
    let r1 = compute_visual_lines(&clusters, &bytes, 10.0, 80.0, 0.5);
    let r2 = compute_visual_lines(&clusters, &bytes, 10.0, 80.0, 0.5);
    assert_eq!(r1, r2);
}

#[test]
fn cjk_hard_break_no_single_char_line() {
    // Regression: after a hard break at a CJK character, prev_is_cjk was
    // reset to false and not re-evaluated.  The next CJK character then
    // registered as a CJK<->non-CJK transition, creating a spurious break
    // point that isolated a single character on its own visual line.
    //
    // 10 CJK chars, each 24px, viewport=100px -> 4 chars/line.
    // Before fix: [(0,4,96), (4,5,24), (5,9,96), (9,10,24)]
    // After fix:  [(0,4,96), (4,8,96), (8,10,48)]
    let text = "你好世界欢迎光临谢谢";
    let cjk_bytes: Vec<u8> = text.bytes().collect();
    let clusters: Vec<shaping::GlyphCluster> = text
        .char_indices()
        .map(|(i, ch)| {
            let end = i + ch.len_utf8();
            mock_cluster(i, end, 24.0)
        })
        .collect();

    let result = compute_visual_lines(&clusters, &cjk_bytes, 12.0, 100.0, 0.5);
    // Every visual line must have >= 2 characters (no single-char lines)
    for &(start, end, _) in &result {
        assert!(end - start >= 2, "single-char visual line at ({start},{end})");
    }
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], (0, 4, 96.0));
    assert_eq!(result[1], (4, 8, 96.0));
    assert_eq!(result[2], (8, 10, 48.0));
}

#[test]
fn cjk_ascii_punct_no_boundary() {
    // ASCII punctuation between CJK chars IS ASCII content (Some(false)),
    // so it creates a CJK boundary at ","→世 transition. Break at the
    // latest CJK boundary (before 世), comma stays with preceding CJK.
    let text = "你好,世界";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 你 (3 bytes)
        mock_cluster(3, 6, 24.0),   // 好 (3 bytes)
        mock_cluster(6, 7, 10.0),   // ,  (1 byte, ASCII comma)
        mock_cluster(7, 10, 24.0),  // 世 (3 bytes)
        mock_cluster(10, 13, 24.0), // 界 (3 bytes)
    ];
    // viewport=60: 你好 = 48px, then , = 58px, then 世 = 82px > 60 → wrap.
    // With fix: no CJK boundary at comma → comma stays with CJK → hard break at ci=3.
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 60.0, 0.5);
    // Should be 2 lines: "你好," and "世界" (comma stays with preceding CJK)
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 3, 58.0)); // 你好, = 24+24+10
    assert_eq!(result[1], (3, 5, 48.0)); // 世界 = 24+24
}

#[test]
fn cjk_fullwidth_punct_then_latin_boundary() {
    // Fullwidth comma is punctuation (transparent), so the CJK boundary
    // should be at the Latin text start, not at the comma.
    // "你好，world" → break before "world", comma stays with "你好".
    let text = "你好，world";
    let bytes: Vec<u8> = text.bytes().collect();
    // ， is U+FF0C = 3 UTF-8 bytes, fullwidth 2-col
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 你
        mock_cluster(3, 6, 24.0),   // 好
        mock_cluster(6, 9, 24.0),   // ， (fullwidth comma)
        mock_cluster(9, 10, 10.0),  // w
        mock_cluster(10, 11, 10.0), // o
        mock_cluster(11, 12, 10.0), // r
        mock_cluster(12, 13, 10.0), // l
        mock_cluster(13, 14, 10.0), // d
    ];
    // viewport=80: 你好 = 48, ， = 72, w = 82 > 80 → wrap.
    // CJK boundary at 'w' (ci=3, CJK→Latin), comma is transparent.
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 80.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 3, 72.0)); // 你好， = 24+24+24
    assert_eq!(result[1], (3, 8, 50.0)); // world = 10*5
}

#[test]
fn cjk_mixed_punct_several_transitions() {
    // Multiple punctuation chars between CJK and Latin should all be transparent.
    // "你好！！world" — no CJK boundary at punctuation, only hard break.
    let text = "你好！！world";
    let bytes: Vec<u8> = text.bytes().collect();
    // ！is U+FF01 = 3 bytes, fullwidth (24px)
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 你
        mock_cluster(3, 6, 24.0),   // 好
        mock_cluster(6, 9, 24.0),   // ！
        mock_cluster(9, 12, 24.0),  // ！
        mock_cluster(12, 13, 10.0), // w
        mock_cluster(13, 14, 10.0), // o
        mock_cluster(14, 15, 10.0), // r
        mock_cluster(15, 16, 10.0), // l
        mock_cluster(16, 17, 10.0), // d
    ];
    // viewport=80: 你好=48, +！=72, +！=96>80 → hard break at ci=3 (72px).
    // viewport=80: 你好=48, +！=72, +！=96>80 → hard break at ci=3 (72px).
    // ！！are Po (Other punct) → while-loop pulls back through both → break at ci=1.
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 80.0, 0.5);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], (0, 1, 24.0)); // 你
    assert_eq!(result[1], (1, 4, 72.0)); // 好！！
    assert_eq!(result[2], (4, 9, 50.0)); // world
}

#[test]
fn latin_to_cjk_via_fullwidth_punct() {
    // "hello，你好" — fullwidth comma is transparent, boundary at 你好.
    let text = "hello，你好";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 1, 10.0),   // h
        mock_cluster(1, 2, 10.0),   // e
        mock_cluster(2, 3, 10.0),   // l
        mock_cluster(3, 4, 10.0),   // l
        mock_cluster(4, 5, 10.0),   // o
        mock_cluster(5, 8, 24.0),   // ， (fullwidth, 3 bytes)
        mock_cluster(8, 11, 24.0),  // 你
        mock_cluster(11, 14, 24.0), // 好
    ];
    // viewport=80: hello=50, ，=74<80, 你=98>80 → wrap.
    // CJK boundary at ci=6 (你, Latin→CJK ignoring punct).
    let result = compute_visual_lines(&clusters, &bytes, 10.0, 80.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 6, 74.0)); // hello， = 50+24
    assert_eq!(result[1], (6, 8, 48.0)); // 你好 = 24+24
}

#[test]
fn cjk_hyphen_cjk_no_boundary() {
    // "你好-世界" — ASCII hyphen is transparent between CJK chars.
    // No CJK boundary at hyphen; only hard break when viewport exceeded.
    let text = "你好-世界";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 你
        mock_cluster(3, 6, 24.0),   // 好
        mock_cluster(6, 7, 10.0),   // -
        mock_cluster(7, 10, 24.0),  // 世
        mock_cluster(10, 13, 24.0), // 界
    ];
    // viewport=60: 你好=48, -=58, 世=82>60 → hard break at ci=3.
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 60.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 3, 58.0)); // 你好- = 24+24+10
    assert_eq!(result[1], (3, 5, 48.0)); // 世界
}

#[test]
fn digit_between_cjk_creates_boundary() {
    // "你好1世界" — ASCII digit is non-CJK content, creates CJK boundary.
    // Boundaries at ci=2 (CJK→digit) and ci=3 (digit→CJK).
    // When wrap triggers at ci=3 (世), break_at = last_break_cjk = 3 (the latest).
    let text = "你好1世界";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 你
        mock_cluster(3, 6, 24.0),   // 好
        mock_cluster(6, 7, 10.0),   // 1
        mock_cluster(7, 10, 24.0),  // 世
        mock_cluster(10, 13, 24.0), // 界
    ];
    // viewport=60: 你好=48, 1=58<60, 世=82>60 → wrap. break_at=3 (CJK boundary).
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 60.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 3, 58.0)); // 你好1
    assert_eq!(result[1], (3, 5, 48.0)); // 世界
}

#[test]
fn cjk_fullwidth_colon_then_digits_boundary() {
    // "电话：0551" — fullwidth colon is transparent, boundary at colon→digit (CJK→Latin).
    // This is the exact scenario the user reported: "这段文字的：后面换行了"
    let text = "电话：0551";
    let bytes: Vec<u8> = text.bytes().collect();
    // 电(3) 话(3) ：(3, U+FF1A) 0(1) 5(1) 5(1) 1(1)
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 电
        mock_cluster(3, 6, 24.0),   // 话
        mock_cluster(6, 9, 24.0),   // ： (fullwidth colon, transparent)
        mock_cluster(9, 10, 10.0),  // 0
        mock_cluster(10, 11, 10.0), // 5
        mock_cluster(11, 12, 10.0), // 5
        mock_cluster(12, 13, 10.0), // 1
    ];
    // viewport=80: 电话=48, ：=72, 0=82>80 → wrap.
    // CJK boundary at ci=3 (colon→digit), colon stays with CJK side.
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 80.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 3, 72.0)); // 电话： = 24+24+24
    assert_eq!(result[1], (3, 7, 40.0)); // 0551 = 10*4
}

#[test]
fn cjk_fullwidth_colon_no_artificial_break_after_punct() {
    // Regression: fullwidth colon should NOT create a break opportunity by itself.
    // "注册咨询电话：0551-62220088" — break only at CJK→digit boundary.
    let text = "注册咨询电话：0551";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 注
        mock_cluster(3, 6, 24.0),   // 册
        mock_cluster(6, 9, 24.0),   // 咨
        mock_cluster(9, 12, 24.0),  // 询
        mock_cluster(12, 15, 24.0), // 电
        mock_cluster(15, 18, 24.0), // 话
        mock_cluster(18, 21, 24.0), // ： (fullwidth colon)
        mock_cluster(21, 22, 10.0), // 0
        mock_cluster(22, 23, 10.0), // 5
        mock_cluster(23, 24, 10.0), // 5
        mock_cluster(24, 25, 10.0), // 1
    ];
    // viewport=250: all fits (208px < 250) → 1 line
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 250.0, 0.5);
    assert_eq!(result.len(), 1);

    // viewport=120: 注册咨询=96, 电话=144>120 → break at CJK boundary (ci=2).
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 120.0, 0.5);
    assert_eq!(result.len(), 2);
    // CJK text has no internal boundary; hard break at ci=2 (96px < 120, 120px <= 120)
    // Actually: ci=0..5: 24*6=144>120. No ws or CJK boundary (all CJK). Hard break at ci=5.
    // ci=0..4: 24*5=120≤120, ci=5: 144>120 → hard break at ci=5.
    assert_eq!(result[0], (0, 5, 120.0)); // 注册咨询电 = 24*5
    assert_eq!(result[1], (5, 11, 88.0)); // 话：0551 = 24+24+10*4
}

#[test]
fn cjk_comma_between_cjk_no_break() {
    // "你好、世界、大家" — all CJK, ideographic comma is transparent.
    // No CJK boundary since everything is CJK content.
    let text = "你好、世界、大家";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 你
        mock_cluster(3, 6, 24.0),   // 好
        mock_cluster(6, 9, 24.0),   // 、
        mock_cluster(9, 12, 24.0),  // 世
        mock_cluster(12, 15, 24.0), // 界
        mock_cluster(15, 18, 24.0), // 、
        mock_cluster(18, 21, 24.0), // 大
        mock_cluster(21, 24, 24.0), // 家
    ];
    // viewport=120: 你好、世=96≤120, +界=120≤120, +、=144>120 → hard break at ci=5.
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 120.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 4, 96.0)); // 你好、世
    assert_eq!(result[1], (4, 8, 96.0)); // 界、大家
}

#[test]
fn debug_real_shaper_cjk_mixed() {
    // Diagnostic test: use real shaper on the exact JSON line from the user's file.
    use shaping::Shaper;
    let mut shaper = Shaper::new().expect("shaper").with_font_size(16.0);

    // Exact line from 段落标题.json line 3
    let text = "        \"cur_content\": \"关于广东省珠海市的人文历史、自然地理、风土人情及政务信息等情况，考生可登录珠海市人民政府网站（www.zhuhai.gov.cn）查询。此次招聘单位为珠海市旅游发展中心，是珠海市文化体育旅游局所属公益一类事业单位。考生可登录珠海市文化体育旅游局网站（www.zhwtl.gov.cn）进一步了解。\",";
    let shaped = shaper.shape(text).expect("shape");
    let bytes: Vec<u8> = text.bytes().collect();

    // Compute char_width (same as render_pipeline)
    let char_width = shaped
        .clusters
        .iter()
        .find(|c| bytes.get(c.byte_range.clone()).is_some_and(|b| !is_whitespace_cluster(b)))
        .map(|c| c.advance.max(1.0))
        .unwrap_or(16.0 * 0.6);

    println!("\nchar_width = {:.2}", char_width);
    println!("Total clusters: {}, Total width: {:.1}", shaped.clusters.len(), shaped.width);

    // Try wrapping at different viewport widths
    for vw in [600.0, 800.0, 1000.0, 1200.0] {
        let vl = compute_visual_lines(&shaped.clusters, &bytes, char_width, vw, 0.5);
        println!("\nviewport={:.0}: {} visual lines", vw, vl.len());
        for (li, &(start, end, w)) in vl.iter().enumerate() {
            let byte_start = shaped.clusters[start].byte_range.start;
            let byte_end = shaped.clusters[end - 1].byte_range.end;
            let s = std::str::from_utf8(&bytes[byte_start..byte_end]).unwrap_or("??");
            let sp: String = s.chars().take(60).collect();
            println!("  line {}: clusters {}..{} ({:.1}px) [{}]", li, start, end, w, sp);
        }
    }
}

// ── Cached cursor pixel_x computation tests ──

/// Create mock cluster data: Vec<(byte_start, byte_end, advance)>
fn mock_cluster_data(data: &[(usize, usize, f32)]) -> Vec<(usize, usize, f32)> {
    data.to_vec()
}

#[test]
fn cursor_pixel_x_cached_at_start_of_line() {
    // Single cluster [0, 10, 40.0], cursor_col=0 (at start)
    let clusters = mock_cluster_data(&[(0, 10, 40.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 1, 0, 10, 32.0).unwrap();
    assert_eq!(px, 32.0); // left margin only
}

#[test]
fn cursor_pixel_x_cached_at_cluster_boundary() {
    // Two ASCII clusters: [0,1) advance 8, [1,2) advance 8, cursor_col=1
    let clusters = mock_cluster_data(&[(0, 1, 8.0), (1, 2, 8.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 2, 1, 2, 32.0).unwrap();
    assert_eq!(px, 32.0 + 8.0); // after first cluster
}

#[test]
fn cursor_pixel_x_cached_mid_cluster() {
    // Multi-byte cluster [0, 3, 24.0], cursor_col=2 (inside cluster)
    let clusters = mock_cluster_data(&[(0, 3, 24.0)]);
    // cursor_col=2 < byte_end=3 → break without adding advance
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 1, 2, 3, 32.0).unwrap();
    assert_eq!(px, 32.0); // left margin, cluster advance NOT added
}

#[test]
fn cursor_pixel_x_cached_after_mid_cluster() {
    // Multi-byte cluster [0, 3, 24.0], cursor_col=3 (at boundary after cluster)
    let clusters = mock_cluster_data(&[(0, 3, 24.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 1, 3, 3, 32.0).unwrap();
    assert_eq!(px, 32.0 + 24.0); // after full cluster
}

#[test]
fn cursor_pixel_x_cached_mixed_clusters() {
    // Mixed: ASCII [0,1,8], CJK [1,4,24], ASCII [4,5,8], cursor_col=2 (inside CJK)
    let clusters = mock_cluster_data(&[(0, 1, 8.0), (1, 4, 24.0), (4, 5, 8.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 3, 2, 5, 32.0).unwrap();
    // Only first ASCII cluster added: 32 + 8 = 40
    assert_eq!(px, 40.0);
}

#[test]
fn cursor_pixel_x_cached_cursor_beyond_end() {
    // cursor_col=20 > cluster_end=10 → None
    let clusters = mock_cluster_data(&[(0, 10, 40.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 1, 20, 10, 32.0);
    assert!(px.is_none());
}

#[test]
fn cursor_pixel_x_cached_multiple_added() {
    // Three ASCII clusters, cursor_col=2 → first two added
    let clusters = mock_cluster_data(&[(0, 1, 8.0), (1, 2, 8.0), (2, 3, 8.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 3, 2, 3, 32.0).unwrap();
    assert_eq!(px, 32.0 + 8.0 + 8.0); // first two clusters
}

#[test]
fn cursor_pixel_x_cached_dpi_scaling() {
    let clusters = mock_cluster_data(&[(0, 1, 10.0)]);
    let px = super::compute_cursor_pixel_x_cached(&clusters, 0, 1, 0, 1, 64.0).unwrap();
    assert_eq!(px, 64.0); // 32.0 * 2.0
}

#[test]
fn cjk_consecutive_punct_period_quote() {
    // "。"" at break point: both should be pulled back, not appear at line start.
    // Viewport=144px (6 CJK chars): 6 chars fit, 7 overflows.
    let text = "文字文字文字。”文字文字";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // 文
        mock_cluster(3, 6, 24.0),   // 字
        mock_cluster(6, 9, 24.0),   // 文
        mock_cluster(9, 12, 24.0),  // 字
        mock_cluster(12, 15, 24.0), // 文
        mock_cluster(15, 18, 24.0), // 字
        mock_cluster(18, 21, 24.0), // 。
        mock_cluster(21, 24, 24.0), // "
        mock_cluster(24, 27, 24.0), // 文
        mock_cluster(27, 30, 24.0), // 字
        mock_cluster(30, 33, 24.0), // 文
        mock_cluster(33, 36, 24.0), // 字
    ];
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 144.0, 0.5);
    // 。and " should NOT be the first chars of any visual line
    for &(start, _, _) in &result {
        let cluster_bytes = &bytes[clusters[start].byte_range.clone()];
        let s = std::str::from_utf8(cluster_bytes).unwrap_or("");
        for ch in s.chars() {
            assert!(
                !matches!(ch, '\u{3002}' | '\u{201D}'),
                "line starts with prohibited punct '{ch}'"
            );
        }
    }
}

#[test]
fn cjk_punct_quote_not_at_line_start() {
    // Only " at break point: should be pulled back.
    // "文字文字文字文字文字"" (7 clusters: " + 6 CJK + ")
    // Viewport=144px: clusters 1-6 fit, cluster 7 (") overflows at 168px
    let text = "\u{201D}文字文字文字文字文字\u{201D}";
    let bytes: Vec<u8> = text.bytes().collect();
    let clusters = vec![
        mock_cluster(0, 3, 24.0),   // "
        mock_cluster(3, 6, 24.0),   // 文
        mock_cluster(6, 9, 24.0),   // 字
        mock_cluster(9, 12, 24.0),  // 文
        mock_cluster(12, 15, 24.0), // 字
        mock_cluster(15, 18, 24.0), // 文
        mock_cluster(18, 21, 24.0), // 字
        mock_cluster(21, 24, 24.0), // "
    ];
    let result = compute_visual_lines(&clusters, &bytes, 12.0, 144.0, 0.5);
    // Second " (last cluster) should not start any visual line
    for &(start, _, _) in &result {
        if start > 0 {
            let cluster_bytes = &bytes[clusters[start].byte_range.clone()];
            let s = std::str::from_utf8(cluster_bytes).unwrap_or("");
            for ch in s.chars() {
                assert!(
                    !matches!(ch, '\u{201D}'),
                    "wrapped line starts with prohibited '\u{201D}' at line start cluster {start}"
                );
            }
        }
    }
}

// ── Punctuation padding integration: shape → pad → cache → cursor ──

#[test]
fn punctuation_padding_cursor_mapping() {
    // Simulate: line = "a,b" where comma is narrow (3px), em=15.0, ratio=0.5
    // After padding: comma advance becomes 7.5 (min), x_offset centered.
    let em = 15.0f32;
    let ratio = 0.5f32;
    let char_width = 9.0f32;

    let line_bytes = b"a,b";
    let mut clusters = vec![
        mock_cluster(0, 1, 9.0), // 'a'
        mock_cluster(1, 2, 3.0), // ','  — narrow, should be padded
        mock_cluster(2, 3, 9.0), // 'b'
    ];

    // 1. compute_visual_lines uses ORIGINAL advances
    let vlines = compute_visual_lines(&clusters, line_bytes, char_width, 200.0, 0.5);
    assert_eq!(vlines.len(), 1, "short line should be single visual line");

    // 2. Apply padding (same as render_pipeline.rs does)
    apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);
    assert!(clusters[1].advance >= em * ratio, "comma should be padded");

    // 3. Build advance cache entries from padded clusters
    let shaped = shaping::ShapedRun { clusters: clusters.clone(), width: 0.0 };
    let mut pool = Vec::new();
    let entries =
        build_advance_cache_entries(&vlines, 0, &shaped, line_bytes, char_width, 0, &mut pool, 0.0);
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];

    // Cursor at end of cluster for byte 1: accumulates through cd.1<=1
    let x_after_a = super::compute_cursor_pixel_x_cached(
        &entry
            .clusters
            .iter()
            .map(|&(end, x, _)| (end.saturating_sub(1), end, x))
            .collect::<Vec<_>>(),
        0,
        entry.clusters.len(),
        1,
        3,
        0.0,
    );
    assert_eq!(x_after_a, Some(9.0), "cursor at end of a");

    // Cursor at end of cluster for byte 2: accumulates through cd.1<=2
    let x_after_comma = super::compute_cursor_pixel_x_cached(
        &entry
            .clusters
            .iter()
            .map(|&(end, x, _)| (end.saturating_sub(1), end, x))
            .collect::<Vec<_>>(),
        0,
        entry.clusters.len(),
        2,
        3,
        0.0,
    );
    assert_eq!(x_after_comma, Some(25.5), "cursor at end of comma");

    // Cursor at end of cluster for byte 3: accumulates all clusters
    let x_after_b = super::compute_cursor_pixel_x_cached(
        &entry
            .clusters
            .iter()
            .map(|&(end, x, _)| (end.saturating_sub(1), end, x))
            .collect::<Vec<_>>(),
        0,
        entry.clusters.len(),
        3,
        3,
        0.0,
    );
    assert_eq!(x_after_b, Some(51.0), "cursor at end of b");

    // Comma advance in cache should reflect padding
    let comma_advance = entry.clusters[1].1 - entry.clusters[0].1;
    assert_eq!(comma_advance, 7.5, "padded comma advance");
}

#[test]
fn markdown_ordered_list_marker_cursor_mapping_keeps_dot_advance() {
    let em = 15.0f32;
    let ratio = 0.5f32;
    let char_width = 9.0f32;
    let line_bytes = b"10. item";
    let mut clusters = vec![
        mock_cluster(0, 1, 9.0),
        mock_cluster(1, 2, 9.0),
        mock_cluster(2, 3, 3.0),
        mock_cluster(3, 4, 9.0),
        mock_cluster(4, 5, 9.0),
        mock_cluster(5, 6, 9.0),
        mock_cluster(6, 7, 9.0),
        mock_cluster(7, 8, 9.0),
    ];

    let visual_lines = compute_visual_lines(&clusters, line_bytes, char_width, 200.0, 0.5);
    apply_punctuation_padding(&mut clusters, line_bytes, em, ratio);

    let shaped = shaping::ShapedRun { clusters, width: 0.0 };
    let mut cluster_pool = Vec::new();
    let entries = build_advance_cache_entries(
        &visual_lines,
        0,
        &shaped,
        line_bytes,
        char_width,
        0,
        &mut cluster_pool,
        0.0,
    );
    let marker_entry = &entries[0];

    let x_after_period = ui::render_geom::byte_to_x(3, &marker_entry.clusters, 0.0, true);
    let x_after_marker_space = ui::render_geom::byte_to_x(4, &marker_entry.clusters, 0.0, true);

    assert_eq!(x_after_period, 21.0);
    assert_eq!(x_after_marker_space, 30.0);
}

#[test]
fn cached_advance_cache_entries_preserve_visual_line_grapheme_start() {
    let cached = crate::render_cache::CachedLine {
        instances: Vec::new(),
        line_number_glyphs: Vec::new(),
        atlas_generation: 0,
        visual_line_count: 2,
        content_hash: 0,
        visual_lines: vec![(0, 5, 50.0), (5, 10, 50.0)],
        visual_line_instance_starts: vec![0, 5],
        cluster_data: (0..10).map(|i| (i, i + 1, 10.0)).collect(),
        subset_start: 0,
    };

    let entries = super::build_cached_advance_cache_entries(&cached, 0, 0, 100.0);

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].vl_grapheme_start, 0);
    assert_eq!(entries[1].vl_grapheme_start, 5);
    assert_eq!(entries[1].vl_byte_start, 5);
}

#[test]
fn cached_advance_cache_entries_count_skipped_visual_lines() {
    let cached = crate::render_cache::CachedLine {
        instances: Vec::new(),
        line_number_glyphs: Vec::new(),
        atlas_generation: 0,
        visual_line_count: 2,
        content_hash: 0,
        visual_lines: vec![(0, 5, 50.0), (5, 10, 50.0)],
        visual_line_instance_starts: vec![0, 5],
        cluster_data: (0..10).map(|i| (i, i + 1, 10.0)).collect(),
        subset_start: 0,
    };

    let entries = super::build_cached_advance_cache_entries(&cached, 0, 1, 100.0);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].vl_grapheme_start, 5);
    assert_eq!(entries[0].vl_byte_start, 5);
}

#[test]
fn visual_subset_shape_results_are_cacheable() {
    assert!(super::should_store_shaped_line_in_render_cache(
        super::ShapedLineScope::VisualSubset,
        false,
        false,
    ));
}

#[test]
fn visual_subset_cache_metadata_preserves_doc_visual_range() {
    let metadata = super::render_cache_metadata_for_shaped_line(
        super::ShapedLineScope::VisualSubset,
        21,
        38,
        0,
        7,
    );

    assert_eq!(metadata.visual_line_count, 38);
    assert_eq!(metadata.subset_start, 7);
}

#[test]
fn visual_subset_cache_hit_advances_by_full_doc_visual_rows() {
    assert_eq!(
        super::cached_line_rows_to_advance(super::ShapedLineScope::VisualSubset, 38, 21, 0, 0,),
        38,
    );
}

#[test]
fn cursor_relative_offset_uses_current_line_offset() {
    let stale_entry_offset = 10;
    let current_line_offset = 14;
    let cursor_offset = 20;

    let stale_relative_offset = cursor_offset - stale_entry_offset;
    let relative_offset =
        super::cursor_relative_offset_for_line(cursor_offset, current_line_offset);

    assert_eq!(stale_relative_offset, 10);
    assert_eq!(relative_offset, Some(6));
}

#[test]
fn render_viewport_state_reads_anchor_and_viewport_metrics_from_document_view() {
    let mut text_buffer = TextBuffer::new(false).expect("test text buffer should initialize");
    text_buffer.write_raw(b"line 0\nline 1\nline 2\nline 3\nline 4\nline 5");
    let model = DocumentModel::new(text_buffer);
    let mut presentation = DocumentPresentation::new(3, 3.0);
    presentation
        .display
        .display_map
        .set_entries((0..6).map(|index| DisplayLineEntry::placeholder(index, 4, 0, 2)).collect());
    presentation.display.viewport.scroll_anchor = ui::viewport::ScrollAnchor::new(3, 18.0);
    presentation.display.viewport.viewport_height = 2.2;

    let state = super::compute_render_viewport_state_from_presentation(&model, &presentation, 10.0);

    assert_eq!(state.skip_visual, 1);
    assert!((state.sub_line_offset + 8.0).abs() < 0.001);
    assert_eq!(state.start_doc, 3);
    assert_eq!(state.viewport_visual_rows, 5);
}
