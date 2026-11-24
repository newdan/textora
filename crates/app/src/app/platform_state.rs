use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use appkit_shell::accessibility_adapter::PlatformAccessibilityAdapter;
use winit::event_loop::EventLoopProxy;

use crate::app_event::AppEvent;
use crate::mouse::MouseState;

pub(crate) struct PlatformState {
    startup_file: Option<PathBuf>,
    startup_started_at: Instant,
    font_preparation: Option<JoinHandle<shaping::FontSystem>>,
    event_loop_proxy: Option<EventLoopProxy<AppEvent>>,
    accessibility: Option<PlatformAccessibilityAdapter>,
    redraw_pending: bool,
    last_cursor_visible: bool,
    mouse: MouseState,
}

impl PlatformState {
    pub(crate) fn new(
        startup_file: Option<PathBuf>,
        startup_started_at: Instant,
        font_preparation: Option<JoinHandle<shaping::FontSystem>>,
    ) -> Self {
        Self {
            startup_file,
            startup_started_at,
            font_preparation,
            event_loop_proxy: None,
            accessibility: None,
            redraw_pending: true,
            last_cursor_visible: true,
            mouse: MouseState::new(),
        }
    }

    pub(crate) fn startup_file(&self) -> Option<&Path> {
        self.startup_file.as_deref()
    }

    pub(crate) fn clear_startup_file(&mut self) {
        self.startup_file = None;
    }

    pub(crate) fn startup_elapsed(&self) -> Duration {
        self.startup_started_at.elapsed()
    }

    pub(crate) fn take_font_preparation(&mut self) -> Option<JoinHandle<shaping::FontSystem>> {
        self.font_preparation.take()
    }

    #[cfg(test)]
    pub(crate) fn has_font_preparation(&self) -> bool {
        self.font_preparation.is_some()
    }

    pub(crate) fn register_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<AppEvent>) {
        self.event_loop_proxy = Some(event_loop_proxy);
    }

    pub(crate) fn event_loop_proxy(&self) -> Option<&EventLoopProxy<AppEvent>> {
        self.event_loop_proxy.as_ref()
    }

    pub(crate) fn install_accessibility(&mut self, adapter: PlatformAccessibilityAdapter) {
        self.accessibility = Some(adapter);
    }

    pub(crate) fn accessibility(&self) -> Option<&PlatformAccessibilityAdapter> {
        self.accessibility.as_ref()
    }

    pub(crate) fn accessibility_mut(&mut self) -> Option<&mut PlatformAccessibilityAdapter> {
        self.accessibility.as_mut()
    }

    pub(crate) fn schedule_redraw(&mut self) {
        self.redraw_pending = true;
    }

    pub(crate) fn clear_redraw(&mut self) {
        self.redraw_pending = false;
    }

    pub(crate) fn redraw_pending(&self) -> bool {
        self.redraw_pending
    }

    pub(crate) fn update_cursor_visibility(&mut self, visible: bool) {
        if visible != self.last_cursor_visible {
            self.last_cursor_visible = visible;
            self.schedule_redraw();
        }
    }

    pub(crate) fn mouse(&self) -> &MouseState {
        &self.mouse
    }

    pub(crate) fn mouse_mut(&mut self) -> &mut MouseState {
        &mut self.mouse
    }
}
