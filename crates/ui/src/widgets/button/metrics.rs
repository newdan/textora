/// 两款产品共享的按钮横向尺寸，单位为逻辑像素。
pub struct ButtonMetrics;

impl ButtonMetrics {
    pub const HORIZONTAL_PADDING: f32 = 8.0;
    pub const ICON_GAP: f32 = 4.0;
    pub const ACTION_GAP: f32 = 4.0;
    pub const MENU_WIDTH: f32 = 24.0;
    pub const FONT_SIZE: f32 = 14.0;

    /// 固定中文标签按全宽字形预留内容宽度，两侧各保留统一内边距。
    pub const fn text_width(wide_glyph_count: usize) -> f32 {
        wide_glyph_count as f32 * Self::FONT_SIZE + Self::HORIZONTAL_PADDING * 2.0
    }

    pub const fn icon_text_width(wide_glyph_count: usize, icon_size: f32) -> f32 {
        Self::text_width(wide_glyph_count) + icon_size + Self::ICON_GAP
    }
}
