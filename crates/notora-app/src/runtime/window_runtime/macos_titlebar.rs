use appkit_shell::macos::{align_traffic_lights as align_native_traffic_lights, native_window};
use objc2_app_kit::NSEvent;
use winit::window::Window;

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
    let titlebar_height = ui::window_frame::WindowFrameState::MacOs.title_height(1.0);
    align_native_traffic_lights(&native_window, f64::from(titlebar_height));
}
