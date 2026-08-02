use notora_app::{NotoraApp, NotoraPaths, WorkspaceCommand};
use std::thread;
use std::time::{Duration, Instant};

fn app() -> NotoraApp {
    let directory = tempfile::tempdir().expect("test should create a temporary directory");
    let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
        .expect("test should create isolated product paths");
    NotoraApp::with_paths(paths).expect("notora app should construct without a window")
}

#[test]
fn opening_supported_external_text_formats_preserves_promoted_tabs() {
    let directory = tempfile::tempdir().expect("external fixture directory should exist");
    let paths = [
        directory.path().join("plain.txt"),
        directory.path().join("document.md"),
        directory.path().join("diagram.mmap.md"),
    ];
    for path in &paths {
        std::fs::write(path, "# External").expect("external fixture should be written");
    }
    let mut app = app();

    for (expected_tab_count, path) in paths.into_iter().enumerate() {
        app.receive_system_open_paths(vec![path]);
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.editor_runtime_tab_count() <= expected_tab_count {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "external preview should install promptly");
            thread::sleep(Duration::from_millis(10));
        }
        app.request_preview_promotion();
    }

    assert_eq!(app.state().external_files.sessions().len(), 3);
    assert_eq!(app.editor_runtime_tab_count(), 3);
    assert_eq!(app.state().library.last_command_error, None);
}

#[test]
fn opening_a_large_workspace_without_selecting_cards_creates_no_runtime_documents() {
    const NOTE_COUNT: usize = 10_000;

    let workspace = tempfile::tempdir().expect("workspace fixture directory should exist");
    for index in 0..NOTE_COUNT {
        std::fs::write(workspace.path().join(format!("note-{index}.md")), "# Note")
            .expect("workspace note fixture should be written");
    }
    let mut app = app();

    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace.path().to_path_buf(),
    })
    .expect("workspace should open");

    assert_eq!(app.editor_runtime_tab_count(), 0);
    assert_eq!(app.state().library.selected_card, None);
}
