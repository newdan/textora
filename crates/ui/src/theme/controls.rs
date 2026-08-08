/// 基础控件共享的逻辑像素尺寸 token。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlMetrics {
    pub control_height_logical: f32,
    pub minimum_hit_target_logical: f32,
    pub corner_radius_logical: f32,
    pub compact_corner_radius_logical: f32,
    pub focus_ring_width_logical: f32,
    pub content_spacing_logical: f32,
    pub compact_spacing_logical: f32,
    pub horizontal_padding_logical: f32,
    pub font_size_logical: f32,
}

impl Default for ControlMetrics {
    fn default() -> Self {
        Self {
            control_height_logical: 32.0,
            minimum_hit_target_logical: 32.0,
            corner_radius_logical: 8.0,
            compact_corner_radius_logical: 4.0,
            focus_ring_width_logical: 2.0,
            content_spacing_logical: 8.0,
            compact_spacing_logical: 4.0,
            horizontal_padding_logical: 12.0,
            font_size_logical: 14.0,
        }
    }
}
