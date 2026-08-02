//! macOS NSWindow bridge for fullSizeContentView / titlebar integration.
//!
//! When ViewMode::Sidebar is active, we make the titlebar transparent and
//! move traffic lights into the sidebar header area. Switching back to
//! ViewMode::Tabs restores the native titlebar.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSView, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    /// Get the NSWindow from a winit Window.
    fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
        let handle = window.window_handle().ok()?;
        let raw = handle.as_raw();
        let RawWindowHandle::AppKit(h) = raw else {
            return None;
        };
        let ns_view_ptr: *mut c_void = h.ns_view.as_ptr();
        // SAFETY: The pointer is valid as long as the window exists.
        unsafe {
            let ns_view = Retained::<NSView>::retain(ns_view_ptr.cast())?;
            let ns_window = ns_view.window()?;
            Some(ns_window)
        }
    }

    pub fn enable_full_size_content(window: &Window) {
        let Some(ns_win) = ns_window(window) else {
            return;
        };
        ns_win.setTitlebarAppearsTransparent(true);
        ns_win.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        let mut mask = ns_win.styleMask();
        mask.insert(NSWindowStyleMask::FullSizeContentView);
        ns_win.setStyleMask(mask);
    }

    pub fn disable_full_size_content(window: &Window) {
        let Some(ns_win) = ns_window(window) else {
            return;
        };
        ns_win.setTitlebarAppearsTransparent(false);
        ns_win.setTitleVisibility(NSWindowTitleVisibility::Visible);
        let mut mask = ns_win.styleMask();
        mask.remove(NSWindowStyleMask::FullSizeContentView);
        ns_win.setStyleMask(mask);
    }

    /// Returns (left_inset, top_inset) in physical pixels for the traffic
    /// light button area. Used by sidebar to offset header content.
    #[allow(dead_code)]
    pub fn traffic_light_inset(window: &Window) -> (f32, f32) {
        let Some(ns_win) = ns_window(window) else {
            return (0.0, 0.0);
        };
        let Some(close_btn) = ns_win.standardWindowButton(NSWindowButton::CloseButton) else {
            return (0.0, 0.0);
        };
        let frame = close_btn.frame();
        // left: close + minimize + zoom buttons + spacing (~12px each + gaps)
        let left = (frame.origin.x) + (frame.size.width) * 3.5;
        let top = (frame.origin.y) + (frame.size.height) + 4.0;
        (left as f32, top as f32)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use winit::window::Window;

    #[allow(unused_variables)]
    pub fn enable_full_size_content(window: &Window) {}

    #[allow(unused_variables)]
    pub fn disable_full_size_content(window: &Window) {}

    #[allow(unused_variables)]
    pub fn traffic_light_inset(window: &Window) -> (f32, f32) {
        (0.0, 0.0)
    }
}

pub(crate) use imp::{disable_full_size_content, enable_full_size_content};
