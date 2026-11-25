use crate::{App, AppEvent, Edition, headless_init, parse_args};
use winit::event_loop::EventLoop;

pub fn launch(edition: Edition) {
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_args(&args);

    if cli.headless {
        println!("{} running in headless mode", edition.name());
        match pollster::block_on(headless_init()) {
            Ok(adapter) => println!("GPU initialized: {adapter}"),
            Err(e) => {
                eprintln!("headless init failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let home = crate::product_paths::home_directory();
    let paths = crate::product_paths::ProductPaths::textora(&home);
    if let Err(error) = paths.migrate_legacy_directory(&home) {
        eprintln!("[settings] failed to migrate Textora configuration: {error}");
    }
    let mut app = App::new_for_edition(cli.file, edition);

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
    if let Err(error) = crate::install_macos_open_document_handler(app.open_document_sender()) {
        eprintln!("failed to install macOS open-document handler: {error}");
        std::process::exit(1);
    }
    app.set_event_loop_proxy(event_loop_proxy);
    event_loop.run_app(&mut app).expect("event loop error");
}
