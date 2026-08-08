//! UiTextLayout — 纯 harfbuzz shape 结果，零 GPU 依赖。
//! Widget 在内容变化时构建，paint 时通过 DrawCmd::TextLayout 传递到 app 层。

use std::sync::atomic::{AtomicU64, Ordering};
use unicode_segmentation::UnicodeSegmentation;

/// Shear factor for italic glyphs — applied in vertex stage to avoid cosmic-text synthetic italic.
pub const ITALIC_SHEAR: f32 = 0.10;

/// 全局自增 ID，用于 RenderCache key
static NEXT_LAYOUT_ID: AtomicU64 = AtomicU64::new(1);

/// 按 Unicode 词边界优先换行，单个词过宽时退化到 grapheme 边界。
/// 当内容超过 `max_lines` 时，最后一行以不越界的省略号收尾。
pub fn wrap_text_to_lines(
    text: &str,
    max_width: f32,
    max_lines: usize,
    measure_width: impl Fn(&str) -> f32,
) -> Vec<String> {
    if text.is_empty() || !max_width.is_finite() || max_width <= 0.0 || max_lines == 0 {
        return Vec::new();
    }

    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(paragraph, max_width, &measure_width, &mut lines);
    }

    let truncated = lines.len() > max_lines;
    lines.truncate(max_lines);
    if truncated && let Some(last_line) = lines.last_mut() {
        append_fitting_ellipsis(last_line, max_width, &measure_width);
    }
    lines
}

fn wrap_paragraph(
    paragraph: &str,
    max_width: f32,
    measure_width: &impl Fn(&str) -> f32,
    lines: &mut Vec<String>,
) {
    if paragraph.is_empty() {
        lines.push(String::new());
        return;
    }

    let mut current_line = String::new();
    for word_boundary in paragraph.split_word_bounds() {
        let boundary =
            if current_line.is_empty() { word_boundary.trim_start() } else { word_boundary };
        if boundary.is_empty() {
            continue;
        }
        let candidate = format!("{current_line}{boundary}");
        if measure_width(&candidate) <= max_width {
            current_line = candidate;
            continue;
        }
        if !current_line.is_empty() {
            lines.push(current_line.trim_end().to_owned());
            current_line.clear();
        }
        append_graphemes(boundary.trim_start(), max_width, measure_width, lines, &mut current_line);
    }
    if !current_line.is_empty() {
        lines.push(current_line.trim_end().to_owned());
    }
}

fn append_graphemes(
    text: &str,
    max_width: f32,
    measure_width: &impl Fn(&str) -> f32,
    lines: &mut Vec<String>,
    current_line: &mut String,
) {
    for grapheme in text.graphemes(true) {
        let candidate = format!("{current_line}{grapheme}");
        if current_line.is_empty() || measure_width(&candidate) <= max_width {
            current_line.push_str(grapheme);
            continue;
        }
        lines.push(std::mem::take(current_line));
        current_line.push_str(grapheme);
    }
}

fn append_fitting_ellipsis(
    line: &mut String,
    max_width: f32,
    measure_width: &impl Fn(&str) -> f32,
) {
    const ELLIPSIS: &str = "…";
    while !line.is_empty() && measure_width(&format!("{line}{ELLIPSIS}")) > max_width {
        let Some((last_grapheme_byte, _)) = line.grapheme_indices(true).next_back() else { break };
        line.truncate(last_grapheme_byte);
    }
    if measure_width(ELLIPSIS) <= max_width {
        line.push_str(ELLIPSIS);
    }
}

/// 预 shape 的文本布局数据（纯 harfbuzz 产出，无 atlas）。
/// 在 crates/ui 定义，app 层消费。
#[derive(Clone, Debug)]
pub struct UiTextLayout {
    /// 全局唯一 ID（RenderCache key）
    pub id: u64,
    /// 原始文本
    pub text: String,
    /// 字号
    pub font_size: f32,
    /// 字体族
    pub font_family: Option<String>,
    /// 字重
    pub font_weight: shaping::Weight,
    /// 字体样式
    pub font_style: shaping::Style,
    /// 是否由 app 层应用斜体剪切变换（不依赖 cosmic-text 合成斜体）
    pub italic: bool,
    /// Harfbuzz shape 结果（clusters with glyph IDs, advances, positions）
    pub shaped: shaping::ShapedRun,
}

impl PartialEq for UiTextLayout {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl UiTextLayout {
    /// 从已有的 ShapedRun 构造（避免 re-shape）。
    /// 调用时机：已有 shape 结果（如 preview layout 产出）。
    pub fn from_shaped(
        text: &str,
        font_size: f32,
        font_family: Option<String>,
        font_weight: shaping::Weight,
        font_style: shaping::Style,
        italic: bool,
        shaped: shaping::ShapedRun,
    ) -> Self {
        Self {
            id: NEXT_LAYOUT_ID.fetch_add(1, Ordering::Relaxed),
            text: text.to_string(),
            font_size,
            font_family,
            font_weight,
            font_style,
            italic,
            shaped,
        }
    }

    /// Shape text 并创建 UiTextLayout。
    /// 调用时机：widget 内容或样式变化时。
    pub fn new(
        text: &str,
        font_size: f32,
        font_family: Option<String>,
        font_weight: shaping::Weight,
        font_style: shaping::Style,
        italic: bool,
        shaper: &mut shaping::Shaper,
    ) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        // Save and restore shaper state
        let old_size = shaper.font_size();
        let old_weight = shaper.font_weight();
        let old_style = shaper.font_style();
        let old_family = shaper.font_family().map(|s| s.to_string());
        shaper.set_font_size(font_size);
        shaper.set_font_weight(font_weight);
        shaper.set_font_style(font_style);
        shaper.set_font_family(font_family.as_deref());

        let result = shaper.shape(text).ok().map(|shaped| {
            let id = NEXT_LAYOUT_ID.fetch_add(1, Ordering::Relaxed);
            Self {
                id,
                text: text.to_string(),
                font_size,
                font_family: font_family.clone(),
                font_weight,
                font_style,
                italic,
                shaped,
            }
        });

        // Restore
        shaper.set_font_size(old_size);
        shaper.set_font_weight(old_weight);
        shaper.set_font_style(old_style);
        shaper.set_font_family(old_family.as_deref());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn with_shaper(f: impl FnOnce(&mut shaping::Shaper)) {
        let mut shaper = shaping::Shaper::new().expect("failed to create shaper");
        f(&mut shaper);
    }

    #[test]
    fn from_shaped_assigns_unique_ids() {
        with_shaper(|shaper| {
            let run1 = shaper.shape("hello").unwrap();
            let run2 = shaper.shape("world").unwrap();
            let tl1 = UiTextLayout::from_shaped(
                "hello",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                run1,
            );
            let tl2 = UiTextLayout::from_shaped(
                "world",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                run2,
            );
            assert_ne!(tl1.id, tl2.id, "IDs should be unique");
        });
    }

    #[test]
    fn from_shaped_preserves_text() {
        with_shaper(|shaper| {
            let run = shaper.shape("test text").unwrap();
            let tl = UiTextLayout::from_shaped(
                "test text",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                run,
            );
            assert_eq!(tl.text, "test text");
            assert_eq!(tl.font_size, 14.0);
        });
    }

    #[test]
    fn new_empty_text_returns_none() {
        with_shaper(|shaper| {
            let result = UiTextLayout::new(
                "",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                shaper,
            );
            assert!(result.is_none(), "empty text should return None");
        });
    }

    #[test]
    fn new_produces_valid_layout() {
        with_shaper(|shaper| {
            let tl = UiTextLayout::new(
                "hello",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                shaper,
            );
            assert!(tl.is_some(), "non-empty text should produce layout");
            let tl = tl.unwrap();
            assert_eq!(tl.text, "hello");
            assert!(!tl.shaped.clusters.is_empty(), "should have clusters");
        });
    }

    #[test]
    fn new_restores_shaper_state() {
        with_shaper(|shaper| {
            shaper.set_font_size(20.0);
            shaper.set_font_weight(shaping::Weight::BOLD);
            let _tl = UiTextLayout::new(
                "test",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                shaper,
            );
            // Shaper state should be restored
            assert_eq!(shaper.font_size(), 20.0, "font_size should be restored");
            assert_eq!(
                shaper.font_weight(),
                shaping::Weight::BOLD,
                "font_weight should be restored"
            );
        });
    }

    #[test]
    fn new_restores_font_family_after_explicit_family() {
        with_shaper(|shaper| {
            let _tl = UiTextLayout::new(
                "code",
                14.0,
                Some("monospace".to_string()),
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                shaper,
            );
            assert_eq!(
                shaper.font_family(),
                None,
                "temporary font families must not leak into later default text shaping"
            );
        });
    }

    #[test]
    fn clone_preserves_id() {
        with_shaper(|shaper| {
            let run = shaper.shape("clone test").unwrap();
            let tl = UiTextLayout::from_shaped(
                "clone test",
                14.0,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                run,
            );
            let cloned = tl.clone();
            assert_eq!(tl.id, cloned.id);
            assert_eq!(tl.text, cloned.text);
        });
    }

    #[test]
    fn wrapping_prefers_word_boundaries_and_truncates_on_graphemes() {
        let measure = |text: &str| text.graphemes(true).count() as f32;

        assert_eq!(
            wrap_text_to_lines("alpha beta gamma", 10.0, 3, measure),
            vec!["alpha beta", "gamma"]
        );
        let truncated = wrap_text_to_lines("一二三四五六七八九", 3.0, 2, measure);
        assert_eq!(truncated.len(), 2);
        assert!(truncated[1].ends_with('…'));
        assert!(truncated.iter().all(|line| measure(line) <= 3.0));
    }

    #[test]
    fn wrapping_never_splits_an_emoji_grapheme() {
        let family = "👨‍👩‍👧‍👦";
        let text = format!("{family}{family}");
        let lines =
            wrap_text_to_lines(&text, 1.0, 2, |candidate| candidate.graphemes(true).count() as f32);

        assert_eq!(lines, vec![family, family]);
    }
}
