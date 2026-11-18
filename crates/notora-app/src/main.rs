#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    let mut app = match notora_app::NotoraApp::try_new() {
        Ok(app) => app,
        Err(error) => {
            eprintln!("notora initialization failed: {error}");
            std::process::exit(1);
        }
    };
    let cli_args: Vec<String> = std::env::args().collect();
    let cli_paths = notora_app::parse_cli_open_paths(&cli_args);
    if !cli_paths.is_empty() {
        app.receive_system_open_paths(cli_paths);
    }
    let event_loop = winit::event_loop::EventLoop::<appkit_shell::ShellEvent>::with_user_event()
        .build()
        .expect("notora event loop must be constructible");
    #[cfg(target_os = "macos")]
    if let Err(error) = notora_app::install_macos_open_document_handler(app.open_document_sender())
    {
        eprintln!("failed to install macOS open-document handler: {error}");
        std::process::exit(1);
    }
    app.set_event_loop_proxy(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("notora event loop failed");
}
