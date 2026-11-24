//! Sidebar visibility, animation timing, and cross-frame scrolling.

use super::types::{SidebarConfig, Visibility};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct SidebarInteraction {
    pub hover: SidebarHoverState,
    pub list_scroll_offset: f32,
}

impl SidebarInteraction {
    pub fn new(cfg: &SidebarConfig) -> Self {
        Self { hover: SidebarHoverState::new(cfg), list_scroll_offset: 0.0 }
    }

    pub fn current_width(&self, cfg: &SidebarConfig, dpi: f32) -> f32 {
        match self.hover.visibility {
            Visibility::Hidden => 0.0,
            Visibility::HoverPeek | Visibility::HoverPeekFadingOut | Visibility::Pinned => {
                cfg.width_px(dpi)
            }
        }
    }

    pub fn editor_left_offset(&self, cfg: &SidebarConfig, dpi: f32) -> f32 {
        match self.hover.visibility {
            Visibility::Pinned => cfg.width_px(dpi),
            _ => 0.0,
        }
    }

    /// Hover state machine: tick every frame.
    pub fn tick(&mut self, now: Instant) -> (bool, bool) {
        self.hover.tick(now)
    }
}

/// Visibility and animation scheduling for the sidebar.
#[derive(Debug, Clone, Default)]
pub struct SidebarHoverState {
    pub visibility: Visibility,
    pub hover_enter_at: Option<Instant>,
    pub hover_leave_at: Option<Instant>,
    pub hover_peek_start: Option<Instant>,
    pub hover_peek_leave_start: Option<Instant>,
    pub suppress_hover_enter: bool,
}

impl SidebarHoverState {
    pub(super) fn new(cfg: &SidebarConfig) -> Self {
        let visibility = if cfg.pinned { Visibility::Pinned } else { Visibility::Hidden };
        Self { visibility, ..Self::default() }
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

    /// Update visibility scheduling from pointer geometry computed by the caller.
    /// Returns whether a visible widget should clear its immediate button hover.
    pub(super) fn pointer_moved(
        &mut self,
        in_hot_zone: bool,
        in_sidebar: bool,
        now: Instant,
    ) -> bool {
        match self.visibility {
            Visibility::Hidden => {
                if !in_hot_zone {
                    self.hover_enter_at = None;
                    self.suppress_hover_enter = false;
                } else if !self.suppress_hover_enter {
                    self.hover_enter_at.get_or_insert(now);
                }
                false
            }
            Visibility::HoverPeek => {
                if in_sidebar {
                    self.hover_leave_at = None;
                } else {
                    self.hover_leave_at.get_or_insert(now);
                }
                !in_sidebar
            }
            Visibility::HoverPeekFadingOut => {
                if in_sidebar {
                    self.visibility = Visibility::HoverPeek;
                    self.hover_peek_leave_start = None;
                    self.hover_leave_at = None;
                }
                !in_sidebar
            }
            Visibility::Pinned => !in_sidebar,
        }
    }
}
