//! Textora's transparent native titlebar and full-size content view.

use winit::window::Window;

#[cfg(target_os = "macos")]
use appkit_shell::macos::{align_traffic_lights as align_native_traffic_lights, native_window};

#[cfg(target_os = "macos")]
pub(crate) fn enable_full_size_content(window: &Window) {
    use objc2_app_kit::{NSWindowStyleMask, NSWindowTitleVisibility};

    let Some(native_window) = native_window(window) else {
        return;
    };
    let mut style = native_window.styleMask();
    let fullscreen = style.contains(NSWindowStyleMask::FullScreen);
    if native_window.titlebarAppearsTransparent() == fullscreen {
        native_window.setTitlebarAppearsTransparent(!fullscreen);
    }
    if fullscreen {
        if !native_window.isOpaque() {
            native_window.setOpaque(true);
        }
        return;
    }
    native_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    if !style.contains(NSWindowStyleMask::FullSizeContentView) {
        style.insert(NSWindowStyleMask::FullSizeContentView);
        native_window.setStyleMask(style);
    }
    align_native_traffic_lights(&native_window, f64::from(ui::constants::TITLE_BAR_HEIGHT));
}

#[cfg(target_os = "macos")]
pub(crate) fn align_traffic_lights(window: &Window) {
    let Some(native_window) = native_window(window) else {
        return;
    };
    align_native_traffic_lights(&native_window, f64::from(ui::constants::TITLE_BAR_HEIGHT));
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn enable_full_size_content(_window: &Window) {}

#[cfg(not(target_os = "macos"))]
pub(crate) fn align_traffic_lights(_window: &Window) {}
