//! 文本测量 trait。
//! 由 app 层实现并注入 LayoutCtx；ui crate 不持有 Shaper。

/// 文本宽度测量接口。
/// app 端通过 `MeasureFromShaper` 包一层 `Shaper` 实现。
pub trait TextMeasure {
    fn measure(&mut self, s: &str, font_size: f32) -> f32;
}

/// 测试用空实现：所有测量返回 0.0。
pub struct NoopMeasure;

impl TextMeasure for NoopMeasure {
    fn measure(&mut self, _s: &str, _font_size: f32) -> f32 {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_returns_zero_without_shaping() {
        let mut m = NoopMeasure;
        let w = m.measure("", 14.0);
        assert_eq!(w, 0.0);
    }

    #[test]
    fn noop_measure_always_returns_zero() {
        let mut m = NoopMeasure;
        assert_eq!(m.measure("hello world", 16.0), 0.0);
        assert_eq!(m.measure("測試中文", 12.0), 0.0);
    }
}
