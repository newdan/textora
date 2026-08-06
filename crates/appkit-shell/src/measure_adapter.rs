//! measure_adapter: 将 shaping::Shaper 包装为 ui::TextMeasure。
//! app 层通过它注入 LayoutCtx，供 ui 组件测量文本宽度。

use ui::core::measure::TextMeasure;

/// 用 Shaper 实现的 TextMeasure。
/// 调用前后会还原 Shaper 的 font_size，避免副作用。
pub struct MeasureFromShaper<'a>(pub &'a mut shaping::Shaper);

impl TextMeasure for MeasureFromShaper<'_> {
    fn measure(&mut self, s: &str, font_size: f32) -> f32 {
        let old = self.0.font_size();
        self.0.set_font_size(font_size);
        let w = self.0.shape(s).map(|r| r.width).unwrap_or(0.0);
        self.0.set_font_size(old);
        w
    }

    fn measure_with_font(
        &mut self,
        text: &str,
        font_size: f32,
        font_family: Option<&str>,
        font_weight: shaping::Weight,
        font_style: shaping::Style,
    ) -> f32 {
        let old_size = self.0.font_size();
        let old_family = self.0.font_family().map(str::to_owned);
        let old_weight = self.0.font_weight();
        let old_style = self.0.font_style();

        self.0.set_font_size(font_size);
        self.0.set_font_family(font_family);
        self.0.set_font_weight(font_weight);
        self.0.set_font_style(font_style);
        let width = self.0.shape(text).map(|run| run.width).unwrap_or(0.0);

        self.0.set_font_size(old_size);
        self.0.set_font_family(old_family.as_deref());
        self.0.set_font_weight(old_weight);
        self.0.set_font_style(old_style);
        width
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "ci-no-fonts"))]
    use super::*;

    /// 验证 MeasureFromShaper 还原调用前的 font_size。
    /// 需要字体文件；本地开发测试通过，CI 无字体时跳过。
    #[test]
    #[cfg(not(feature = "ci-no-fonts"))]
    fn measure_restores_caller_font_size() {
        use shaping::Shaper;

        let mut shaper = Shaper::new().expect("Shaper::new() should succeed with fonts");
        let original = shaper.font_size();

        {
            let mut adapter = MeasureFromShaper(&mut shaper);
            let _w = adapter.measure("Hello", 24.0);
        }

        assert_eq!(shaper.font_size(), original, "font_size should be restored after measure");
    }

    #[test]
    #[cfg(not(feature = "ci-no-fonts"))]
    fn title_text_box_caret_measure_matches_its_painted_font() {
        use shaping::{Shaper, Style, Weight};
        use ui::core::{LayoutCtx, Rect};
        use ui::text_box::TextBox;

        const TITLE_FONT_SIZE: f32 = 24.0;
        const TITLE: &str = "H2AI 战略";
        const CURSOR_PREFIX: &str = "H2AI 战";

        let mut shaper = Shaper::new().expect("system fonts should create a shaper");
        shaper.set_font_family(None);
        shaper.set_font_size(TITLE_FONT_SIZE);
        shaper.set_font_weight(Weight::NORMAL);
        shaper.set_font_style(Style::Normal);
        let painted_prefix_width = shaper
            .shape(CURSOR_PREFIX)
            .expect("title prefix should shape with the painted font")
            .width;

        shaper.set_font_family(Some("Menlo"));
        let mut text_box = TextBox::new();
        text_box.set_font_size_logical(TITLE_FONT_SIZE);
        text_box.set_leading_content_inset_logical(0.0);
        text_box.set_text("123456789");
        text_box.sync_text(TITLE);

        let theme = ui::theme::test_theme();
        {
            let mut measure = MeasureFromShaper(&mut shaper);
            let mut layout_context =
                LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
            text_box.layout(Rect::new(0.0, 0.0, 300.0, 40.0), &mut layout_context);
        }

        assert_eq!(text_box.cursor_byte(), CURSOR_PREFIX.len());
        assert!(
            (text_box.ime_cursor_rect().x - painted_prefix_width).abs() < 0.01,
            "caret measurement must use the same font as title painting"
        );
        assert_eq!(shaper.font_family(), Some("Menlo"));
    }
}
