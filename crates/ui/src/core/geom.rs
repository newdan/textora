//! 物理像素几何 + 屏幕→NDC 单一转换。
//! ui crate 内部除本文件外不应再出现 NDC 形态的 [f32; 4]。

#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn left(self) -> f32 {
        self.x
    }
    pub fn top(self) -> f32 {
        self.y
    }
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// 缩进 (top, right, bottom, left)，得到内部 rect。
    /// 若缩进总和超过尺寸，返回 ZERO。
    pub fn shrink(self, top: f32, right: f32, bottom: f32, left: f32) -> Rect {
        let w = self.w - left - right;
        let h = self.h - top - bottom;
        if w <= 0.0 || h <= 0.0 {
            return Rect::ZERO;
        }
        Rect::new(self.x + left, self.y + top, w, h)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Screen {
    pub w: f32,
    pub h: f32,
}

impl Screen {
    pub fn new(w: f32, h: f32) -> Self {
        Self { w: w.max(1.0), h: h.max(1.0) }
    }

    /// 像素 (x: 左→右; y: 上→下) 转 NDC ([-1, 1]; y: 上正下负)。
    pub fn px_to_ndc(self, x: f32, y: f32) -> [f32; 2] {
        [x / self.w * 2.0 - 1.0, 1.0 - y / self.h * 2.0]
    }

    /// 像素 Rect 转 NDC [left, right, top, bottom]（与现有代码约定一致）。
    pub fn rect_to_ndc(self, r: Rect) -> [f32; 4] {
        let [l, t] = self.px_to_ndc(r.left(), r.top());
        let [right, b] = self.px_to_ndc(r.right(), r.bottom());
        [l, right, t, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_inclusive_left_top_exclusive_right_bottom() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(10.0, 20.0)); // 左上含
        assert!(r.contains(109.99, 69.99)); // 右下接近边
        assert!(!r.contains(110.0, 20.0)); // 右边不含
        assert!(!r.contains(10.0, 70.0)); // 下边不含
        assert!(!r.contains(9.99, 20.0)); // 左外不含
    }

    #[test]
    fn rect_shrink_normal_case() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let s = r.shrink(10.0, 5.0, 20.0, 15.0);
        assert_eq!(s.x, 15.0);
        assert_eq!(s.y, 10.0);
        assert_eq!(s.w, 80.0);
        assert_eq!(s.h, 70.0);
    }

    #[test]
    fn rect_shrink_exceeding_returns_zero() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        let s3 = r.shrink(10.0, 10.0, 10.0, 10.0);
        assert_eq!(s3, Rect::ZERO);
    }

    #[test]
    fn screen_px_to_ndc_center_is_zero() {
        let s = Screen::new(800.0, 600.0);
        let [x, y] = s.px_to_ndc(400.0, 300.0);
        assert!((x - 0.0).abs() < 0.001);
        assert!((y - 0.0).abs() < 0.001);
    }

    #[test]
    fn screen_px_to_ndc_corners() {
        let s = Screen::new(800.0, 600.0);
        let [x_tl, y_tl] = s.px_to_ndc(0.0, 0.0);
        assert!((x_tl + 1.0).abs() < 0.001);
        assert!((y_tl - 1.0).abs() < 0.001);
        let [x_br, y_br] = s.px_to_ndc(800.0, 600.0);
        assert!((x_br - 1.0).abs() < 0.001);
        assert!((y_br + 1.0).abs() < 0.001);
    }

    #[test]
    fn rect_to_ndc_maps_correctly() {
        let s = Screen::new(800.0, 600.0);
        let r = Rect::new(0.0, 0.0, 800.0, 600.0);
        let ndc = s.rect_to_ndc(r);
        assert!((ndc[0] + 1.0).abs() < 0.001); // left
        assert!((ndc[1] - 1.0).abs() < 0.001); // right
        assert!((ndc[2] - 1.0).abs() < 0.001); // top
        assert!((ndc[3] + 1.0).abs() < 0.001); // bottom
    }

    #[test]
    fn screen_new_clamps_w_h_to_min_1() {
        let s = Screen::new(0.0, -5.0);
        assert_eq!(s.w, 1.0);
        assert_eq!(s.h, 1.0);
    }
}
