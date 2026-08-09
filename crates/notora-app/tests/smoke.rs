use notora_app::action::NotoraAction;
use notora_app::{FocusTarget, NotoraApp, NotoraPaths};
use std::thread;
use std::time::{Duration, Instant};

fn app() -> NotoraApp {
    let directory = tempfile::tempdir().expect("test should create a temporary directory");
    let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
        .expect("test should create isolated product paths");
    NotoraApp::with_paths(paths).expect("notora app should construct without a window")
}

#[test]
fn binary_title_and_three_pane_editor_rect_are_product_specific() {
    let manifest = include_str!("../Cargo.toml");
    let app_source = include_str!("../src/app.rs");
    let layout = app().shell_layout();

    assert!(manifest.contains("name = \"notora\""));
    assert!(app_source.contains("with_title(\"notora\")"));
    assert!(layout.editor_rect.x > 0.0);
    assert!(layout.navigation_rect.right() <= layout.editor_rect.x);
    assert!(layout.card_list_rect.right() <= layout.editor_rect.x);
}

#[test]
fn non_editor_focus_and_modal_block_document_ime() {
    let directory = tempfile::tempdir().expect("external fixture directory should exist");
    let path = directory.path().join("ime.md");
    std::fs::write(&path, "# 输入法").expect("external fixture should be written");
    let mut app = app();
    app.receive_system_open_paths(vec![path]);
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.editor_runtime_tab_count() == 0 {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    }

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    assert!(app.update_editor_preedit("编辑器".to_owned(), Some((0, 3))));

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
    assert!(!app.update_editor_preedit("卡片".to_owned(), Some((0, 2))));

    app.dispatch_action(NotoraAction::OpenSettings);
    assert!(!app.update_editor_preedit("设置".to_owned(), Some((0, 2))));
}

#[test]
fn modal_new_document_menu_and_tooltip_are_painted_after_the_editor() {
    let source = include_str!("../src/render.rs");
    let editor_position = source
        .find("frame.paint_editor_with")
        .expect("shell should paint the editor through EditorFrame");
    let overlay_source = &source[editor_position..];
    let modal_position = overlay_source
        .find("if model.show_settings_overlay")
        .expect("shell should paint a modal overlay after the editor");
    let new_document_menu_position = overlay_source
        .find("if let Some(menu) = self.new_document_menu.as_ref()")
        .expect("shell should paint the new document menu after the editor");
    let tooltip_position = overlay_source
        .find("if model.show_tooltip")
        .expect("shell should paint a tooltip layer after the editor");

    assert!(modal_position < new_document_menu_position);
    assert!(new_document_menu_position < tooltip_position);
}

#[test]
fn session_restore_is_scheduled_only_after_the_first_presented_frame() {
    let app_source = include_str!("../src/app.rs");
    let events_source = include_str!("../src/events.rs");
    let resume_start = app_source.find("pub(crate) fn resume").expect("resume should exist");
    let resume_end = app_source[resume_start..]
        .find("pub(crate) fn resize_window")
        .map(|offset| resume_start + offset)
        .expect("resume should end before resize_window");

    assert!(!app_source[resume_start..resume_end].contains("restore_pending_session"));
    let redraw_start =
        events_source.find("WindowEvent::RedrawRequested").expect("redraw handling should exist");
    let redraw_source = &events_source[redraw_start..];
    let render_position = redraw_source.find("self.render()").expect("redraw should render");
    let restore_position = redraw_source
        .find("self.restore_session_after_first_frame()")
        .expect("redraw should schedule session restore");
    assert!(render_position < restore_position);
}

#[test]
fn gpu_preparation_starts_before_event_loop_construction() {
    let main_source = include_str!("../src/main.rs");
    let app_construction = main_source
        .find("NotoraApp::try_new()")
        .expect("main should construct the notora application");
    let event_loop_construction =
        main_source.find("let event_loop =").expect("main should construct the event loop");

    assert!(app_construction < event_loop_construction);
}

#[test]
fn system_open_entry_reuses_an_external_file_session_without_a_duplicate_tab() {
    let directory = tempfile::tempdir().expect("external fixture directory should exist");
    let path = directory.path().join("shared.txt");
    std::fs::write(&path, "shared external document").expect("external fixture should be written");
    let mut app = app();

    app.receive_system_open_paths(vec![path.clone()]);
    app.receive_system_open_paths(vec![path]);

    let deadline = Instant::now() + Duration::from_secs(2);
    while app.editor_runtime_tab_count() == 0 {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.editor_runtime_tab_count(), 1);
    assert_eq!(app.state().external_files.sessions().len(), 1);
}
