use notora_app::action::NotoraAction;
use notora_app::{FocusTarget, NotoraApp, NotoraPaths};

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
    let mut app = app();
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    assert!(app.update_editor_preedit("编辑器".to_owned(), Some((0, 3))));

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
    assert!(!app.update_editor_preedit("卡片".to_owned(), Some((0, 2))));

    app.dispatch_action(NotoraAction::OpenSettings);
    assert!(!app.update_editor_preedit("设置".to_owned(), Some((0, 2))));
}

#[test]
fn modal_menu_and_tooltip_are_painted_after_the_editor() {
    let source = include_str!("../src/render.rs");
    let editor_position = source
        .find("frame.paint_editor_with")
        .expect("shell should paint the editor through EditorFrame");
    let modal_position =
        source.find("if model.show_settings_overlay").expect("shell should paint a modal overlay");
    let menu_position = source.find("if model.show_menu").expect("shell should paint a menu layer");
    let tooltip_position =
        source.find("if model.show_tooltip").expect("shell should paint a tooltip layer");

    assert!(editor_position < modal_position);
    assert!(modal_position < menu_position);
    assert!(menu_position < tooltip_position);
}
