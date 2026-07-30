use std::path::Path;

use appkit_shell::prepared_tab::PreparedTab;

use crate::app_init::build_product_workspace;
use crate::document_view::DocumentView;
use crate::plugins::editor::EditorPlugin;
use crate::tab_runtime::{TabRuntime, TabRuntimeStore};

#[test]
fn clean_reload_preserves_and_clamps_edit_position() {
    let mut document =
        DocumentView::from_external_content(Path::new("notes.md"), "first\nsecond\nthird", 2, 48.0);
    document.restore_edit_position(
        10_000,
        Some(10_000),
        ui::viewport::ScrollAnchor::new(100, 40.0),
    );

    assert_eq!(document.cursor_offset().to_usize(), document.buffer_len());
    assert_eq!(document.cursor().selection_anchor, Some(document.buffer_len()));
    assert_eq!(document.presentation.display.viewport.scroll_anchor.doc_line, 2);
}

#[test]
fn deleted_document_becomes_dirty_recovery_without_losing_content() {
    let original = Path::new("notes.md");
    let document = DocumentView::from_external_content(original, "local edits", 2, 48.0);
    let (document, presentation) = document.into_parts();
    let prepared = PreparedTab::new(
        document,
        TabRuntime::with_presentation(Box::new(EditorPlugin::new()), presentation),
    );
    let mut workspace = build_product_workspace();
    let mut runtime_store = TabRuntimeStore::default();
    let tab_id = workspace.append_prepared_tab(&mut runtime_store, prepared, None);

    crate::workspace_product::detach_deleted_document(&mut workspace, 0, original);

    assert_eq!(workspace.tab_ids(), runtime_store.ids());
    assert!(runtime_store.contains(tab_id));
    let entry = workspace.entry(0).expect("recovery entry");
    assert_eq!(entry.file_path, None);
    assert!(entry.dirty);
    assert_eq!(entry.full_text(), "local edits");
    assert_eq!(entry.disk_revision, None);
    assert_eq!(workspace.entry_title(0).as_deref(), Some("恢复：notes.md"));
}
