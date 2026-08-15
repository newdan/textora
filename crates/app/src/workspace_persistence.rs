use std::io;
use std::path::Path;

use appkit_core::document::DocumentModel;
use appkit_core::workspace::types::{PersistedTab, PersistedWorkspace, TabId};
use appkit_shell::editor_runtime::{EditorTabSnapshot, EditorWorkspaceSnapshot};
use appkit_shell::prepared_tab::PreparedTab;
use ui::plugin::PLUGIN_EDITOR;

use crate::document_view::DocumentView;
use crate::tab_runtime::{TabRuntime, TabRuntimeStore};
use crate::tab_session::{TabSession, TabSessionMut};
use crate::workspace::Workspace;
use crate::workspace_tab_factory::ViewportDimensions;

const WORKSPACE_VERSION: u32 = 1;
const RECOVERY_TITLE_PREFIX: &str = "恢复：";
const UNNAMED_RECOVERY_TITLE: &str = "恢复：未命名";

pub(crate) struct RestoredWorkspace {
    pub(crate) workspace: Workspace,
    pub(crate) runtimes: TabRuntimeStore,
}

struct PersistedDirtyState {
    snapshot_filename: Option<String>,
    original_file_size: Option<u64>,
    original_mtime_secs: Option<i64>,
    original_disk_revision: Option<crate::dirty_snapshot::PersistedDiskRevision>,
}

fn document_lines(document: &DocumentModel) -> Vec<String> {
    (0..document.line_count())
        .filter_map(|line| {
            document.doc_line_bytes(line).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect()
}

fn read_disk_baseline(path: &Path) -> (Vec<String>, u64, i64) {
    let Ok(bytes) = std::fs::read(path) else {
        return (Vec::new(), 0, 0);
    };
    let metadata = std::fs::metadata(path).ok();
    let file_size = metadata.as_ref().map(std::fs::Metadata::len).unwrap_or(0);
    let modified_time = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let content = String::from_utf8_lossy(&bytes);
    let lines = content.lines().map(str::to_owned).collect();
    (lines, file_size, modified_time)
}

fn snapshot_dirty_state(document: &DocumentModel, snapshots_dir: &Path) -> PersistedDirtyState {
    let empty_state = || PersistedDirtyState {
        snapshot_filename: None,
        original_file_size: None,
        original_mtime_secs: None,
        original_disk_revision: None,
    };
    if !document.dirty {
        return empty_state();
    }

    let current_lines = document_lines(document);
    if current_lines.is_empty() {
        return empty_state();
    }

    let (filename, original_lines, file_size, modified_time, disk_revision) =
        if let Some(path) = document.file_path.as_deref() {
            let filename = crate::dirty_snapshot::snapshot_id_for_path(path);
            let disk_revision = crate::file_safety::capture_revision(path).ok();
            let (original_lines, file_size, modified_time) = read_disk_baseline(path);
            (filename, original_lines, file_size, modified_time, disk_revision)
        } else {
            let filename = document.dirty_snapshot_id.clone().unwrap_or_else(|| {
                crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id())
            });
            (filename, Vec::new(), 0, 0, None)
        };

    let write_result = if let Some(revision) = disk_revision.as_ref() {
        crate::dirty_snapshot::write_snapshot_with_revision(
            snapshots_dir,
            &filename,
            revision,
            &original_lines,
            &current_lines,
        )
    } else {
        crate::dirty_snapshot::write_snapshot(
            snapshots_dir,
            &filename,
            file_size,
            modified_time,
            &original_lines,
            &current_lines,
        )
    };
    if let Err(error) = write_result {
        eprintln!("[workspace] write snapshot failed: {error}");
        return empty_state();
    }

    PersistedDirtyState {
        snapshot_filename: Some(filename),
        original_file_size: Some(file_size),
        original_mtime_secs: Some(modified_time),
        original_disk_revision: disk_revision
            .as_ref()
            .map(crate::dirty_snapshot::PersistedDiskRevision::from_disk_revision),
    }
}

fn snapshot_dirty_state_from_runtime(
    tab: &EditorTabSnapshot,
    snapshots_dir: &Path,
) -> PersistedDirtyState {
    let empty_state = || PersistedDirtyState {
        snapshot_filename: None,
        original_file_size: None,
        original_mtime_secs: None,
        original_disk_revision: None,
    };
    if !tab.dirty || tab.content_lines.is_empty() {
        return empty_state();
    }

    let (filename, original_lines, file_size, modified_time, disk_revision) =
        if let Some(path) = tab.path.as_deref() {
            let filename = crate::dirty_snapshot::snapshot_id_for_path(path);
            let disk_revision = crate::file_safety::capture_revision(path).ok();
            let (original_lines, file_size, modified_time) = read_disk_baseline(path);
            (filename, original_lines, file_size, modified_time, disk_revision)
        } else {
            let filename = tab.dirty_snapshot_id.clone().unwrap_or_else(|| {
                crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id())
            });
            (filename, Vec::new(), 0, 0, None)
        };

    let write_result = if let Some(revision) = disk_revision.as_ref() {
        crate::dirty_snapshot::write_snapshot_with_revision(
            snapshots_dir,
            &filename,
            revision,
            &original_lines,
            &tab.content_lines,
        )
    } else {
        crate::dirty_snapshot::write_snapshot(
            snapshots_dir,
            &filename,
            file_size,
            modified_time,
            &original_lines,
            &tab.content_lines,
        )
    };
    if let Err(error) = write_result {
        eprintln!("[workspace] write snapshot failed: {error}");
        return empty_state();
    }

    PersistedDirtyState {
        snapshot_filename: Some(filename),
        original_file_size: Some(file_size),
        original_mtime_secs: Some(modified_time),
        original_disk_revision: disk_revision
            .as_ref()
            .map(crate::dirty_snapshot::PersistedDiskRevision::from_disk_revision),
    }
}

fn persisted_runtime_tab(tab: &EditorTabSnapshot, snapshots_dir: &Path) -> PersistedTab {
    let dirty_state = snapshot_dirty_state_from_runtime(tab, snapshots_dir);
    let active_plugin = (tab.default_plugin_name.as_deref() != Some(tab.plugin_name.as_str()))
        .then(|| tab.plugin_name.clone());

    PersistedTab {
        file_path: tab.path.clone(),
        suggested_file_name: tab.suggested_file_name.clone(),
        cursor_offset: tab.cursor_offset,
        selection_anchor: tab.selection_anchor,
        dirty: tab.dirty,
        scroll_anchor_line: Some(tab.scroll_anchor_line),
        scroll_anchor_offset: Some(tab.scroll_anchor_offset),
        snapshot_filename: dirty_state.snapshot_filename,
        original_file_size: dirty_state.original_file_size,
        original_mtime_secs: dirty_state.original_mtime_secs,
        original_disk_revision: dirty_state.original_disk_revision,
        unsaved_lines: None,
        active_plugin,
        preview_anchor_text: tab.preview_anchor_text.clone(),
        preview_anchor_offset: tab.preview_anchor_text.as_ref().map(|_| tab.scroll_anchor_offset),
        clean_untitled_content: tab.clean_untitled_content.clone(),
    }
}

fn persisted_tab(
    workspace: &Workspace,
    runtimes: &TabRuntimeStore,
    entry: &appkit_core::workspace::model::WorkspaceEntry<appkit_core::document::DocumentModel>,
    snapshots_dir: &Path,
) -> PersistedTab {
    let document = &entry.value;
    let runtime = runtimes
        .get(entry.id)
        .expect("every persisted workspace entry must have a matching tab runtime");
    let session = TabSession::new(entry.id, document, runtime);
    let dirty_state = snapshot_dirty_state(document, snapshots_dir);
    let cursor_offset =
        document.cursor().snapshot_offset.unwrap_or(document.cursor().offset.to_usize());
    let selection_anchor =
        document.cursor().snapshot_selection_anchor.unwrap_or(document.cursor().selection_anchor);
    let route_default = document.file_path.as_deref().map(|path| {
        workspace.plugin_route_for_path(path).map_or(PLUGIN_EDITOR, |route| route.default_plugin)
    });
    let active_plugin =
        (route_default != Some(session.plugin_name())).then(|| session.plugin_name().to_owned());
    let (preview_anchor_text, preview_anchor_offset) = if session.allows_editing() {
        (None, None)
    } else {
        session.scroll_anchor().map_or((None, None), |(text, offset)| (Some(text), Some(offset)))
    };
    let clean_untitled_content =
        (document.file_path.is_none() && !document.dirty).then(|| document.full_text());

    PersistedTab {
        file_path: document.file_path.clone(),
        suggested_file_name: entry.suggested_file_name.clone(),
        cursor_offset,
        selection_anchor,
        dirty: document.dirty,
        scroll_anchor_line: Some(session.scroll_anchor_state().doc_line),
        scroll_anchor_offset: Some(session.scroll_anchor_state().pixel_offset),
        snapshot_filename: dirty_state.snapshot_filename,
        original_file_size: dirty_state.original_file_size,
        original_mtime_secs: dirty_state.original_mtime_secs,
        original_disk_revision: dirty_state.original_disk_revision,
        unsaved_lines: None,
        active_plugin,
        preview_anchor_text,
        preview_anchor_offset,
        clean_untitled_content,
    }
}

pub(crate) fn snapshot_workspace(
    workspace: &Workspace,
    runtimes: &TabRuntimeStore,
    sidebar_pinned: bool,
    sidebar_width: Option<f32>,
    snapshots_dir: &Path,
) -> PersistedWorkspace {
    let entries = workspace
        .entries()
        .iter()
        .map(|entry| persisted_tab(workspace, runtimes, entry, snapshots_dir))
        .collect();
    PersistedWorkspace {
        version: WORKSPACE_VERSION,
        active_index: workspace.active_index(),
        entries,
        sidebar_pinned,
        sidebar_width,
    }
}

pub(crate) fn snapshot_runtime_workspace(
    snapshot: &EditorWorkspaceSnapshot,
    sidebar_pinned: bool,
    sidebar_width: Option<f32>,
    snapshots_dir: &Path,
) -> PersistedWorkspace {
    PersistedWorkspace {
        version: WORKSPACE_VERSION,
        active_index: snapshot.active_index,
        entries: snapshot
            .tabs
            .iter()
            .map(|tab| persisted_runtime_tab(tab, snapshots_dir))
            .collect(),
        sidebar_pinned,
        sidebar_width,
    }
}

fn restore_dirty_lines(
    tab: &PersistedTab,
    snapshots_dir: &Path,
    snapshot_baseline: &mut Option<crate::dirty_snapshot::PersistedDiskRevision>,
) -> (Option<Vec<String>>, bool) {
    let Some(snapshot_filename) = tab.snapshot_filename.as_deref() else {
        return (tab.unsaved_lines.clone(), false);
    };
    let Some(path) = tab.file_path.as_deref() else {
        let restored = crate::dirty_snapshot::read_and_apply(snapshots_dir, snapshot_filename, &[])
            .map(|(lines, _header)| lines)
            .ok();
        return (restored, false);
    };

    let original_lines = std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).lines().map(str::to_owned).collect::<Vec<_>>())
        .unwrap_or_default();
    let restored =
        crate::dirty_snapshot::read_and_apply(snapshots_dir, snapshot_filename, &original_lines)
            .map(|(lines, header)| {
                if snapshot_baseline.is_none() {
                    *snapshot_baseline = header.baseline_revision;
                }
                lines
            })
            .ok();
    let legacy_without_baseline = restored.is_some() && snapshot_baseline.is_none();
    (restored, legacy_without_baseline)
}

fn restore_document(
    tab: &PersistedTab,
    is_active: bool,
    dimensions: ViewportDimensions,
    snapshots_dir: &Path,
) -> (DocumentView, bool) {
    let mut snapshot_baseline = tab.original_disk_revision.clone();
    let (dirty_lines, mut legacy_without_baseline) =
        restore_dirty_lines(tab, snapshots_dir, &mut snapshot_baseline);
    let mut document = if let Some(lines) = dirty_lines {
        let mut restored =
            DocumentView::new(lines, dimensions.visible_rows, dimensions.viewport_height);
        restored.file_path = (!legacy_without_baseline).then(|| tab.file_path.clone()).flatten();
        if let Some(path) = restored.file_path.clone() {
            restored.set_language_from_path(&path);
            restored.disk_revision =
                snapshot_baseline.as_ref().and_then(|revision| revision.to_disk_revision(&path));
            if restored.disk_revision.is_none() {
                restored.file_path = None;
                legacy_without_baseline = true;
            }
        }
        restored.dirty_snapshot_id = tab.snapshot_filename.clone();
        restored.resize(dimensions.visible_rows, dimensions.viewport_height);
        restored
    } else if let Some(path) = tab.file_path.as_deref() {
        if is_active {
            DocumentView::from_file(path, dimensions.visible_rows, dimensions.viewport_height)
                .unwrap_or_else(|_| {
                    let mut stub = DocumentView::new(
                        vec![String::new()],
                        dimensions.visible_rows,
                        dimensions.viewport_height,
                    );
                    stub.file_path = Some(path.to_owned());
                    stub
                })
        } else {
            let mut stub = DocumentView::new(
                vec![String::new()],
                dimensions.visible_rows,
                dimensions.viewport_height,
            );
            stub.file_path = Some(path.to_owned());
            stub
        }
    } else if let Some(content) = tab.clean_untitled_content.as_ref() {
        DocumentView::new(
            vec![content.clone()],
            dimensions.visible_rows,
            dimensions.viewport_height,
        )
    } else {
        DocumentView::new(vec![String::new()], dimensions.visible_rows, dimensions.viewport_height)
    };

    let is_stub = document.buffer_len() == 0 && document.file_path.is_some();
    if is_stub {
        document.cursor_mut().snapshot_offset = Some(tab.cursor_offset);
        document.cursor_mut().snapshot_selection_anchor = Some(tab.selection_anchor);
        document.set_cursor_offset_synced(0);
        document.cursor_mut().selection_anchor = tab.selection_anchor.map(|_| 0);
    } else {
        document.set_cursor_offset_synced(tab.cursor_offset.min(document.buffer_len()));
        document.cursor_mut().selection_anchor =
            tab.selection_anchor.map(|anchor| anchor.min(document.buffer_len()));
    }
    document.dirty = tab.dirty;
    (document, legacy_without_baseline)
}

fn recovery_name(tab: &PersistedTab, legacy_without_baseline: bool) -> Option<String> {
    if !legacy_without_baseline {
        return None;
    }
    Some(
        tab.file_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(|name| format!("{RECOVERY_TITLE_PREFIX}{name}"))
            .unwrap_or_else(|| UNNAMED_RECOVERY_TITLE.to_owned()),
    )
}

fn restore_runtime_state(
    session: &mut TabSessionMut<'_>,
    anchor_text: Option<&str>,
    anchor_offset: f32,
    cached_toggle_source: Option<Box<dyn ui::plugin::ViewPlugin>>,
) {
    if !session.runtime.plugin.allows_editing()
        && let Some(text) = anchor_text
    {
        session.send_message(ui::plugin::PluginMessage::RestoreScrollAnchor {
            text: text.to_owned(),
            offset: anchor_offset,
        });
    }
    if let Some(plugin) = cached_toggle_source {
        session.cache_toggle_source_plugin(plugin);
    }
}

fn append_restored_tab(
    workspace: &mut Workspace,
    runtimes: &mut TabRuntimeStore,
    tab: &PersistedTab,
    is_active: bool,
    dimensions: ViewportDimensions,
    snapshots_dir: &Path,
) -> TabId {
    let (document, legacy_without_baseline) =
        restore_document(tab, is_active, dimensions, snapshots_dir);
    let route = tab.file_path.as_deref().and_then(|path| workspace.plugin_route_for_path(path));
    let plugin_name = tab
        .active_plugin
        .as_deref()
        .or(route.map(|rule| rule.default_plugin))
        .unwrap_or(PLUGIN_EDITOR);
    let plugin = workspace.create_plugin_by_name(plugin_name);
    let cached_toggle_source = route
        .and_then(|rule| rule.toggle_target)
        .filter(|target| plugin_name != *target)
        .map(|target| workspace.create_plugin_by_name(target));
    let scroll_anchor = ui::viewport::ScrollAnchor::new(
        tab.scroll_anchor_line.unwrap_or(0),
        tab.scroll_anchor_offset.unwrap_or(0.0),
    );
    let is_stub = document.buffer_len() == 0 && document.file_path.is_some();
    let (document, presentation) = document.into_parts();
    let runtime = TabRuntime::with_presentation(plugin, presentation);
    let suggested_name =
        tab.suggested_file_name.clone().or_else(|| recovery_name(tab, legacy_without_baseline));
    let tab_id = workspace.append_prepared_tab(
        runtimes,
        PreparedTab::new(document, runtime),
        suggested_name,
    );
    let index = workspace
        .index_of(tab_id)
        .expect("a restored tab must remain installed while its runtime state is restored");
    let document = workspace.entry_mut(index).expect("a restored tab ID must address its document");
    let runtime = runtimes
        .get_mut(tab_id)
        .expect("append_prepared_tab must install a matching restored runtime");
    let restored_anchor = if is_stub {
        ui::viewport::ScrollAnchor::new(
            scroll_anchor.doc_line.min(document.line_count().saturating_sub(1)),
            scroll_anchor.pixel_offset.max(0.0),
        )
    } else {
        scroll_anchor
    };
    let mut session = TabSessionMut::new(tab_id, document, runtime);
    session.set_scroll_anchor_state(restored_anchor);
    restore_runtime_state(
        &mut session,
        tab.preview_anchor_text.as_deref(),
        tab.preview_anchor_offset.unwrap_or(0.0),
        cached_toggle_source,
    );
    tab_id
}

pub(crate) fn restore_workspace(
    mut workspace: Workspace,
    snapshot: PersistedWorkspace,
    dimensions: ViewportDimensions,
    line_height: f64,
    snapshots_dir: &Path,
) -> io::Result<RestoredWorkspace> {
    let mut runtimes = TabRuntimeStore::default();
    let active_index = snapshot.active_index;
    for (index, tab) in snapshot.entries.iter().enumerate() {
        append_restored_tab(
            &mut workspace,
            &mut runtimes,
            tab,
            index == active_index,
            dimensions,
            snapshots_dir,
        );
    }

    if !workspace.is_empty() {
        let target_index = active_index.min(workspace.len() - 1);
        let effect = workspace.switch_to(target_index);
        effect.reconcile_runtime_store(&mut runtimes);
        let active_id = workspace
            .tab_id_at(workspace.active_index())
            .expect("a non-empty restored workspace must have an active tab ID");
        if let (Some(document), Some(runtime)) =
            (workspace.active_entry_mut(), runtimes.get_mut(active_id))
        {
            let mut session = TabSessionMut::new(active_id, document, runtime);
            session.ensure_cursor_visible(line_height as f32);
        }
    }

    Ok(RestoredWorkspace { workspace, runtimes })
}

#[cfg(test)]
mod tests {
    use appkit_shell::prepared_tab::PreparedTab;
    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_NOVEL_VIEW};
    use ui::viewport::ScrollAnchor;

    use super::{restore_workspace, snapshot_workspace};
    use crate::app_init::build_product_workspace;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;
    use crate::tab_runtime::{TabRuntime, TabRuntimeStore};
    use crate::tab_session::{TabSession, TabSessionMut};
    use crate::workspace_tab_factory::ViewportDimensions;

    fn append_document(
        workspace: &mut crate::workspace::Workspace,
        runtimes: &mut TabRuntimeStore,
        document: DocumentView,
    ) -> appkit_core::workspace::types::TabId {
        let (document, presentation) = document.into_parts();
        let prepared = PreparedTab::new(
            document,
            TabRuntime::with_presentation(Box::new(EditorPlugin::new()), presentation),
        );
        workspace.append_prepared_tab(runtimes, prepared, None)
    }

    #[test]
    fn persisted_workspace_without_suggested_file_name_defaults_to_none() {
        let serialized = r#"
version = 1
active_index = 0

[[tabs]]
file_path = "/tmp/test.txt"
cursor_offset = 5
dirty = false
"#;

        let parsed: appkit_core::workspace::types::PersistedWorkspace =
            toml::from_str(serialized).expect("legacy workspace snapshot should deserialize");

        assert_eq!(parsed.entries[0].suggested_file_name, None);
    }

    #[test]
    fn direct_snapshot_uses_stub_cursor_snapshot_fields() {
        let directory = tempfile::tempdir().expect("stub snapshot directory should be created");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let mut stub = DocumentView::new(vec![String::new()], 10, 160.0);
        stub.file_path = Some(directory.path().join("missing.txt"));
        stub.dirty = true;
        stub.cursor_mut().snapshot_offset = Some(5);
        stub.cursor_mut().snapshot_selection_anchor = Some(Some(2));
        append_document(&mut workspace, &mut runtimes, stub);

        let snapshot = snapshot_workspace(&workspace, &runtimes, false, None, directory.path());

        assert_eq!(snapshot.entries[0].cursor_offset, 5);
        assert_eq!(snapshot.entries[0].selection_anchor, Some(2));
    }

    #[test]
    fn dirty_file_round_trip_preserves_diff_content_and_revision_baseline() {
        let directory = tempfile::tempdir().expect("dirty snapshot directory should be created");
        let file_path = directory.path().join("dirty.txt");
        std::fs::write(&file_path, "original").expect("dirty snapshot baseline should be writable");
        let dimensions = ViewportDimensions { visible_rows: 20, viewport_height: 320.0 };
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let tab_id = append_document(
            &mut workspace,
            &mut runtimes,
            DocumentView::from_file(
                &file_path,
                dimensions.visible_rows,
                dimensions.viewport_height,
            )
            .expect("dirty snapshot baseline should load"),
        );
        let index = workspace.index_of(tab_id).expect("dirty test tab should remain installed");
        let document = workspace.entry_mut(index).expect("dirty test document should exist");
        document.cursor_move_to_line_end();
        document.insert_at_cursor(b" changed");

        let snapshot = snapshot_workspace(&workspace, &runtimes, false, None, directory.path());

        assert!(snapshot.entries[0].dirty);
        assert!(snapshot.entries[0].snapshot_filename.is_some());
        assert!(snapshot.entries[0].original_disk_revision.is_some());
        let restored = restore_workspace(
            build_product_workspace(),
            snapshot,
            dimensions,
            16.0,
            directory.path(),
        )
        .expect("dirty workspace should restore");
        let restored_document =
            restored.workspace.active_entry().expect("restored dirty document should exist");
        assert_eq!(restored_document.full_text(), "original changed");
        assert_eq!(restored_document.file_path.as_deref(), Some(file_path.as_path()));
        assert!(restored_document.disk_revision.is_some());
        assert!(restored_document.dirty);
        assert_eq!(restored.workspace.tab_ids(), restored.runtimes.ids());
    }

    #[test]
    fn direct_restore_uses_supplied_viewport_dimensions() {
        let directory = tempfile::tempdir().expect("viewport snapshot directory should be created");
        let mut source = build_product_workspace();
        let mut source_runtimes = TabRuntimeStore::default();
        append_document(
            &mut source,
            &mut source_runtimes,
            DocumentView::new(vec![String::new()], 3, 48.0),
        );
        let snapshot = snapshot_workspace(&source, &source_runtimes, false, None, directory.path());
        let dimensions = ViewportDimensions { visible_rows: 9, viewport_height: 144.0 };

        let restored = restore_workspace(
            build_product_workspace(),
            snapshot,
            dimensions,
            16.0,
            directory.path(),
        )
        .expect("workspace should restore with supplied viewport dimensions");
        let active_id = restored
            .workspace
            .tab_id_at(restored.workspace.active_index())
            .expect("restored viewport tab should have an ID");
        let viewport = &restored
            .runtimes
            .get(active_id)
            .expect("restored viewport tab should have a runtime")
            .presentation
            .display
            .viewport;

        assert_eq!(viewport.visible_rows, dimensions.visible_rows);
        assert_eq!(viewport.viewport_height, dimensions.viewport_height);
    }

    #[test]
    fn direct_snapshot_reads_workspace_suggested_file_name() {
        let directory =
            tempfile::tempdir().expect("suggested-name snapshot directory should be created");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let document = DocumentView::new(vec![String::new()], 10, 160.0);
        let (document, presentation) = document.into_parts();
        let prepared = PreparedTab::new(
            document,
            TabRuntime::with_presentation(Box::new(EditorPlugin::new()), presentation),
        );
        workspace.append_prepared_tab(&mut runtimes, prepared, Some("suggested.md".to_owned()));
        workspace.clear_suggested_file_name(workspace.active_index());

        let snapshot = snapshot_workspace(&workspace, &runtimes, false, None, directory.path());

        assert_eq!(snapshot.entries[0].suggested_file_name, None);
    }

    #[test]
    fn direct_snapshot_omits_editor_for_file_without_explicit_route() {
        let directory =
            tempfile::tempdir().expect("default-plugin snapshot directory should be created");
        let file_path = directory.path().join("main.rs");
        std::fs::write(&file_path, "fn main() {}")
            .expect("default-plugin snapshot file should be writable");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        append_document(
            &mut workspace,
            &mut runtimes,
            DocumentView::from_file(&file_path, 20, 320.0)
                .expect("default-plugin snapshot file should load"),
        );

        let snapshot = snapshot_workspace(&workspace, &runtimes, false, None, directory.path());

        assert_eq!(snapshot.entries[0].active_plugin, None);
    }

    #[test]
    fn clean_large_workspace_snapshot_does_not_clone_all_lines() {
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let lines = (0..4096).map(|line| format!("clean line {line}")).collect::<Vec<_>>();
        for _ in 0..32 {
            append_document(
                &mut workspace,
                &mut runtimes,
                DocumentView::new(lines.clone(), 80, 1280.0),
            );
        }
        let directory = tempfile::tempdir().expect("large snapshot directory should be created");

        let started_at = std::time::Instant::now();
        let snapshot = snapshot_workspace(&workspace, &runtimes, false, None, directory.path());
        let elapsed = started_at.elapsed();

        assert_eq!(snapshot.entries.len(), 32);
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "clean workspace snapshot should not clone every document line, took {elapsed:?}"
        );
    }

    #[test]
    fn orphan_cleanup_removes_stale_snapshots_and_keeps_active_snapshots() {
        let directory = tempfile::tempdir().expect("orphan snapshot directory should be created");
        let snapshots_dir = directory.path().join("snapshots");
        std::fs::create_dir_all(&snapshots_dir)
            .expect("orphan snapshot directory should be writable");
        std::fs::write(snapshots_dir.join("stale.dirty"), b"old")
            .expect("stale snapshot should be writable");
        std::fs::write(snapshots_dir.join("active.dirty"), b"current")
            .expect("active snapshot should be writable");
        std::fs::write(snapshots_dir.join("not_a_snapshot.txt"), b"ignore")
            .expect("non-snapshot file should be writable");
        let active_snapshots = std::collections::HashSet::from(["active.dirty".to_owned()]);

        crate::dirty_snapshot::cleanup_orphans(&snapshots_dir, &active_snapshots);

        assert!(!snapshots_dir.join("stale.dirty").exists());
        assert!(snapshots_dir.join("active.dirty").exists());
        assert!(snapshots_dir.join("not_a_snapshot.txt").exists());
    }

    #[test]
    fn direct_adapter_round_trip_preserves_active_session_and_runtime_bijection() {
        let directory = tempfile::tempdir().expect("persistence test directory should be created");
        let first_path = directory.path().join("first.txt");
        let active_path = directory.path().join("active.txt");
        std::fs::write(&first_path, "first").expect("first test document should be writable");
        std::fs::write(
            &active_path,
            "line 0\nline 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nline 11",
        )
            .expect("active test document should be writable");

        let dimensions = ViewportDimensions { visible_rows: 10, viewport_height: 160.0 };
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        append_document(
            &mut workspace,
            &mut runtimes,
            DocumentView::from_file(
                &first_path,
                dimensions.visible_rows,
                dimensions.viewport_height,
            )
            .expect("first test document should load"),
        );
        append_document(
            &mut workspace,
            &mut runtimes,
            DocumentView::from_file(
                &active_path,
                dimensions.visible_rows,
                dimensions.viewport_height,
            )
            .expect("active test document should load"),
        );
        let effect = workspace.switch_to(1);
        effect.reconcile_runtime_store(&mut runtimes);
        let active_id = workspace
            .tab_id_at(workspace.active_index())
            .expect("source active tab should have an ID");
        let expected_cursor_offset;
        {
            let document =
                workspace.active_entry_mut().expect("source active document should exist");
            document.set_cursor_offset_synced(0);
            document.cursor_move_down();
            assert_eq!(document.cursor_line(), 1);
            expected_cursor_offset = document.cursor_offset().to_usize();
            document.cursor_mut().selection_anchor = Some(2);
            let runtime = runtimes.get_mut(active_id).expect("source active runtime should exist");
            let mut session = TabSessionMut::new(active_id, document, runtime);
            session.set_scroll_anchor_state(ScrollAnchor::new(1, 3.5));
        }

        let snapshot =
            snapshot_workspace(&workspace, &runtimes, true, Some(240.0), directory.path());
        assert_eq!(snapshot.entries[1].scroll_anchor_line, Some(1));
        assert_eq!(snapshot.entries[1].scroll_anchor_offset, Some(3.5));
        let restored = restore_workspace(
            build_product_workspace(),
            snapshot,
            dimensions,
            16.0,
            directory.path(),
        )
        .expect("workspace should restore through the direct adapter");

        assert_eq!(restored.workspace.active_index(), 1);
        let restored_active_id = restored
            .workspace
            .tab_id_at(restored.workspace.active_index())
            .expect("restored active tab should have an ID");
        assert_eq!(restored.workspace.tab_ids(), restored.runtimes.ids());
        let restored_document =
            restored.workspace.active_entry().expect("restored active document should exist");
        assert_eq!(restored_document.cursor_offset().to_usize(), expected_cursor_offset);
        assert_eq!(restored_document.cursor().selection_anchor, Some(2));
        let restored_runtime = restored
            .runtimes
            .get(restored_active_id)
            .expect("restored active runtime should exist");
        let restored_session =
            TabSession::new(restored_active_id, restored_document, restored_runtime);
        assert_eq!(restored_session.scroll_anchor_state().doc_line, 1);
        assert_eq!(restored_session.scroll_anchor_state().pixel_offset, 3.5);
    }

    #[test]
    fn restored_file_route_preserves_default_and_toggle_plugins() {
        let directory = tempfile::tempdir().expect("route persistence directory should be created");
        let file_path = directory.path().join("route.txt");
        std::fs::write(&file_path, "route").expect("route test document should be writable");
        let dimensions = ViewportDimensions { visible_rows: 20, viewport_height: 320.0 };
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        append_document(
            &mut workspace,
            &mut runtimes,
            DocumentView::from_file(
                &file_path,
                dimensions.visible_rows,
                dimensions.viewport_height,
            )
            .expect("route test document should load"),
        );

        let snapshot = snapshot_workspace(&workspace, &runtimes, false, None, directory.path());
        let mut restored = restore_workspace(
            build_product_workspace(),
            snapshot,
            dimensions,
            16.0,
            directory.path(),
        )
        .expect("route workspace should restore");

        let active_id = restored
            .workspace
            .tab_id_at(restored.workspace.active_index())
            .expect("restored route tab should have an ID");
        assert_eq!(
            restored
                .runtimes
                .get(active_id)
                .expect("restored route runtime should exist")
                .plugin
                .name(),
            PLUGIN_EDITOR
        );
        restored.workspace.switch_plugin_with_runtime(&mut restored.runtimes);
        assert_eq!(
            restored
                .runtimes
                .get(active_id)
                .expect("toggled route runtime should exist")
                .plugin
                .name(),
            PLUGIN_NOVEL_VIEW
        );
        restored.workspace.switch_plugin_with_runtime(&mut restored.runtimes);
        assert_eq!(
            restored
                .runtimes
                .get(active_id)
                .expect("restored default route runtime should exist")
                .plugin
                .name(),
            PLUGIN_EDITOR
        );
    }
}
