#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/runtime/window_runtime/macos_titlebar.rs"]
mod macos_titlebar;

#[cfg(target_os = "macos")]
fn main() {
    use appkit_shell::macos::native_window;
    use winit::platform::macos::WindowAttributesExtMacOS;

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
    let dpi = window.scale_factor() as f32;
    macos_titlebar::synchronize_pointer_dragging(&window, (94.0 * dpi, 18.0 * dpi));
    macos_titlebar::align_traffic_lights(&window);
    let native_window =
        native_window(&window).expect("the hidden window must have an AppKit window");
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
    macos_titlebar::synchronize_pointer_dragging(&window, (250.0 * dpi, 18.0 * dpi));
    assert!(native_window.isMovable(), "blank titlebar must enable native dragging");
    macos_titlebar::synchronize_pointer_dragging(&window, (94.0 * dpi, 18.0 * dpi));
    assert!(
        !native_window.isMovable(),
        "returning to the sidebar toggle must restore client titlebar input"
    );
    for (x, y) in [(4.0, 18.0), (40.0, 4.0), (76.0, 18.0)] {
        macos_titlebar::synchronize_pointer_dragging(&window, (x * dpi, y * dpi));
        assert!(
            native_window.isMovable(),
            "blank space around the traffic lights must allow native dragging at ({x}, {y})"
        );
    }
    macos_titlebar::synchronize_pointer_dragging(&window, (40.0 * dpi, 100.0 * dpi));
    assert!(!native_window.isMovable(), "content below the traffic lights must keep client input");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
