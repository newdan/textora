const HOVER_REDRAW_THRESHOLD_PX_SQUARED: f32 = 4.0;
/// Mouse-related state.
pub(crate) struct MouseState {
    pub pos: (f64, f64),
    /// 上一次因 overlay hover 而触发 RequestRedraw 时的鼠标位置。用于
    /// 方案 2026-07-06 阶段 4a：鼠标未跨越阈值时不重复触发 redraw。
    /// None 表示尚未推过 hover redraw，或 overlay 已关闭。
    pub last_hover_redraw_pos: Option<(f32, f32)>,
    /// 上一次推送的 HoverTab 值。用于去重（None → None 时不再推送）。
    pub last_hover_tab: Option<usize>,
}

impl MouseState {
    pub fn new() -> Self {
        Self { pos: (0.0, 0.0), last_hover_redraw_pos: None, last_hover_tab: None }
    }

    /// 判断距上次 hover redraw 是否越过阈值（默认 2 px），或状态需要刷新。
    /// 阈值内的移动通常是 sub-pixel jitter 或跨相同 overlay 元素的移动，
    /// 无需重绘。
    pub fn overlay_hover_needs_redraw(&self, px: f32, py: f32) -> bool {
        match self.last_hover_redraw_pos {
            None => true,
            Some((last_x, last_y)) => {
                let dx = px - last_x;
                let dy = py - last_y;
                dx * dx + dy * dy >= HOVER_REDRAW_THRESHOLD_PX_SQUARED
            }
        }
    }
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod state_tests {
    use super::MouseState;

    #[test]
    fn overlay_hover_needs_redraw_first_time_returns_true() {
        let mouse = MouseState::new();
        assert!(mouse.overlay_hover_needs_redraw(100.0, 200.0));
    }

    #[test]
    fn overlay_hover_needs_redraw_within_threshold_returns_false() {
        let mut mouse = MouseState::new();
        mouse.last_hover_redraw_pos = Some((100.0, 200.0));

        assert!(!mouse.overlay_hover_needs_redraw(100.5, 200.5));
        assert!(!mouse.overlay_hover_needs_redraw(101.0, 200.0));
        assert!(!mouse.overlay_hover_needs_redraw(100.0, 201.0));
    }

    #[test]
    fn overlay_hover_needs_redraw_past_threshold_returns_true() {
        let mut mouse = MouseState::new();
        mouse.last_hover_redraw_pos = Some((100.0, 200.0));

        assert!(mouse.overlay_hover_needs_redraw(102.0, 200.0));
        assert!(mouse.overlay_hover_needs_redraw(105.0, 200.0));
    }

    #[test]
    fn default_matches_new_for_complete_mouse_state() {
        let new_state = MouseState::new();
        let default_state = MouseState::default();

        assert_eq!(default_state.pos, new_state.pos);
        assert_eq!(default_state.last_hover_redraw_pos, new_state.last_hover_redraw_pos);
        assert_eq!(default_state.last_hover_tab, new_state.last_hover_tab);
    }
}
