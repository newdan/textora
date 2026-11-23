use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Returns the AppKit window backing a winit window on the main thread.
pub fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    MainThreadMarker::new()?;
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: The borrowed winit window keeps its AppKit view alive on the main thread.
    unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast())?.window() }
}

fn titlebar_center_y(
    content_bounds: NSRect,
    content_is_flipped: bool,
    titlebar_height: f64,
) -> f64 {
    let half_height = titlebar_height * 0.5;
    if content_is_flipped {
        content_bounds.origin.y + half_height
    } else {
        content_bounds.origin.y + content_bounds.size.height - half_height
    }
}

/// Centers the standard macOS window buttons vertically in a titlebar measured in points.
pub fn align_traffic_lights(window: &NSWindow, titlebar_height: f64) {
    let Some(content_view) = window.contentView() else {
        return;
    };
    let center_y =
        titlebar_center_y(content_view.bounds(), content_view.isFlipped(), titlebar_height);
    for kind in
        [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
    {
        let Some(button) = window.standardWindowButton(kind) else {
            continue;
        };
        // SAFETY: AppKit owns the standard button and its superview on the main thread.
        let Some(parent) = (unsafe { button.superview() }) else {
            continue;
        };
        let button_in_content = content_view.convertRect_fromView(button.bounds(), Some(&button));
        let center_in_parent = parent.convertPoint_fromView(
            NSPoint::new(button_in_content.origin.x + button_in_content.size.width * 0.5, center_y),
            Some(&content_view),
        );
        let frame = button.frame();
        let origin = NSPoint::new(frame.origin.x, center_in_parent.y - frame.size.height * 0.5);
        if frame.origin != origin {
            button.setFrameOrigin(origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSSize;

    #[test]
    fn traffic_lights_align_to_title_center_in_logical_coordinates() {
        for height in [28.0, 36.0] {
            for origin_y in [0.0, 30.0] {
                let content_height = 400.0;
                let bounds =
                    NSRect::new(NSPoint::new(0.0, origin_y), NSSize::new(1200.0, content_height));
                assert_eq!(titlebar_center_y(bounds, true, height), origin_y + height * 0.5);
                assert_eq!(
                    titlebar_center_y(bounds, false, height),
                    origin_y + content_height - height * 0.5
                );
            }
        }
    }
}
