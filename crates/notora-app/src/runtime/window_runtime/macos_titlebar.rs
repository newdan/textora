use objc2::rc::Retained;
use objc2_app_kit::{NSEvent, NSView, NSWindow, NSWindowButton};
use objc2_foundation::{NSPoint, NSRect};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit 的窗口在调用期间存活，NSView 指针有效；调用发生在 UI 线程。
    unsafe {
        let view = Retained::<NSView>::retain(handle.ns_view.as_ptr().cast())?;
        view.window()
    }
}

fn title_center_y(content_bounds: NSRect, content_is_flipped: bool) -> f64 {
    let half_height = f64::from(ui::window_frame::WindowFrameState::MacOs.title_height(1.0)) * 0.5;
    if content_is_flipped {
        content_bounds.origin.y + half_height
    } else {
        content_bounds.origin.y + content_bounds.size.height - half_height
    }
}

pub(super) fn drag_window(window: &Window) -> Result<(), winit::error::ExternalError> {
    let Some(native_window) = native_window(window) else {
        return window.drag_window();
    };
    native_window.setMovable(true);
    // AppKit 立即返回，原生拖动仍在进行；由下一次无按键指针更新结束移动状态。
    window.drag_window()
}

pub(super) fn synchronize_pointer_dragging(window: &Window, pointer_position: (f32, f32)) {
    if NSEvent::pressedMouseButtons() != 0 {
        return;
    }
    let Some(native_window) = native_window(window) else {
        return;
    };
    let size = window.inner_size();
    let mut frame = ui::window_frame::WindowFrameWidget::default();
    frame.set_input(
        ui::window_frame::WindowFrameInput {
            state: ui::window_frame::WindowFrameState::MacOs,
            navigation_toggle: ui::window_frame::WindowFrameNavigationToggle::Enabled,
            ..Default::default()
        },
        ui::Rect::new(0.0, 0.0, size.width as f32, size.height as f32),
        window.scale_factor() as f32,
    );
    native_window.setMovable(
        window.fullscreen().is_none()
            && frame.is_title_drag_position(pointer_position.0, pointer_position.1),
    );
}

pub(super) fn align_traffic_lights(window: &Window) {
    if window.fullscreen().is_some() {
        return;
    }
    let Some(native_window) = native_window(window) else {
        return;
    };
    let Some(content_view) = native_window.contentView() else {
        return;
    };
    let center_y = title_center_y(content_view.bounds(), content_view.isFlipped());
    for kind in
        [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
    {
        let Some(button) = native_window.standardWindowButton(kind) else {
            continue;
        };
        // SAFETY: AppKit 在当前 UI 线程持有标准按钮及父视图。
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
    #[test]
    fn traffic_lights_align_to_title_center_in_logical_coordinates() {
        use super::*;
        use objc2_foundation::NSSize;

        let height = f64::from(ui::window_frame::WindowFrameState::MacOs.title_height(1.0));
        for origin_y in [0.0, 30.0] {
            for content_height in [400.0, 800.0] {
                let bounds =
                    NSRect::new(NSPoint::new(0.0, origin_y), NSSize::new(1200.0, content_height));
                assert_eq!(title_center_y(bounds, true), origin_y + height * 0.5);
                assert_eq!(title_center_y(bounds, false), origin_y + content_height - height * 0.5);
            }
        }
    }
}
