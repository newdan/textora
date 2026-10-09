//! macOS NSWindow bridge for fullSizeContentView / titlebar integration.
//!
//! Both view modes draw below the transparent native titlebar and keep
//! the platform traffic lights.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSView, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility,
    };
    use objc2_foundation::{NSPoint, NSRect};
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
        align_native_traffic_lights(&ns_win);
    }

    fn titlebar_center_y(content_bounds: NSRect, content_is_flipped: bool) -> f64 {
        let titlebar_half_height = f64::from(ui::constants::TITLE_BAR_HEIGHT) * 0.5;
        if content_is_flipped {
            content_bounds.origin.y + titlebar_half_height
        } else {
            content_bounds.origin.y + content_bounds.size.height - titlebar_half_height
        }
    }

    fn align_native_traffic_lights(ns_win: &NSWindow) {
        let Some(content_view) = ns_win.contentView() else {
            return;
        };
        let target_y = titlebar_center_y(content_view.bounds(), content_view.isFlipped());
        for button_kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            let Some(button) = ns_win.standardWindowButton(button_kind) else {
                continue;
            };
            // SAFETY: AppKit owns the standard button and its superview while the window exists.
            let Some(parent) = (unsafe { button.superview() }) else {
                continue;
            };
            let button_in_content =
                content_view.convertRect_fromView(button.bounds(), Some(&button));
            let target_in_content = NSPoint::new(
                button_in_content.origin.x + button_in_content.size.width * 0.5,
                target_y,
            );
            let target_in_parent =
                parent.convertPoint_fromView(target_in_content, Some(&content_view));
            let frame = button.frame();
            let origin = NSPoint::new(frame.origin.x, target_in_parent.y - frame.size.height * 0.5);
            button.setFrameOrigin(origin);
        }
    }

    pub fn align_traffic_lights(window: &Window) {
        let Some(ns_win) = ns_window(window) else {
            return;
        };
        align_native_traffic_lights(&ns_win);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        #[test]
        fn traffic_lights_align_to_custom_titlebar_center_in_flipped_content() {
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(600.0, 400.0));
            let target = titlebar_center_y(bounds, true);

            assert_eq!(target, f64::from(ui::constants::TITLE_BAR_HEIGHT) * 0.5);
        }

        #[test]
        fn traffic_lights_align_to_custom_titlebar_center_in_unflipped_content() {
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(600.0, 400.0));
            let target = titlebar_center_y(bounds, false);

            assert_eq!(target, 400.0 - f64::from(ui::constants::TITLE_BAR_HEIGHT) * 0.5);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use winit::window::Window;

    #[allow(unused_variables)]
    pub fn enable_full_size_content(window: &Window) {}

    #[allow(unused_variables)]
    pub fn align_traffic_lights(window: &Window) {}
}

pub(crate) use imp::align_traffic_lights;
pub(crate) use imp::enable_full_size_content;
