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
    let runtime_source = include_str!("../src/runtime.rs");
    let layout = app().shell_layout();

    assert!(manifest.contains("name = \"notora\""));
    assert!(runtime_source.contains("with_title(\"notora\")"));
    assert!(layout.editor_rect.x > 0.0);
    assert!(layout.navigation_rect.right() <= layout.editor_rect.x);
    assert!(layout.card_list_rect.right() <= layout.editor_rect.x);
}

#[test]
fn notora_app_remains_a_thin_composition_root() {
    let app_source = include_str!("../src/app.rs");
    let events_source = include_str!("../src/events.rs");
    let adapter_start = events_source
        .find("impl ApplicationHandler<ShellEvent> for NotoraApp")
        .expect("NotoraApp should remain the winit adapter");
    let runtime_start = events_source[adapter_start..]
        .find("impl NotoraRuntime")
        .map(|offset| adapter_start + offset)
        .expect("event implementation should belong to NotoraRuntime");
    let application_handler = &events_source[adapter_start..runtime_start];

    assert!(app_source.contains("runtime: NotoraRuntime"));
    assert!(!app_source.contains("NotoraProductEvent"));
    assert!(!app_source.contains("impl NotoraEffectService"));
    assert!(!application_handler.contains("match event"));
    assert!(!application_handler.contains("WindowEvent::"));
    assert_eq!(application_handler.matches("runtime_mut()").count(), 4);
}

#[test]
fn runtime_state_is_owned_by_named_components() {
    let runtime_source = include_str!("../src/runtime.rs");
    let document_runtime_source = include_str!("../src/runtime/document_runtime.rs");
    let document_command_executor_source =
        include_str!("../src/runtime/document_command_executor.rs");
    let notora_effect_executor_source = include_str!("../src/runtime/notora_effect_executor.rs");
    let frame_runtime_source = include_str!("../src/runtime/frame_runtime.rs");
    let action_runtime_source = include_str!("../src/runtime/action_runtime.rs");
    let persistence_runtime_source = include_str!("../src/runtime/persistence_runtime.rs");
    let product_coordinator_source = include_str!("../src/app/product_event_coordinator.rs");
    let document_interpreter_source = include_str!("../src/app/document_completion_interpreter.rs");
    let persistence_interpreter_source =
        include_str!("../src/app/persistence_completion_interpreter.rs");
    let workspace_interpreter_source =
        include_str!("../src/app/workspace_completion_interpreter.rs");
    let deadline_coordinator_source = include_str!("../src/app/deadline_coordinator.rs");
    let effect_executor_source = include_str!("../src/effect_executor.rs");
    let runtime_fields_start = runtime_source
        .find("pub(crate) struct NotoraRuntime")
        .expect("runtime struct should exist");
    let runtime_fields_end = runtime_source[runtime_fields_start..]
        .find("\n}\n\nimpl NotoraRuntime")
        .map(|offset| runtime_fields_start + offset)
        .expect("runtime fields should end before its implementation");
    let runtime_fields = &runtime_source[runtime_fields_start..runtime_fields_end];

    for component in [
        "action_runtime: ActionRuntime",
        "document_runtime: DocumentRuntime",
        "persistence_runtime: PersistenceRuntime",
        "frame_runtime: FrameRuntime",
        "window_runtime: WindowRuntime",
    ] {
        assert!(runtime_fields.contains(component), "runtime should own {component}");
    }
    assert!(!runtime_fields.contains("pending_"));
    assert!(!runtime_fields.contains("HashMap<"));
    assert!(document_runtime_source.contains("pending_conflict_retries:"));
    assert!(document_runtime_source.contains("pending_metadata_mutations:"));
    assert!(!product_coordinator_source.contains("NotoraRuntime"));
    assert!(!document_interpreter_source.contains("NotoraRuntime"));
    assert!(!persistence_interpreter_source.contains("NotoraRuntime"));
    assert!(!workspace_interpreter_source.contains("NotoraRuntime"));
    assert!(!document_command_executor_source.contains("NotoraRuntime"));
    assert!(!notora_effect_executor_source.contains("NotoraRuntime"));
    assert!(!deadline_coordinator_source.contains("NotoraRuntime"));
    assert!(!runtime_source.contains("fn apply_product_event"));
    assert!(!runtime_source.contains("DocumentCommand::"));
    assert!(!runtime_source.contains("NotoraEffect::"));
    assert!(!runtime_source.contains("impl NotoraEffectService for NotoraRuntime"));
    assert!(!runtime_source.contains("impl ShellEffectTarget for NotoraRuntime"));
    assert!(!runtime_source.contains("wgpu::RenderPassDescriptor"));
    assert!(!runtime_source.contains("create_buffer(&wgpu::BufferDescriptor"));
    assert!(!frame_runtime_source.contains("&mut NotoraState"));
    assert!(!effect_executor_source.contains("dispatch_action"));
    for private_state in [
        "#[cfg(not(test))]\n    state: NotoraState",
        "#[cfg(not(test))]\n    editor_runtime: EditorRuntime",
        "#[cfg(not(test))]\n    shell: NotoraShell",
        "#[cfg(not(test))]\n    product_settings: ProductSettings",
    ] {
        assert!(
            action_runtime_source.contains(private_state)
                || document_runtime_source.contains(private_state)
                || frame_runtime_source.contains(private_state)
                || persistence_runtime_source.contains(private_state),
            "production component state should remain private: {private_state}"
        );
    }
}

#[test]
fn product_events_use_the_shared_runtime_and_domain_completions() {
    let product_source = include_str!("../src/product.rs");
    let action_runtime_source = include_str!("../src/runtime/action_runtime.rs");
    let coordinator_source = include_str!("../src/app/product_event_coordinator.rs");

    assert!(product_source.contains("ProductEventInbox<NotoraProductEvent>"));
    assert!(product_source.contains("Workspace(WorkspaceCompletionEnvelope)"));
    assert!(product_source.contains("Document(DocumentCompletion)"));
    assert!(product_source.contains("Persistence(PersistenceCompletion)"));
    assert!(!product_source.contains("std::sync::mpsc"));
    assert!(action_runtime_source.contains("use appkit_shell::{DrainStart, EventPump}"));
    assert!(coordinator_source.contains("trait WorkspaceCompletionTarget"));
    assert!(coordinator_source.contains("trait DocumentCompletionTarget"));
    assert!(coordinator_source.contains("trait PersistenceCompletionTarget"));
    assert!(!coordinator_source.contains("WorkspaceCompletion::"));
    assert!(!coordinator_source.contains("DocumentCompletion::"));
    assert!(!coordinator_source.contains("PersistenceCompletion::"));
    assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/event_pump.rs").exists());
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
    let runtime_source = include_str!("../src/runtime.rs");
    let events_source = include_str!("../src/events.rs");
    let resume_start = runtime_source.find("pub(crate) fn resume").expect("resume should exist");
    let resume_end = runtime_source[resume_start..]
        .find("pub(crate) fn resize_window")
        .map(|offset| resume_start + offset)
        .expect("resume should end before resize_window");

    assert!(!runtime_source[resume_start..resume_end].contains("restore_pending_session"));
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
fn shutdown_flushes_state_before_stopping_background_workers() {
    let runtime_source = include_str!("../src/runtime.rs");
    let shutdown_start =
        runtime_source.find("pub(crate) fn shutdown").expect("runtime shutdown should exist");
    let shutdown_end = runtime_source[shutdown_start..]
        .find("fn finish_saves_and_snapshot_dirty_documents")
        .map(|offset| shutdown_start + offset)
        .expect("shutdown should end before its save helper");
    let shutdown_source = &runtime_source[shutdown_start..shutdown_end];

    let finish_saves = shutdown_source
        .find("self.finish_saves_and_snapshot_dirty_documents()")
        .expect("shutdown should settle saves before stopping workers");
    let flush_catalog = shutdown_source
        .find("self.flush_pending_catalog_backup()")
        .expect("shutdown should flush the catalog backup");
    let save_session = shutdown_source
        .find(".save_session(")
        .expect("shutdown should enqueue the final session snapshot");
    let save_settings = shutdown_source
        .find(".save_settings(")
        .expect("shutdown should enqueue the final settings snapshot");
    let stop_persistence = shutdown_source
        .find("self.persistence_runtime.shutdown()")
        .expect("shutdown should stop the persistence worker");
    let stop_product = shutdown_source
        .find("ProductHost::shutdown(&mut self.product)")
        .expect("shutdown should stop product services");
    let stop_editor = shutdown_source
        .find("self.document_runtime.editor_mut().shutdown()")
        .expect("shutdown should stop the editor runtime");

    assert!(finish_saves < flush_catalog);
    assert!(flush_catalog < save_session);
    assert!(save_session < save_settings);
    assert!(save_settings < stop_persistence);
    assert!(stop_persistence < stop_product);
    assert!(stop_product < stop_editor);
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
