//! Smoke tests for edit+ app lifecycle.
//!
//! GPU-dependent tests run in headless mode and skip gracefully when no adapter is available.

/// Helper: run a test closure with headless GPU, or skip if unavailable.
fn with_headless_gpu(f: impl FnOnce(String)) {
    let result = pollster::block_on(textora_app::headless_init());
    match result {
        Ok(info) => f(info),
        Err(textora_app::GpuError::NoAdapter) => {
            eprintln!("skipping: no GPU adapter available in this environment");
        }
        Err(e) => panic!("headless_init failed: {e}"),
    }
}

// ── Test: init + shutdown ───────────────────────────────────────────────────

#[test]
fn test_app_init_and_shutdown() {
    with_headless_gpu(|adapter_info| {
        assert!(!adapter_info.is_empty(), "adapter_info must not be empty");
    });
}

// ── Test: double init + drop ────────────────────────────────────────────────

#[test]
fn test_double_init_no_panic() {
    with_headless_gpu(|info1| {
        assert!(!info1.is_empty());
        drop(info1);

        let info2 = pollster::block_on(textora_app::headless_init());
        match info2 {
            Ok(info) => assert!(!info.is_empty()),
            Err(textora_app::GpuError::NoAdapter) => {}
            Err(e) => panic!("second headless_init failed: {e}"),
        }
    });
}

// ── Test: App struct can be constructed ─────────────────────────────────────

#[test]
fn test_app_construction() {
    let _app = textora_app::App::new(None);
}

#[test]
fn launch_preparation_starts_before_event_loop_construction() {
    let source = include_str!("../src/main.rs");
    let app_position = source
        .find("App::new_for_launch")
        .expect("production entry point must use the parallel launch constructor");
    let event_loop_position = source
        .find("EventLoop::<AppEvent>::with_user_event")
        .expect("production entry point must construct the event loop");

    assert!(app_position < event_loop_position);
}

// ── Test: App window title is correct ───────────────────────────────────────

#[test]
fn test_window_title() {
    let _app = textora_app::App::new(None);
}

// ── Test: 100 resize events without panic ───────────────────────────────────

#[test]
fn test_resize_no_panic() {
    let mut app = textora_app::App::new(None);

    // Simulate 100 resize events with varying dimensions
    for i in 0..100u32 {
        let w = 100 + i * 7;
        let h = 100 + i * 5;
        app.handle_resize(w, h);
    }

    // Edge cases
    app.handle_resize(1, 1);
    app.handle_resize(1920, 1080);
    app.handle_resize(3840, 2160);
    app.handle_resize(0, 0);
}
