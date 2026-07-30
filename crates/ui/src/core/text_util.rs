//! core/text_util.rs — 文本宽度估算与截断工具。
//! 原位于 tab_bar/text.rs，迁移到 core 以便 list 等 widget 共用。

use crate::core::measure::TextMeasure;
use shaping::Shaper;

/// 估算单个字符的像素宽度。
/// ASCII 约 0.5em，CJK/宽字符约 1.0em。
pub fn char_width(ch: char, font_size: f32) -> f32 {
    if ch.is_ascii() && ch != '\u{2026}' { font_size * 0.5 } else { font_size * 1.0 }
}

/// 截断标题使其像素宽度不超过 `max_width_px`。
/// 超长时用中间省略号 (…) 截断。
pub fn truncate_title_by_width(title: &str, max_width_px: f32, font_size: f32) -> String {
    let full_w = estimate_text_width_px(title, font_size);
    if full_w <= max_width_px {
        return title.to_string();
    }
    let chars: Vec<char> = title.chars().collect();
    let ellipsis_w = font_size * 1.0;
    let half = (max_width_px - ellipsis_w) * 0.5;

    let mut prefix_w = 0.0;
    let mut prefix_len = 0;
    for ch in &chars {
        let cw = char_width(*ch, font_size);
        if prefix_w + cw > half {
            break;
        }
        prefix_w += cw;
        prefix_len += 1;
    }

    let mut suffix_w = 0.0;
    let mut suffix_chars: Vec<char> = Vec::new();
    for ch in chars.iter().rev() {
        let cw = char_width(*ch, font_size);
        if suffix_w + cw > half {
            break;
        }
        suffix_w += cw;
        suffix_chars.push(*ch);
    }

    let prefix: String = chars[..prefix_len].iter().collect();
    let suffix: String = suffix_chars.iter().rev().collect();
    format!("{}…{}", prefix, suffix)
}

/// 精确截断：通过 TextMeasure 测量宽度，前后缀对称平分可用空间。
/// 与 truncate_title_by_width 同算法，区别仅在使用精确测量而非 char_width 估算。
pub fn truncate_title_precise(
    title: &str,
    max_width_px: f32,
    font_size: f32,
    measure: &mut dyn TextMeasure,
) -> String {
    let full_w = measure.measure(title, font_size);
    if full_w <= max_width_px {
        return title.to_string();
    }
    let chars: Vec<char> = title.chars().collect();
    let n = chars.len();
    if n == 0 {
        return String::new();
    }
    let ellipsis = "\u{2026}";
    let ellipsis_w = measure.measure(ellipsis, font_size);
    let half = (max_width_px - ellipsis_w) * 0.5;
    // 10% tolerance: 允许前后缀略微超出 half，避免在词语中间截断
    let bound = half * 1.10;

    // longest prefix that fits in bound
    let prefix_len = {
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let s: String = chars[..mid].iter().collect();
            if measure.measure(&s, font_size) <= bound {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    };

    // longest suffix that fits in bound (no overlap with prefix)
    let suffix_len = {
        let max_suffix = n.saturating_sub(prefix_len);
        let mut lo = 0usize;
        let mut hi = max_suffix;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let s: String = chars[n - mid..].iter().collect();
            if measure.measure(&s, font_size) <= bound {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    };

    if prefix_len == 0 && suffix_len == 0 {
        let first: String = chars[..1].iter().collect();
        return format!("{first}{ellipsis}");
    }

    // 如果总宽超限，从 prefix 端逐字缩减（prefix 信息量通常低于 suffix）
    let mut p = prefix_len;
    loop {
        let prefix: String = chars[..p].iter().collect();
        let suffix: String = chars[n - suffix_len..].iter().collect();
        let pw = measure.measure(&prefix, font_size);
        let sw = measure.measure(&suffix, font_size);
        if pw + ellipsis_w + sw <= max_width_px || p == 0 {
            return format!("{prefix}{ellipsis}{suffix}");
        }
        p -= 1;
    }
}

/// 估算文本字符串的像素宽度。
pub fn estimate_text_width_px(title: &str, font_size: f32) -> f32 {
    title.chars().map(|ch| char_width(ch, font_size)).sum()
}

/// 使用 shaper 计算文本宽度（回退到估算）。
pub fn compute_text_width(title: &str, font_size: f32, shaper: Option<&mut Shaper>) -> f32 {
    let Some(shaper) = shaper else {
        return estimate_text_width_px(title, font_size);
    };
    let old_size = shaper.font_size();
    shaper.set_font_size(font_size);
    let width = shaper.shape(title).map(|r| r.width).unwrap_or(0.0);
    shaper.set_font_size(old_size);
    if width > 0.0 { width } else { estimate_text_width_px(title, font_size) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::TextMeasure;

    /// 用 char_width 估算的 TextMeasure，供 truncate_title_precise 测试使用。
    struct EstMeasure;
    impl TextMeasure for EstMeasure {
        fn measure(&mut self, s: &str, font_size: f32) -> f32 {
            s.chars().map(|ch| char_width(ch, font_size)).sum()
        }
    }

    #[test]
    fn short_title_not_truncated() {
        let result = truncate_title_by_width("main.rs", 200.0, 13.0);
        assert_eq!(result, "main.rs");
    }

    #[test]
    fn long_title_truncated_with_ellipsis() {
        let result =
            truncate_title_by_width("very_long_filename_that_exceeds_width.rs", 80.0, 13.0);
        assert!(result.contains('…'), "Expected ellipsis in: {result}");
        assert!(result.len() < "very_long_filename_that_exceeds_width.rs".len());
    }

    #[test]
    fn char_width_ascii_half_em() {
        let w = char_width('a', 12.0);
        assert!((w - 6.0).abs() < 0.01);
    }

    #[test]
    fn char_width_cjk_one_em() {
        let w = char_width('文', 12.0);
        assert!((w - 12.0).abs() < 0.01);
    }

    // ── truncate_title_precise ──

    #[test]
    fn precise_short_title_not_truncated() {
        let mut m = EstMeasure;
        let result = truncate_title_precise("main.rs", 200.0, 13.0, &mut m);
        assert_eq!(result, "main.rs");
    }

    #[test]
    fn precise_long_title_truncated() {
        let mut m = EstMeasure;
        let result =
            truncate_title_precise("very_long_filename_that_exceeds_width.rs", 80.0, 13.0, &mut m);
        assert!(result.contains('…'), "Expected ellipsis in: {result}");
        assert!(result.len() < "very_long_filename_that_exceeds_width.rs".len());
    }

    #[test]
    fn precise_empty_title() {
        let mut m = EstMeasure;
        let result = truncate_title_precise("", 100.0, 13.0, &mut m);
        assert_eq!(result, "");
    }

    #[test]
    fn precise_result_width_within_max() {
        let mut m = EstMeasure;
        let title = "a_very_long_filename_that_should_be_truncated.rs";
        for max_w in [60.0, 80.0, 100.0, 120.0, 150.0] {
            let result = truncate_title_precise(title, max_w, 13.0, &mut m);
            let result_w = m.measure(&result, 13.0);
            assert!(
                result_w <= max_w + 0.01,
                "result width {result_w} > max {max_w} for '{result}'"
            );
        }
    }

    #[test]
    fn precise_narrow_width_gives_first_char_plus_ellipsis() {
        let mut m = EstMeasure;
        // Narrow enough that nothing fits — should yield first char + …
        let result = truncate_title_precise("hello.txt", 30.0, 13.0, &mut m);
        assert!(result.starts_with('h'), "Expected start with first char, got '{result}'");
        assert!(result.contains('…'), "Expected ellipsis in: {result}");
    }
}
