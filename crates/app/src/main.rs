// macOS native UI bindings (AppKit/winit); library crates remain cross-platform.
#[cfg(not(target_os = "macos"))]
compile_error!(
    "The Edit+ application (NoteR binary) currently supports macOS only; library crates remain portable."
);

#[cfg(target_os = "macos")]
use appkit_shell::ProductWakeHandle;
use textora_app::{App, AppEvent, headless_init, parse_args};
use winit::event_loop::EventLoop;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_args(&args);

    if cli.headless {
        println!("edit+ running in headless mode");
        match pollster::block_on(headless_init()) {
            Ok(adapter) => println!("GPU initialized: {adapter}"),
            Err(e) => {
                eprintln!("headless init failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mut app = App::new_for_launch(cli.file);

    #[cfg(target_os = "macos")]
    let event_loop = {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        EventLoop::<AppEvent>::with_user_event()
            .with_default_menu(false)
            .build()
            .expect("failed to create event loop")
    };
    #[cfg(not(target_os = "macos"))]
    let event_loop =
        EventLoop::<AppEvent>::with_user_event().build().expect("failed to create event loop");

    let event_loop_proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    if let Err(error) = textora_app::install_macos_open_document_handler(
        ProductWakeHandle::new(event_loop_proxy.clone()),
        app.open_document_sender(),
    ) {
        eprintln!("failed to install macOS open-document handler: {error}");
        std::process::exit(1);
    }
    app.set_event_loop_proxy(event_loop_proxy);
    event_loop.run_app(&mut app).expect("event loop error");
}
