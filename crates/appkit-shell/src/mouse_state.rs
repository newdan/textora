use std::ops::Range;
use std::time::Instant;

const HOVER_REDRAW_THRESHOLD_PX_SQUARED: f32 = 4.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasDragEligibility {
    Enabled,
    Disabled,
}

/// 当前编辑器指针捕获类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseCapture {
    None,
    TextSelection,
    CanvasDrag,
}

#[derive(Clone, Debug)]
pub struct CanvasDragSession {
    pub source_range: Range<usize>,
    pub pressed_at: (f32, f32),
    pub source_generation: u32,
    pub eligibility: CanvasDragEligibility,
    pub started: bool,
}

/// Mouse-related state.
pub struct MouseState {
    pub pos: (f64, f64),
    pub is_down: bool,
    pub down_byte_offset: Option<usize>,
    /// Semantic source range that bounds the current custom-renderer text drag.
    pub wysiwyg_selection_scope: Option<Range<usize>>,
    pub canvas_drag: Option<CanvasDragSession>,
    pub last_click_time: Instant,
    pub last_click_pos: (f32, f32),
    pub click_count: u8,
    /// 上一次因 overlay hover 而触发 RequestRedraw 时的鼠标位置。用于
    /// 方案 2026-07-06 阶段 4a：鼠标未跨越阈值时不重复触发 redraw。
    /// None 表示尚未推过 hover redraw，或 overlay 已关闭。
    pub last_hover_redraw_pos: Option<(f32, f32)>,
    /// 上一次推送的 HoverTab 值。用于去重（None → None 时不再推送）。
    pub last_hover_tab: Option<usize>,
}

impl MouseState {
    pub fn capture(&self) -> MouseCapture {
        if self.canvas_drag.is_some() {
            MouseCapture::CanvasDrag
        } else if self.is_down {
            MouseCapture::TextSelection
        } else {
            MouseCapture::None
        }
    }

    pub fn captures_pointer(&self) -> bool {
        self.capture() != MouseCapture::None
    }

    pub fn new() -> Self {
        Self {
            pos: (0.0, 0.0),
            is_down: false,
            down_byte_offset: None,
            wysiwyg_selection_scope: None,
            canvas_drag: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            last_hover_redraw_pos: None,
            last_hover_tab: None,
        }
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
mod tests {
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
        assert_eq!(default_state.is_down, new_state.is_down);
        assert_eq!(default_state.down_byte_offset, new_state.down_byte_offset);
        assert_eq!(default_state.wysiwyg_selection_scope, new_state.wysiwyg_selection_scope);
        assert!(default_state.canvas_drag.is_none());
        assert!(new_state.canvas_drag.is_none());
        assert!(default_state.last_click_time >= new_state.last_click_time);
        assert_eq!(default_state.last_click_pos, new_state.last_click_pos);
        assert_eq!(default_state.click_count, new_state.click_count);
        assert_eq!(default_state.last_hover_redraw_pos, new_state.last_hover_redraw_pos);
        assert_eq!(default_state.last_hover_tab, new_state.last_hover_tab);
    }

    #[test]
    fn capture_query_prioritizes_canvas_drag_over_text_selection() {
        let mut mouse = MouseState::new();
        assert_eq!(mouse.capture(), super::MouseCapture::None);
        assert!(!mouse.captures_pointer());

        mouse.is_down = true;
        assert_eq!(mouse.capture(), super::MouseCapture::TextSelection);
        assert!(mouse.captures_pointer());

        mouse.canvas_drag = Some(super::CanvasDragSession {
            source_range: 0..1,
            pressed_at: (0.0, 0.0),
            source_generation: 1,
            eligibility: super::CanvasDragEligibility::Enabled,
            started: false,
        });
        assert_eq!(mouse.capture(), super::MouseCapture::CanvasDrag);
    }
}
