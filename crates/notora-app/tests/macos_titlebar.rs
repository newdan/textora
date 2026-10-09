#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/runtime/window_runtime/macos_titlebar.rs"]
mod macos_titlebar;

#[cfg(target_os = "macos")]
fn main() {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use winit::platform::macos::WindowAttributesExtMacOS;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let event_loop = winit::event_loop::EventLoop::new()
        .expect("native titlebar regression must initialize on the main thread");
    let attributes = winit::window::WindowAttributes::default()
        .with_visible(false)
        .with_titlebar_transparent(true)
        .with_title_hidden(true)
        .with_fullsize_content_view(true);
    #[allow(deprecated)]
    let window = event_loop
        .create_window(attributes)
        .expect("native titlebar regression should create a hidden window");
    macos_titlebar::synchronize_pointer_dragging(&window, (0.0, 0.0));
    macos_titlebar::align_traffic_lights(&window);
    let handle = window.window_handle().expect("the hidden window must have an AppKit handle");
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        panic!("macOS regression requires an AppKit window");
    };
    // SAFETY: 窗口在调用期间存活；此无默认测试 harness 的测试运行在 macOS 主线程。
    let view = unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }
        .expect("winit must own a valid content view");
    let native_window = view.window().expect("the view must belong to the hidden window");
    assert!(
        !native_window.isMovable(),
        "native titlebar dragging must not swallow client button presses"
    );
    assert!(!native_window.isMovableByWindowBackground());
    native_window.setMovable(true);
    macos_titlebar::align_traffic_lights(&window);
    assert!(
        native_window.isMovable(),
        "geometry synchronization must not cancel an asynchronous native drag"
    );
    let dpi = window.scale_factor() as f32;
    macos_titlebar::synchronize_pointer_dragging(&window, (250.0 * dpi, 18.0 * dpi));
    assert!(native_window.isMovable(), "blank titlebar must enable native dragging");
    macos_titlebar::synchronize_pointer_dragging(&window, (94.0 * dpi, 18.0 * dpi));
    assert!(
        !native_window.isMovable(),
        "returning to the sidebar toggle must restore client titlebar input"
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
