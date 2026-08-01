//! notora 对 shared dirty snapshot 格式的产品适配。

use std::path::{Path, PathBuf};

use appkit_core::snapshot::{
    snapshot_id_for_path, snapshot_id_for_untitled, write_snapshot, write_snapshot_with_revision,
};
use appkit_core::workspace::types::TabId;
use appkit_shell::editor_runtime::EditorWorkspaceSnapshot;

/// 可在后台写入的单个 dirty tab 快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirtySnapshotPlan {
    pub tab_id: TabId,
    pub filename: String,
    pub baseline: Option<appkit_core::file_safety::DiskRevision>,
    pub current_lines: Vec<String>,
}

/// 启动时展示给产品 UI 的恢复候选；不会自动覆盖源文件。
#[derive(Clone, Debug)]
pub enum RecoverableDirtySnapshot {
    Ready {
        filename: String,
        content_lines: Vec<String>,
        header: appkit_core::snapshot::SnapshotHeader,
    },
    Unreadable {
        filename: String,
        message: String,
    },
}

/// 根据 runtime 的只读快照生成写入计划；干净 tab 不产生快照。
pub fn collect_dirty_snapshots(workspace: &EditorWorkspaceSnapshot) -> Vec<DirtySnapshotPlan> {
    workspace
        .tabs
        .iter()
        .filter(|tab| tab.dirty)
        .map(|tab| DirtySnapshotPlan {
            tab_id: tab.tab_id,
            filename: tab
                .path
                .as_deref()
                .map(snapshot_id_for_path)
                .unwrap_or_else(|| snapshot_id_for_untitled(tab.dirty_snapshot_id.as_deref())),
            baseline: tab.disk_revision.clone(),
            current_lines: tab.content_lines.clone(),
        })
        .collect()
}

/// 只将计划写入产品注入的 snapshots 目录；实际编解码完全复用 shared crate。
pub fn write_dirty_snapshot(
    snapshots_directory: &Path,
    plan: &DirtySnapshotPlan,
) -> std::io::Result<PathBuf> {
    match &plan.baseline {
        Some(baseline) => write_snapshot_with_revision(
            snapshots_directory,
            &plan.filename,
            baseline,
            &plan.current_lines,
            &plan.current_lines,
        )?,
        None => {
            write_snapshot(snapshots_directory, &plan.filename, 0, 0, &[], &plan.current_lines)?
        }
    }
    Ok(snapshots_directory.join(&plan.filename))
}

/// 列出 snapshots 目录中的恢复候选，不执行任何源文件写入。
pub fn list_recoverable_snapshots(
    snapshots_directory: &Path,
) -> std::io::Result<Vec<RecoverableDirtySnapshot>> {
    let mut entries = Vec::new();
    for directory_entry in std::fs::read_dir(snapshots_directory)? {
        let directory_entry = directory_entry?;
        let path = directory_entry.path();
        let is_dirty_snapshot = path.extension().is_some_and(|extension| {
            extension == std::ffi::OsStr::new(appkit_core::snapshot::SNAPSHOT_EXT)
        });
        if !is_dirty_snapshot {
            continue;
        }
        let filename = directory_entry.file_name().to_string_lossy().into_owned();
        match appkit_core::snapshot::read_and_apply(snapshots_directory, &filename, &[]) {
            Ok((content_lines, header)) => {
                entries.push(RecoverableDirtySnapshot::Ready { filename, content_lines, header });
            }
            Err(error) => entries.push(RecoverableDirtySnapshot::Unreadable {
                filename,
                message: error.to_string(),
            }),
        }
    }
    entries.sort_by_key(|entry| match entry {
        RecoverableDirtySnapshot::Ready { filename, .. }
        | RecoverableDirtySnapshot::Unreadable { filename, .. } => filename.clone(),
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use appkit_core::workspace::types::TabIdAllocator;
    use appkit_shell::editor_runtime::{EditorTabSnapshot, EditorWorkspaceSnapshot};

    use super::{
        RecoverableDirtySnapshot, collect_dirty_snapshots, list_recoverable_snapshots,
        write_dirty_snapshot,
    };

    fn tab_snapshot(
        tab_id: appkit_core::workspace::types::TabId,
        dirty: bool,
    ) -> EditorTabSnapshot {
        EditorTabSnapshot {
            tab_id,
            path: Some("/workspace/note.md".into()),
            suggested_file_name: None,
            cursor_offset: 0,
            selection_anchor: None,
            cursor_line: 0,
            cursor_column: 0,
            dirty,
            disk_revision: None,
            dirty_snapshot_id: None,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
            preview_anchor_text: None,
            plugin_name: "markdown".to_owned(),
            default_plugin_name: None,
            allows_editing: true,
            content_lines: vec!["local change".to_owned()],
            clean_untitled_content: None,
        }
    }

    #[test]
    fn plans_only_dirty_tabs_and_writes_to_the_injected_directory() {
        let mut tabs = TabIdAllocator::new();
        let dirty_tab = tabs.allocate();
        let clean_tab = tabs.allocate();
        let workspace = EditorWorkspaceSnapshot {
            active_index: 0,
            tabs: vec![tab_snapshot(dirty_tab, true), tab_snapshot(clean_tab, false)],
        };
        let directory = tempfile::tempdir().expect("snapshot test directory should exist");

        let plans = collect_dirty_snapshots(&workspace);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].tab_id, dirty_tab);
        let path = write_dirty_snapshot(directory.path(), &plans[0])
            .expect("dirty snapshot should be written into the injected directory");

        assert!(path.starts_with(directory.path()));
        let (restored, _) =
            appkit_core::snapshot::read_and_apply(directory.path(), &plans[0].filename, &[])
                .expect("shared snapshot reader should restore the snapshot");
        assert_eq!(restored, vec!["local change"]);
    }

    #[test]
    fn recovery_listing_returns_content_without_writing_to_the_source_document() {
        let mut tabs = TabIdAllocator::new();
        let tab_id = tabs.allocate();
        let workspace =
            EditorWorkspaceSnapshot { active_index: 0, tabs: vec![tab_snapshot(tab_id, true)] };
        let directory = tempfile::tempdir().expect("snapshot test directory should exist");
        let plan = collect_dirty_snapshots(&workspace)
            .pop()
            .expect("dirty tab should create a snapshot plan");
        let _ = write_dirty_snapshot(directory.path(), &plan)
            .expect("dirty snapshot should be written for recovery listing");

        let entries = list_recoverable_snapshots(directory.path())
            .expect("snapshot directory should be listable");

        assert!(matches!(
            entries.as_slice(),
            [RecoverableDirtySnapshot::Ready { content_lines, .. }]
                if content_lines == &vec!["local change".to_owned()]
        ));
    }
}
