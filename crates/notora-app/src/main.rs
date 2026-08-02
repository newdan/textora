fn main() {
    let mut app = match notora_app::NotoraApp::try_new() {
        Ok(app) => app,
        Err(error) => {
            eprintln!("notora initialization failed: {error}");
            std::process::exit(1);
        }
    };
    let event_loop = winit::event_loop::EventLoop::<appkit_shell::ShellEvent>::with_user_event()
        .build()
        .expect("notora event loop must be constructible");
    app.set_event_loop_proxy(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("notora event loop failed");
}
