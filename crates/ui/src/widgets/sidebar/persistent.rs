//! SidebarPersistent — 跨帧保持的 sidebar 状态。

use super::types::{HOT_BAND_LOGICAL, SidebarConfig, SidebarHoverButton, Visibility};
use std::time::{Duration, Instant};

// ── Persistent State (minimal cross-frame backup) ──

#[derive(Debug, Clone)]
pub struct SidebarPersistent {
    pub visibility: Visibility,
    pub hovered_index: Option<usize>,
    pub list_scroll_offset: f32,
    pub open_menu: Option<crate::widgets::popup_menu::PopupMenu>,
    pub hover_enter_at: Option<Instant>,
    pub hover_leave_at: Option<Instant>,
    pub hover_peek_start: Option<Instant>,
    pub hover_peek_leave_start: Option<Instant>,
    pub hovered_button: SidebarHoverButton,
    /// When true, suppress hover entry until mouse leaves hot zone.
    /// Set when visibility is programmatically changed to Hidden (e.g. TogglePin).
    pub suppress_hover_enter: bool,
    pub settings_btn_rect: crate::core::Rect,
}

impl SidebarPersistent {
    pub fn new(cfg: &SidebarConfig) -> Self {
        Self {
            visibility: if cfg.pinned { Visibility::Pinned } else { Visibility::Hidden },
            hovered_index: None,
            list_scroll_offset: 0.0,
            open_menu: None,
            hover_enter_at: None,
            hover_leave_at: None,
            hover_peek_start: None,
            hover_peek_leave_start: None,
            hovered_button: SidebarHoverButton::None,
            suppress_hover_enter: false,
            settings_btn_rect: crate::core::Rect::ZERO,
        }
    }

    pub fn current_width(&self, cfg: &SidebarConfig) -> f32 {
        match self.visibility {
            Visibility::Hidden => 0.0,
            Visibility::HoverPeek | Visibility::HoverPeekFadingOut | Visibility::Pinned => {
                cfg.width
            }
        }
    }

    pub fn editor_left_offset(&self, cfg: &SidebarConfig) -> f32 {
        match self.visibility {
            Visibility::Pinned => cfg.width,
            _ => 0.0,
        }
    }

    /// Hover state machine: tick every frame.
    /// Returns (visibility_changed, animating).
    pub fn tick(&mut self, now: Instant) -> (bool, bool) {
        match self.visibility {
            Visibility::Hidden => {
                if self.hover_enter_at.is_some() {
                    self.visibility = Visibility::HoverPeek;
                    self.hover_enter_at = None;
                    self.hover_peek_start = Some(now);
                    return (true, true);
                }
            }
            Visibility::HoverPeek => {
                // fade-in still in progress?
                let animating = if let Some(start) = self.hover_peek_start {
                    now.duration_since(start) < Duration::from_millis(150)
                } else {
                    false
                };
                if self.hover_leave_at.is_some() {
                    self.visibility = Visibility::HoverPeekFadingOut;
                    self.hover_leave_at = None;
                    self.hover_peek_leave_start = Some(now);
                    return (true, true);
                }
                if animating {
                    return (false, true);
                }
            }
            Visibility::HoverPeekFadingOut => {
                if let Some(leave_start) = self.hover_peek_leave_start
                    && now.duration_since(leave_start) >= Duration::from_millis(150)
                {
                    self.visibility = Visibility::Hidden;
                    self.hover_peek_leave_start = None;
                    self.hover_peek_start = None;
                    return (true, false);
                }
                return (false, true);
            }
            Visibility::Pinned => {}
        }
        (false, false)
    }
    /// Feed mouse position into hover state machine.
    pub fn on_mouse_move(
        &mut self,
        px: f32,
        py: f32,
        screen_w: f32,
        dpi: f32,
        traffic_light_inset_x: f32,
        cfg: &SidebarConfig,
    ) {
        let hot_band = HOT_BAND_LOGICAL * dpi;
        let in_left_hot = px >= 0.0 && px <= hot_band;
        // Hamburger button as trigger zone
        let btn_size = 16.0 * dpi;
        let hx = traffic_light_inset_x + 8.0 * dpi;
        let hy = 16.0 * dpi - btn_size * 0.5;
        let on_hamburger = px >= hx && px <= hx + btn_size && py >= hy && py <= hy + btn_size;
        let in_hot_zone = in_left_hot || on_hamburger;

        match self.visibility {
            Visibility::Hidden => {
                if in_hot_zone {
                    if self.suppress_hover_enter {
                        // Suppress: visibility was just set to Hidden programmatically
                        // (e.g. TogglePin). Wait for mouse to leave hot zone first.
                    } else if self.hover_enter_at.is_none() {
                        self.hover_enter_at = Some(Instant::now());
                    }
                } else {
                    self.hover_enter_at = None;
                    self.suppress_hover_enter = false;
                }
            }
            Visibility::HoverPeek => {
                let sidebar_w = cfg.width;
                let in_sidebar = px >= 0.0 && px <= sidebar_w && px < screen_w;
                if !in_sidebar {
                    self.hovered_button = SidebarHoverButton::None;
                    if self.hover_leave_at.is_none() {
                        self.hover_leave_at = Some(Instant::now());
                    }
                } else {
                    self.hover_leave_at = None;
                }
            }
            Visibility::HoverPeekFadingOut => {
                let sidebar_w = cfg.width;
                let in_sidebar = px >= 0.0 && px <= sidebar_w && px < screen_w;
                if in_sidebar {
                    // 鼠标回到 sidebar：取消 fade out，回到 HoverPeek
                    self.visibility = Visibility::HoverPeek;
                    self.hover_peek_leave_start = None;
                    self.hover_leave_at = None;
                } else {
                    self.hovered_button = SidebarHoverButton::None;
                }
            }
            Visibility::Pinned => {
                let sidebar_w = cfg.width;
                let in_sidebar = px >= 0.0 && px <= sidebar_w && px < screen_w;
                if !in_sidebar {
                    self.hovered_button = SidebarHoverButton::None;
                }
            }
        }
    }
}
