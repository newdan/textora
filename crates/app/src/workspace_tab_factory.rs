use std::path::Path;

use appkit_shell::prepared_tab::PreparedTab;
use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP};
use ui::sidebar::NewDocumentKind;

use crate::document_view::DocumentView;
use crate::tab_runtime::TabRuntime;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewportDimensions {
    pub(crate) visible_rows: usize,
    pub(crate) viewport_height: f64,
}

pub(crate) struct ProductPreparedTab {
    pub(crate) prepared: PreparedTab,
    pub(crate) suggested_file_name: Option<String>,
}

struct TypedUntitledSpec {
    suggested_file_name: &'static str,
    plugin_name: &'static str,
    initial_text: &'static str,
    initial_cursor_byte: usize,
}

fn typed_untitled_spec(kind: NewDocumentKind) -> TypedUntitledSpec {
    match kind {
        NewDocumentKind::Text => TypedUntitledSpec {
            suggested_file_name: "未命名.txt",
            plugin_name: PLUGIN_EDITOR,
            initial_text: "",
            initial_cursor_byte: 0,
        },
        NewDocumentKind::Mindmap => TypedUntitledSpec {
            suggested_file_name: "未命名.mmap.md",
            plugin_name: PLUGIN_MINDMAP,
            initial_text: "#",
            initial_cursor_byte: 1,
        },
        NewDocumentKind::Markdown => TypedUntitledSpec {
            suggested_file_name: "未命名.md",
            plugin_name: PLUGIN_MARKDOWN_EDITOR,
            initial_text: "",
            initial_cursor_byte: 0,
        },
    }
}

fn prepare_document(
    document: DocumentView,
    plugin: Box<dyn ui::plugin::ViewPlugin>,
    suggested_file_name: Option<String>,
) -> ProductPreparedTab {
    let (document, presentation) = document.into_parts();
    let runtime = TabRuntime::with_presentation(plugin, presentation);
    ProductPreparedTab { prepared: PreparedTab::new(document, runtime), suggested_file_name }
}

fn assign_untitled_snapshot(document: &mut DocumentView) {
    document.dirty_snapshot_id =
        Some(crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id()));
}

pub(crate) fn prepare_file(
    workspace: &Workspace,
    path: &Path,
    dimensions: ViewportDimensions,
) -> Result<ProductPreparedTab, String> {
    let plugin = workspace.create_plugin_for_path(path);
    prepare_file_with_plugin(path, dimensions, plugin)
}

pub(crate) fn prepare_file_with_plugin(
    path: &Path,
    dimensions: ViewportDimensions,
    plugin: Box<dyn ui::plugin::ViewPlugin>,
) -> Result<ProductPreparedTab, String> {
    let document =
        DocumentView::from_file(path, dimensions.visible_rows, dimensions.viewport_height)?;
    Ok(prepare_document(document, plugin, None))
}

pub(crate) fn prepare_external_content(
    workspace: &Workspace,
    path: &Path,
    content: &str,
    dimensions: ViewportDimensions,
) -> ProductPreparedTab {
    let plugin = workspace.create_plugin_for_path(path);
    prepare_external_content_with_plugin(path, content, dimensions, plugin)
}

pub(crate) fn prepare_external_content_with_plugin(
    path: &Path,
    content: &str,
    dimensions: ViewportDimensions,
    plugin: Box<dyn ui::plugin::ViewPlugin>,
) -> ProductPreparedTab {
    let document = DocumentView::from_external_content(
        path,
        content,
        dimensions.visible_rows,
        dimensions.viewport_height,
    );
    prepare_document(document, plugin, None)
}

pub(crate) fn prepare_untitled(
    workspace: &Workspace,
    dimensions: ViewportDimensions,
) -> ProductPreparedTab {
    let plugin = workspace.create_plugin_by_name(PLUGIN_EDITOR);
    prepare_untitled_with_plugin(dimensions, plugin)
}

pub(crate) fn prepare_untitled_with_plugin(
    dimensions: ViewportDimensions,
    plugin: Box<dyn ui::plugin::ViewPlugin>,
) -> ProductPreparedTab {
    let mut document =
        DocumentView::new(vec![String::new()], dimensions.visible_rows, dimensions.viewport_height);
    assign_untitled_snapshot(&mut document);
    prepare_document(document, plugin, None)
}

pub(crate) fn prepare_typed_untitled(
    workspace: &Workspace,
    kind: NewDocumentKind,
    dimensions: ViewportDimensions,
) -> ProductPreparedTab {
    let specification = typed_untitled_spec(kind);
    let plugin = workspace.create_plugin_by_name(specification.plugin_name);
    prepare_typed_untitled_with_plugin(kind, dimensions, plugin)
}

pub(crate) fn prepare_typed_untitled_with_plugin(
    kind: NewDocumentKind,
    dimensions: ViewportDimensions,
    plugin: Box<dyn ui::plugin::ViewPlugin>,
) -> ProductPreparedTab {
    let specification = typed_untitled_spec(kind);
    let mut document = DocumentView::new(
        vec![specification.initial_text.to_owned()],
        dimensions.visible_rows,
        dimensions.viewport_height,
    );
    document.set_cursor_offset_synced(specification.initial_cursor_byte);
    assign_untitled_snapshot(&mut document);
    prepare_document(document, plugin, Some(specification.suggested_file_name.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use appkit_shell::prepared_tab::PreparedTab;
    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP, PLUGIN_NOVEL_VIEW};
    use ui::sidebar::NewDocumentKind;

    use super::{
        ProductPreparedTab, ViewportDimensions, prepare_external_content, prepare_file,
        prepare_typed_untitled, prepare_untitled,
    };
    use crate::app_init::build_product_workspace;
    use crate::tab_runtime::TabRuntimeStore;
    use crate::workspace_persistence::{restore_workspace, snapshot_workspace};

    fn test_dimensions() -> ViewportDimensions {
        ViewportDimensions { visible_rows: 22, viewport_height: 440.0 }
    }

    fn assert_prepared_file(
        prepared: ProductPreparedTab,
        expected_path: &Path,
        expected_text: &str,
        expected_plugin: &str,
    ) {
        let ProductPreparedTab { prepared, suggested_file_name } = prepared;
        let PreparedTab { document, runtime } = prepared;

        assert_eq!(document.file_path.as_deref(), Some(expected_path));
        assert_eq!(document.full_text(), expected_text);
        assert_eq!(runtime.plugin.name(), expected_plugin);
        assert_eq!(suggested_file_name, None);
    }

    #[test]
    fn prepare_file_selects_product_plugin_for_txt_markdown_and_mindmap() {
        let directory = tempfile::tempdir().expect("factory test directory should be created");
        let cases = [
            ("notes.txt", "plain text", PLUGIN_EDITOR),
            ("notes.md", "# Markdown", PLUGIN_MARKDOWN_EDITOR),
            ("notes.mmap.md", "# Mindmap", PLUGIN_MINDMAP),
        ];
        let workspace = build_product_workspace();

        for (file_name, content, expected_plugin) in cases {
            let path = directory.path().join(file_name);
            std::fs::write(&path, content).expect("factory test file should be writable");

            let prepared =
                prepare_file(&workspace, &path, test_dimensions()).expect("test file should load");

            assert_prepared_file(prepared, &path, content, expected_plugin);
        }
    }

    #[test]
    fn workspace_path_route_preserves_default_and_toggle_plugins() {
        let workspace = build_product_workspace();

        let route = workspace
            .plugin_route_for_path(Path::new("draft.txt"))
            .expect("textora text route should exist");

        assert_eq!(route.default_plugin, PLUGIN_EDITOR);
        assert_eq!(route.toggle_target, Some(PLUGIN_NOVEL_VIEW));
    }

    #[test]
    fn prepare_external_content_selects_product_plugin_without_suggested_name() {
        let workspace = build_product_workspace();
        let path = Path::new("/virtual/shared.md");

        let prepared = prepare_external_content(&workspace, path, "# Shared", test_dimensions());

        assert_prepared_file(prepared, path, "# Shared", PLUGIN_MARKDOWN_EDITOR);
    }

    #[test]
    fn prepare_untitled_uses_editor_and_preserves_dirty_snapshot_name() {
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();

        let ProductPreparedTab { prepared, suggested_file_name } =
            prepare_untitled(&workspace, test_dimensions());
        let snapshot_name = prepared
            .document
            .dirty_snapshot_id
            .clone()
            .expect("prepared untitled tab should allocate a dirty snapshot name");

        assert!(prepared.document.file_path.is_none());
        assert_eq!(prepared.document.full_text(), "");
        assert_eq!(prepared.document.cursor_offset().to_usize(), 0);
        assert_eq!(prepared.runtime.plugin.name(), PLUGIN_EDITOR);
        assert_eq!(prepared.runtime.presentation.display.viewport.visible_rows, 22);
        assert_eq!(prepared.runtime.presentation.display.viewport.viewport_height, 440.0);
        assert_eq!(suggested_file_name, None);
        assert!(snapshot_name.ends_with(".dirty"));

        let tab_id = workspace.append_prepared_tab(&mut runtimes, prepared, suggested_file_name);
        let snapshots_directory =
            tempfile::tempdir().expect("snapshot test directory should be created");
        let persisted =
            snapshot_workspace(&workspace, &runtimes, false, None, snapshots_directory.path());

        let installed_document = workspace
            .entry(workspace.index_of(tab_id).expect("untitled tab should remain installed"))
            .expect("untitled document should remain installed");
        assert_eq!(installed_document.dirty_snapshot_id.as_deref(), Some(snapshot_name.as_str()));
        assert_eq!(persisted.entries.len(), 1);
        assert_eq!(persisted.entries[0].snapshot_filename, None);
        assert_eq!(persisted.entries[0].clean_untitled_content.as_deref(), Some(""));
    }

    #[test]
    fn prepare_typed_untitled_preserves_product_specification() {
        let cases = [
            (NewDocumentKind::Text, "", 0, PLUGIN_EDITOR, "未命名.txt"),
            (NewDocumentKind::Mindmap, "#", 1, PLUGIN_MINDMAP, "未命名.mmap.md"),
            (NewDocumentKind::Markdown, "", 0, PLUGIN_MARKDOWN_EDITOR, "未命名.md"),
        ];
        let workspace = build_product_workspace();

        for (kind, expected_text, expected_cursor, expected_plugin, expected_name) in cases {
            let ProductPreparedTab { prepared, suggested_file_name } =
                prepare_typed_untitled(&workspace, kind, test_dimensions());
            let PreparedTab { document, runtime } = prepared;

            assert!(document.file_path.is_none());
            assert_eq!(document.full_text(), expected_text);
            assert_eq!(document.cursor_offset().to_usize(), expected_cursor);
            assert_eq!(runtime.plugin.name(), expected_plugin);
            assert_eq!(suggested_file_name.as_deref(), Some(expected_name));
            assert!(
                document
                    .dirty_snapshot_id
                    .as_deref()
                    .is_some_and(|file_name| file_name.ends_with(".dirty"))
            );
        }
    }

    #[test]
    fn prepared_markdown_route_drives_workspace_plugin_toggle_round_trip() {
        let directory = tempfile::tempdir().expect("factory test directory should be created");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "# Markdown").expect("factory test file should be writable");
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let ProductPreparedTab { prepared, suggested_file_name } =
            prepare_file(&workspace, &path, test_dimensions()).expect("markdown should load");
        let tab_id = workspace.append_prepared_tab(&mut runtimes, prepared, suggested_file_name);

        assert_eq!(
            runtimes.get(tab_id).expect("prepared runtime should be installed").plugin.name(),
            PLUGIN_MARKDOWN_EDITOR
        );
        assert_eq!(workspace.toggle_target(), Some(PLUGIN_EDITOR));

        workspace.switch_plugin_with_runtime(&mut runtimes);
        assert_eq!(
            runtimes.get(tab_id).expect("toggled runtime should remain installed").plugin.name(),
            PLUGIN_EDITOR
        );
        assert!(workspace.is_toggled_for_plugin(PLUGIN_EDITOR));

        workspace.switch_plugin_with_runtime(&mut runtimes);
        assert_eq!(
            runtimes.get(tab_id).expect("restored runtime should remain installed").plugin.name(),
            PLUGIN_MARKDOWN_EDITOR
        );
        assert!(!workspace.is_toggled_for_plugin(PLUGIN_MARKDOWN_EDITOR));
    }

    #[test]
    fn prepared_tabs_without_toggle_routes_keep_their_plugins() {
        let directory = tempfile::tempdir().expect("factory test directory should be created");
        let rust_path = directory.path().join("main.rs");
        std::fs::write(&rust_path, "fn main() {}")
            .expect("factory test source file should be writable");

        let mut file_workspace = build_product_workspace();
        let mut file_runtimes = TabRuntimeStore::default();
        let ProductPreparedTab { prepared, suggested_file_name } =
            prepare_file(&file_workspace, &rust_path, test_dimensions())
                .expect("rust source should load");
        let file_id =
            file_workspace.append_prepared_tab(&mut file_runtimes, prepared, suggested_file_name);

        assert_eq!(file_workspace.toggle_target(), None);
        file_workspace.switch_plugin_with_runtime(&mut file_runtimes);
        assert_eq!(
            file_runtimes
                .get(file_id)
                .expect("source runtime should remain installed")
                .plugin
                .name(),
            PLUGIN_EDITOR
        );

        let mut untitled_workspace = build_product_workspace();
        let mut untitled_runtimes = TabRuntimeStore::default();
        let ProductPreparedTab { prepared, suggested_file_name } =
            prepare_untitled(&untitled_workspace, test_dimensions());
        let untitled_id = untitled_workspace.append_prepared_tab(
            &mut untitled_runtimes,
            prepared,
            suggested_file_name,
        );

        assert_eq!(untitled_workspace.toggle_target(), None);
        untitled_workspace.switch_plugin_with_runtime(&mut untitled_runtimes);
        assert_eq!(
            untitled_runtimes
                .get(untitled_id)
                .expect("untitled runtime should remain installed")
                .plugin
                .name(),
            PLUGIN_EDITOR
        );
    }

    #[test]
    fn typed_prepared_tab_persists_suggested_name_and_plugin() {
        let mut workspace = build_product_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let ProductPreparedTab { prepared, suggested_file_name } =
            prepare_typed_untitled(&workspace, NewDocumentKind::Mindmap, test_dimensions());
        workspace.append_prepared_tab(&mut runtimes, prepared, suggested_file_name);
        let snapshots_directory =
            tempfile::tempdir().expect("snapshot test directory should be created");
        let snapshot =
            snapshot_workspace(&workspace, &runtimes, false, None, snapshots_directory.path());
        let serialized =
            toml::to_string(&snapshot).expect("typed workspace snapshot should serialize");
        let snapshot =
            toml::from_str(&serialized).expect("typed workspace snapshot should deserialize");

        let restored = restore_workspace(
            build_product_workspace(),
            snapshot,
            test_dimensions(),
            16.0,
            snapshots_directory.path(),
        )
        .expect("typed workspace should restore");
        let active_id = restored
            .workspace
            .tab_id_at(restored.workspace.active_index())
            .expect("restored typed tab should be active");

        assert_eq!(restored.workspace.tab_ids(), restored.runtimes.ids());
        let restored_document =
            restored.workspace.active_entry().expect("restored mindmap should be active");
        assert_eq!(restored_document.full_text(), "#");
        assert_eq!(restored_document.cursor_offset().to_usize(), 1);
        assert!(!restored_document.dirty);
        assert_eq!(restored.workspace.suggested_file_name(0), Some("未命名.mmap.md"));
        assert_eq!(
            restored
                .runtimes
                .get(active_id)
                .expect("restored typed tab should retain its runtime")
                .plugin
                .name(),
            PLUGIN_MINDMAP
        );
    }

    #[test]
    fn production_tab_creation_does_not_call_workspace_product_constructors() {
        let workspace_source = include_str!("../../appkit-shell/src/workspace.rs");
        let tab_dispatch_source = include_str!("dispatch/tabs.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("tab dispatch production source should precede tests");
        let lifecycle_source = include_str!("app_lifecycle.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application lifecycle production source should precede tests");
        let command_dispatch_source = include_str!("dispatch/commands.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("command dispatch production source should precede tests");

        for legacy_call in [
            concat!("workspace.", "open_file_with_viewport("),
            concat!("workspace.", "new_untitled("),
            concat!("workspace.", "new_typed_untitled("),
            concat!("workspace.", "push_entry_for_file("),
        ] {
            assert!(
                !tab_dispatch_source.contains(legacy_call),
                "tab dispatch still calls legacy product constructor {legacy_call}"
            );
        }
        assert!(
            !lifecycle_source.contains(concat!("workspace.", "open_external_content(")),
            "application lifecycle still calls the legacy external-content constructor"
        );
        assert!(
            !command_dispatch_source.contains(concat!("workspace.", "open_file_with_viewport(")),
            "command dispatch still calls the legacy recent-file constructor"
        );
        for test_only_api in ["Workspace::new()", "set_active_index_for_test"] {
            assert!(
                !workspace_source.contains(test_only_api),
                "Workspace still exposes productized test API {test_only_api}"
            );
        }
    }
}
