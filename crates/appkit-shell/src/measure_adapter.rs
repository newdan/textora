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
}
