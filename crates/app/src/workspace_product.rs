use std::path::Path;

use appkit_core::document::DocumentModel;

use crate::document_view::DocumentView;
use crate::file_history::FileHistoryEntry;
use crate::workspace::Workspace;

const RECOVERY_TITLE_PREFIX: &str = "恢复：";

pub(crate) fn hydrate_active_stub(workspace: &mut Workspace) -> bool {
    let active_index = workspace.active_index();
    let Some(document) = workspace.entry_mut(active_index) else {
        return false;
    };
    hydrate_stub_document(document)
}

pub(crate) fn hydrate_stub_document(document: &mut DocumentModel) -> bool {
    if document.file_path.is_none() || document.line_count() != 1 || document.buffer_len() != 0 {
        return false;
    }

    let hydration_started = std::time::Instant::now();
    let path =
        document.file_path.clone().expect("stub path existence checked before active hydration");
    let Ok(loaded) = DocumentView::from_file(&path, 1, 1.0) else {
        return false;
    };
    let (mut loaded_model, _) = loaded.into_parts();
    loaded_model.dirty = document.dirty;
    *document = loaded_model;
    eprintln!("[perf:lazy_load] from_file={}us", hydration_started.elapsed().as_micros());
    true
}

pub(crate) fn detach_deleted_document(
    workspace: &mut Workspace,
    index: usize,
    original_path: &Path,
) {
    let Some(document) = workspace.entry_mut(index) else {
        return;
    };
    document.file_path = None;
    document.disk_revision = None;
    document.dirty = true;
    if document.dirty_snapshot_id.is_none() {
        document.dirty_snapshot_id =
            Some(crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id()));
    }

    let file_name = original_path.file_name().and_then(|name| name.to_str()).unwrap_or("untitled");
    workspace.set_suggested_file_name(index, Some(format!("{RECOVERY_TITLE_PREFIX}{file_name}")));
}

pub(crate) fn history_entry(
    workspace: &Workspace,
    index: usize,
    scroll_anchor: ui::viewport::ScrollAnchor,
) -> Option<FileHistoryEntry> {
    let document = workspace.entry(index)?;
    let file_path = document.file_path.clone()?;
    Some(FileHistoryEntry {
        file_path,
        workspace_root: None,
        last_closed_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        last_cursor_line: document.cursor_line(),
        last_cursor_col: document.cursor_column(),
        scroll_anchor_line: scroll_anchor.doc_line,
        scroll_anchor_offset: scroll_anchor.pixel_offset,
    })
}

fn copy_tab_path_with<E>(
    workspace: &Workspace,
    index: usize,
    mut copy_text: impl FnMut(String) -> Result<(), E>,
) {
    let Some(path) = workspace.entry(index).and_then(|document| document.file_path.as_deref())
    else {
        return;
    };
    let _ = copy_text(path.to_string_lossy().into_owned());
}

pub(crate) fn copy_tab_path(workspace: &Workspace, index: usize) {
    let Some(document) = workspace.entry(index) else {
        return;
    };
    copy_document_path(document);
}

pub(crate) fn copy_document_path(document: &DocumentModel) {
    let Some(path) = document.file_path.as_deref() else {
        return;
    };
    let text = path.to_string_lossy().into_owned();
    let _ = (|| {
        let mut clipboard = arboard::Clipboard::new().map_err(|_| ())?;
        clipboard.set_text(text).map_err(|_| ())
    })();
}

#[cfg(test)]
mod tests {
    use appkit_shell::prepared_tab::PreparedTab;

    use super::{copy_tab_path_with, detach_deleted_document, history_entry, hydrate_active_stub};
    use crate::app_init::build_product_workspace;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;
    use crate::tab_runtime::{TabRuntime, TabRuntimeStore};
    use crate::workspace::Workspace;

    fn append_document(
        workspace: &mut Workspace,
        runtimes: &mut TabRuntimeStore,
        document: DocumentView,
    ) {
        let (document, presentation) = document.into_parts();
        let prepared = PreparedTab::new(
            document,
            TabRuntime::with_presentation(Box::new(EditorPlugin::new()), presentation),
        );
        workspace.append_prepared_tab(runtimes, prepared, None);
    }

    #[test]
    fn active_stub_hydration_loads_file_and_preserves_dirty_state() {
        let directory = tempfile::tempdir().expect("hydration directory should be created");
        let file_path = directory.path().join("hydrated.txt");
        std::fs::write(&file_path, "loaded content\nline 2\n")
            .expect("hydration fixture should be written");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let mut stub = DocumentView::new(vec![String::new()], 10, 160.0);
        stub.file_path = Some(file_path);
        stub.dirty = true;
        stub.cursor_mut().snapshot_offset = Some(8);
        stub.cursor_mut().snapshot_selection_anchor = Some(Some(2));
        append_document(&mut workspace, &mut runtimes, stub);
        let inactive_path = directory.path().join("inactive.txt");
        std::fs::write(&inactive_path, "inactive content")
            .expect("inactive hydration fixture should be written");
        let mut inactive_stub = DocumentView::new(vec![String::new()], 10, 160.0);
        inactive_stub.file_path = Some(inactive_path);
        append_document(&mut workspace, &mut runtimes, inactive_stub);

        assert!(hydrate_active_stub(&mut workspace));
        let hydrated = workspace.active_doc().expect("hydrated document should remain active");
        assert_eq!(hydrated.full_text(), "loaded content\nline 2\n");
        assert!(hydrated.dirty);
        assert_eq!(hydrated.cursor().offset.to_usize(), 0);
        assert_eq!(hydrated.cursor().selection_anchor, None);
        assert_eq!(
            workspace.entry(1).expect("inactive stub should remain installed").buffer_len(),
            0
        );
        assert!(!hydrate_active_stub(&mut workspace));
    }

    #[test]
    fn active_stub_hydration_swallowing_read_failure_leaves_stub_unchanged() {
        let directory = tempfile::tempdir().expect("hydration directory should be created");
        let missing_path = directory.path().join("missing.txt");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let mut stub = DocumentView::new(vec![String::new()], 10, 160.0);
        stub.file_path = Some(missing_path.clone());
        append_document(&mut workspace, &mut runtimes, stub);

        assert!(!hydrate_active_stub(&mut workspace));
        let unchanged = workspace.active_doc().expect("failed hydration should keep the stub");
        assert_eq!(unchanged.file_path.as_ref(), Some(&missing_path));
        assert_eq!(unchanged.buffer_len(), 0);
    }

    #[test]
    fn deleted_document_detaches_as_named_dirty_recovery() {
        let original_path = std::path::Path::new("/notes/review.md");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let document = DocumentView::from_external_content(original_path, "local edits", 10, 160.0);
        append_document(&mut workspace, &mut runtimes, document);

        detach_deleted_document(&mut workspace, 0, original_path);

        let recovered = workspace.entry(0).expect("recovered document should remain installed");
        assert_eq!(recovered.file_path, None);
        assert_eq!(recovered.disk_revision, None);
        assert!(recovered.dirty);
        assert!(recovered.dirty_snapshot_id.is_some());
        assert_eq!(recovered.full_text(), "local edits");
        assert_eq!(workspace.entry_title(0).as_deref(), Some("恢复：review.md"));
    }

    #[test]
    fn history_entry_preserves_document_cursor_and_scroll_fields() {
        let file_path = std::path::Path::new("/notes/history.txt");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let mut document =
            DocumentView::from_external_content(file_path, "first\nsecond", 10, 160.0);
        document.set_cursor_offset_synced(8);
        append_document(&mut workspace, &mut runtimes, document);
        let scroll_anchor = ui::viewport::ScrollAnchor::new(1, 7.5);

        let entry = history_entry(&workspace, 0, scroll_anchor)
            .expect("file-backed document should produce history");

        assert_eq!(entry.file_path, file_path);
        assert_eq!(entry.workspace_root, None);
        assert!(entry.last_closed_at > 0);
        assert_eq!(entry.last_cursor_line, 1);
        assert_eq!(entry.last_cursor_col, 2);
        assert_eq!(entry.scroll_anchor_line, 1);
        assert_eq!(entry.scroll_anchor_offset, 7.5);
    }

    #[test]
    fn copy_tab_path_passes_exact_path_and_swallows_sink_failure() {
        let file_path = std::path::Path::new("/notes/copied.txt");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let document = DocumentView::from_external_content(file_path, "copy", 10, 160.0);
        append_document(&mut workspace, &mut runtimes, document);
        let mut copied_text = None;

        copy_tab_path_with(&workspace, 0, |text| {
            copied_text = Some(text);
            Err(())
        });

        assert_eq!(copied_text.as_deref(), Some("/notes/copied.txt"));
    }

    #[test]
    fn deleted_file_callers_use_the_product_adapter_directly() {
        let lifecycle_source = include_str!("app_lifecycle.rs");
        let app_tab_source = include_str!("app_tab.rs");
        let external_change_test_source = include_str!("external_change_tests.rs");

        assert!(lifecycle_source.contains("detach_deleted_editor_document"));
        assert!(!lifecycle_source.contains(".detach_after_deletion("));
        assert!(app_tab_source.contains("editor_runtime.detach_document"));

        assert!(external_change_test_source.contains("workspace_product::detach_deleted_document"));
        assert!(!external_change_test_source.contains(".detach_after_deletion("));
    }

    #[test]
    fn workspace_source_has_no_product_dependencies() {
        let workspace_source = include_str!("../../appkit-shell/src/workspace.rs");
        for forbidden in [
            "crate::document_view",
            "crate::dirty_snapshot",
            "crate::file_safety",
            "crate::file_history",
            "crate::app_init",
            "crate::plugins",
            "DocumentView",
            "NewDocumentKind",
            "arboard",
            "textora_markdown",
        ] {
            assert!(!workspace_source.contains(forbidden), "found {forbidden}");
        }
    }
}
