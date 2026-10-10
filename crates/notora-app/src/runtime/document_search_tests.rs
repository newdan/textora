use super::tests::{app, install_registered_note};
use super::*;
use crate::FocusTarget;
use ui::{KeyCode, Modifiers};

const PRIMARY: Modifiers = Modifiers { cmd: true, ..Modifiers::NONE };

#[test]
fn find_shortcut_opens_current_document_search_instead_of_library_search() {
    let mut runtime = app();
    let (_, tab_id) = install_registered_note(&mut runtime, "find.md", "中文正文 中文");
    runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    runtime.handle_key_input(KeyCode::Char('f'), PRIMARY, None);

    let tab = runtime
        .document_runtime
        .editor()
        .tab_session(tab_id)
        .expect("find fixture should retain its document");
    assert!(tab.search_state().panel_visible, "find should open the current document search");
    assert_ne!(runtime.state().layout.focus_target, FocusTarget::NavigationSearch);
}

#[test]
fn search_ime_input_never_edits_document_and_escape_restores_editor() {
    let mut runtime = app();
    let (_, tab_id) = install_registered_note(&mut runtime, "find.md", "中文正文 中文");
    let original_layout = runtime.shell_layout();
    runtime.handle_key_input(KeyCode::Char('f'), PRIMARY, None);
    assert_eq!(runtime.state().layout.focus_target, FocusTarget::DocumentSearch);
    let search_layout = runtime.shell_layout();
    assert!(search_layout.editor_body_rect.y > original_layout.editor_body_rect.y);
    assert_eq!(search_layout.editor_body_rect.bottom(), original_layout.editor_body_rect.bottom());
    runtime.handle_ime_input(winit::event::Ime::Preedit("中文".to_owned(), Some((0, 6))));
    assert!(search_state(&runtime).query.is_empty());
    runtime.handle_ime_input(winit::event::Ime::Commit("中文".to_owned()));
    assert_eq!(search_state(&runtime).match_count(), 2);
    runtime.handle_key_input(KeyCode::Enter, Modifiers::NONE, None);
    assert_eq!(search_state(&runtime).active_match_idx, 1);
    runtime.handle_key_input(KeyCode::Enter, Modifiers { shift: true, ..Modifiers::NONE }, None);
    assert_eq!(search_state(&runtime).active_match_idx, 0);
    assert_eq!(
        runtime
            .document_runtime
            .editor()
            .document_text_snapshot(tab_id)
            .expect("search retains source")
            .text,
        "中文正文 中文"
    );
    runtime.handle_key_input(KeyCode::Escape, Modifiers::NONE, None);
    assert_eq!(runtime.state().layout.focus_target, FocusTarget::Editor);
    assert!(!search_state(&runtime).panel_visible);
    assert_eq!(runtime.shell_layout().editor_body_rect, original_layout.editor_body_rect);
}

#[test]
fn library_search_and_document_queries_are_independent() {
    let mut runtime = app();
    let (_, first_tab) = install_registered_note(&mut runtime, "first.md", "正文正文");
    runtime.handle_key_input(KeyCode::Char('f'), PRIMARY, None);
    runtime.handle_ime_input(winit::event::Ime::Commit("正文".to_owned()));
    runtime.handle_key_input(KeyCode::Char('f'), Modifiers { shift: true, ..PRIMARY }, None);
    assert_eq!(runtime.state().layout.focus_target, FocusTarget::NavigationSearch);
    runtime.handle_ime_input(winit::event::Ime::Commit("笔记库".to_owned()));
    assert_eq!(runtime.state().library.search_text, "笔记库");
    assert_eq!(search_state(&runtime).query, "正文");
    let (_, second_tab) = install_registered_note(&mut runtime, "second.md", "新文档");
    runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    runtime.handle_key_input(KeyCode::Char('f'), PRIMARY, None);
    assert!(search_state(&runtime).query.is_empty());
    runtime.handle_ime_input(winit::event::Ime::Commit("新".to_owned()));
    assert_eq!(search_state(&runtime).match_count(), 1);
    assert_eq!(
        runtime
            .document_runtime
            .editor()
            .tab_session(first_tab)
            .expect("first document remains loaded")
            .search_state()
            .query,
        "正文"
    );
    assert_eq!(
        runtime
            .document_runtime
            .editor()
            .tab_session(second_tab)
            .expect("second document remains loaded")
            .document
            .full_text(),
        "新文档"
    );
}

#[test]
fn physical_find_and_read_only_documents_allow_search_without_edits() {
    let mut runtime = app();
    let (_, tab_id) = install_registered_note(&mut runtime, "readonly.txt", "中文正文 中文");
    runtime
        .document_runtime
        .editor_mut()
        .tab_session_mut(tab_id)
        .expect("read-only fixture has document")
        .runtime
        .set_editing_access(appkit_shell::tab_runtime::DocumentEditingAccess::ReadOnly);
    runtime.handle_key_input(
        KeyCode::Char('ш'),
        PRIMARY,
        Some(winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyF)),
    );
    assert_eq!(runtime.state().layout.focus_target, FocusTarget::DocumentSearch);
    runtime.handle_ime_input(winit::event::Ime::Commit("中文".to_owned()));
    assert_eq!(search_state(&runtime).match_count(), 2);
    assert!(
        !runtime
            .document_runtime
            .editor()
            .document_summary(tab_id)
            .expect("read-only document remains loaded")
            .dirty
    );
}

#[test]
fn toolbar_search_request_opens_current_document() {
    let mut runtime = app();
    install_registered_note(&mut runtime, "toolbar.md", "正文");
    runtime.dispatch_action(NotoraAction::DocumentSearchRequested);
    assert_eq!(runtime.state().layout.focus_target, FocusTarget::DocumentSearch);
}

#[test]
fn editor_popup_receives_escape_before_the_search_bar() {
    let mut runtime = app();
    install_registered_note(&mut runtime, "popup.md", "正文");
    runtime.handle_key_input(KeyCode::Char('f'), PRIMARY, None);
    let mut model = crate::render::NotoraRenderModel::from_state_and_settings(
        runtime.state(),
        &crate::ProductSettings::default(),
    );
    model.editor_chrome.location.open = true;
    runtime.frame_runtime.shell.update_model(&model);
    runtime.handle_key_input(KeyCode::Escape, Modifiers::NONE, None);
    assert!(search_state(&runtime).panel_visible, "Escape must dismiss the popup first");
}

#[test]
fn escape_from_editor_closes_a_visible_document_search() {
    let mut runtime = app();
    install_registered_note(&mut runtime, "escape.md", "正文");
    runtime.handle_key_input(KeyCode::Char('f'), PRIMARY, None);
    runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    runtime.handle_key_input(KeyCode::Escape, Modifiers::NONE, None);
    assert!(!search_state(&runtime).panel_visible);
    assert_eq!(runtime.state().layout.focus_target, FocusTarget::Editor);
}

fn search_state(runtime: &NotoraRuntime) -> appkit_core::document::SearchState {
    let editor = runtime.document_runtime.editor();
    editor
        .tab_session(editor.active_tab_id().expect("fixture has active document"))
        .expect("active document remains loaded")
        .search_state()
        .clone()
}
