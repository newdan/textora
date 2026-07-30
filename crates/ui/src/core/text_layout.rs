//! UiTextLayout — 纯 harfbuzz shape 结果，零 GPU 依赖。
//! Widget 在内容变化时构建，paint 时通过 DrawCmd::TextLayout 传递到 app 层。

use std::sync::atomic::{AtomicU64, Ordering};

/// Shear factor for italic glyphs — applied in vertex stage to avoid cosmic-text synthetic italic.
pub const ITALIC_SHEAR: f32 = 0.10;

/// 全局自增 ID，用于 RenderCache key
static NEXT_LAYOUT_ID: AtomicU64 = AtomicU64::new(1);

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
}
