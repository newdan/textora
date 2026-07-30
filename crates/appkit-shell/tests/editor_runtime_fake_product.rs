//! 不依赖 textora-app 的最小产品宿主验收。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use appkit_core::document::DocumentModel;
use appkit_core::file_safety::capture_revision;
use appkit_shell::editor_plugin::EditorPluginFactory;
use appkit_shell::editor_runtime::{
    EditorFocus, EditorInputContext, EditorNotification, EditorRuntime, EditorRuntimeConfig,
    OpenDisposition, execute_prepared_save,
};
use appkit_shell::prepared_tab::PreparedTab;
use appkit_shell::tab_runtime::TabRuntime;
use appkit_shell::view_route::ViewRouteTable;
use core::buffer::TextBuffer;
use ui::plugin::PluginFactory;

struct FakeProductDirectory(PathBuf);

impl FakeProductDirectory {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("fake product test clock should be after UNIX epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("textora-fake-product-{suffix}"));
        fs::create_dir_all(&path).expect("fake product directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for FakeProductDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn runtime() -> EditorRuntime {
    let mut registry = ui::plugin::PluginRegistry::new();
    registry.register(Box::new(EditorPluginFactory));
    let routes = ViewRouteTable::new(
        Vec::new(),
        &std::collections::HashSet::from([ui::plugin::PLUGIN_EDITOR]),
    )
    .expect("fake product route table should be valid");
    EditorRuntime::new(EditorRuntimeConfig {
        plugin_registry: registry,
        view_routes: routes,
        initial_settings: ui::Settings::new(),
        initial_theme: ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark()),
        snapshots_directory: PathBuf::from("fake-product-snapshots"),
    })
    .expect("fake product runtime should construct")
}

#[test]
fn fake_product_completes_nonzero_frame_save_and_late_close_lifecycle() {
    let directory = FakeProductDirectory::new();
    let path = directory.path().join("note.txt");
    fs::write(&path, "before").expect("fake product baseline should be written");
    let baseline = capture_revision(&path).expect("fake product baseline should capture");

    let mut text_buffer = TextBuffer::new(false).expect("fake product buffer should construct");
    text_buffer.write_raw(b"before");
    text_buffer.mark_as_clean();
    let mut document = DocumentModel::new(text_buffer);
    document.file_path = Some(path);
    document.disk_revision = Some(baseline);
    document.insert_at_cursor(b" + edit");

    let mut runtime = runtime();
    let editor_context = EditorInputContext {
        editor_rect: ui::Rect::new(72.0, 96.0, 640.0, 420.0),
        focus: EditorFocus::Active,
        modal_blocked: false,
    };
    let inactive_context = EditorInputContext { focus: EditorFocus::Inactive, ..editor_context };
    assert!(!runtime.keyboard_input_allowed(inactive_context));
    assert!(!runtime.update_preedit(inactive_context, "拼".to_owned(), Some((0, 3))));
    assert!(runtime.keyboard_input_allowed(editor_context));
    assert!(runtime.update_preedit(editor_context, "拼".to_owned(), Some((0, 3))));
    assert_eq!(runtime.preedit().0, "拼");
    assert!(!runtime.pointer_input_allowed(editor_context, (24.0, 24.0)));

    let install = runtime.install_prepared_tab(
        PreparedTab::new(document, TabRuntime::new(EditorPluginFactory.create())),
        None,
        OpenDisposition::Persistent,
    );
    let tab_id = runtime.active_tab_id().expect("fake product tab should activate");
    assert!(install.notifications.iter().any(|notification| matches!(
        notification,
        EditorNotification::ActiveDocumentChanged { tab_id: Some(id) } if *id == tab_id
    )));

    let mut frame = runtime.begin_frame().expect("fake product frame should begin");
    let editor_rect = ui::Rect::new(72.0, 96.0, 640.0, 420.0);
    frame.with_layout_context(|context| assert_eq!(context.dpi, 1.0));
    frame.with_paint_context(|context| {
        context.list.fill(ui::Rect::new(0.0, 0.0, 72.0, 600.0), [0.0; 4]);
    });
    frame.paint_editor(editor_rect).expect("nonzero editor rect should paint");
    frame.with_paint_context(|context| {
        context.list.fill(ui::Rect::new(72.0, 0.0, 640.0, 96.0), [1.0; 4]);
    });
    frame.present().expect("fake product frame should present");

    let completion = execute_prepared_save(
        runtime.prepare_save(tab_id).expect("fake product dirty tab should prepare save"),
    );
    let saved = runtime.apply_save_completion(completion);
    assert!(saved.notifications.iter().any(|notification| matches!(
        notification,
        EditorNotification::SaveCompleted { tab_id: id, .. } if *id == tab_id
    )));
    assert!(!runtime.document_summary(tab_id).expect("saved tab should remain").dirty);

    let late_completion = execute_prepared_save(
        runtime.prepare_save(tab_id).expect("clean tab can still prepare a snapshot"),
    );
    let _ = runtime.confirm_close(tab_id, appkit_shell::editor_runtime::CloseConfirmation::Saved);
    assert!(runtime.apply_save_completion(late_completion).notifications.is_empty());
    assert!(runtime.document_summary(tab_id).is_none());
}
