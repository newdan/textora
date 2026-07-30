//! Workspace identity types.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stable identity for a tab, independent of its position in the tab strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(u64);

impl TabId {
    /// Returns the raw u64 value. Intended for debugging and persistence only.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Allocates stable, monotonically increasing tab IDs within one workspace.
#[derive(Debug, Default)]
pub struct TabIdAllocator {
    next_raw_id: u64,
}

impl TabIdAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate the next tab ID.
    ///
    /// IDs start at 1 and never decrease, so a value of 0 can be used to mean
    /// "no tab" in external serialization if desired.
    pub fn allocate(&mut self) -> TabId {
        self.next_raw_id += 1;
        TabId(self.next_raw_id)
    }
}

/// Persisted representation of one workspace tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTab {
    pub file_path: Option<PathBuf>,
    #[serde(default)]
    pub suggested_file_name: Option<String>,
    pub cursor_offset: usize,
    pub selection_anchor: Option<usize>,
    pub dirty: bool,
    #[serde(default)]
    pub scroll_anchor_line: Option<usize>,
    #[serde(default)]
    pub scroll_anchor_offset: Option<f32>,
    #[serde(default)]
    pub snapshot_filename: Option<String>,
    #[serde(default)]
    pub original_file_size: Option<u64>,
    #[serde(default)]
    pub original_mtime_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_disk_revision: Option<crate::snapshot::PersistedDiskRevision>,
    #[serde(default)]
    pub unsaved_lines: Option<Vec<String>>,
    #[serde(default)]
    pub active_plugin: Option<String>,
    #[serde(default)]
    pub preview_anchor_text: Option<String>,
    #[serde(default)]
    pub preview_anchor_offset: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clean_untitled_content: Option<String>,
}

/// Persisted representation of the workspace session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedWorkspace {
    pub version: u32,
    pub active_index: usize,
    #[serde(default)]
    pub sidebar_pinned: bool,
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    #[serde(rename = "tabs")]
    pub entries: Vec<PersistedTab>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn allocated_ids_are_non_zero() {
        let mut allocator = TabIdAllocator::new();
        let id = allocator.allocate();
        assert_ne!(id.as_u64(), 0);
    }

    #[test]
    fn allocated_ids_are_monotonically_increasing() {
        let mut allocator = TabIdAllocator::new();
        let first = allocator.allocate();
        let second = allocator.allocate();
        let third = allocator.allocate();
        assert!(first.as_u64() < second.as_u64());
        assert!(second.as_u64() < third.as_u64());
    }

    #[test]
    fn id_is_unequal_after_close_and_reopen() {
        let mut allocator = TabIdAllocator::new();
        let first = allocator.allocate();
        let second = allocator.allocate();
        // Simulate closing the first tab and allocating a new one: the new ID
        // must not reuse the old value.
        let _ = first;
        let third = allocator.allocate();
        assert_ne!(first.as_u64(), third.as_u64());
        assert_ne!(second.as_u64(), third.as_u64());
    }

    #[test]
    fn ids_survive_logical_reordering() {
        let mut allocator = TabIdAllocator::new();
        let a = allocator.allocate();
        let b = allocator.allocate();
        let c = allocator.allocate();
        // Reorder the tabs conceptually: [b, c, a]. The IDs must remain
        // attached to their original tabs, not derived from their index.
        let reordered = [b, c, a];
        assert_eq!(reordered[0], b);
        assert_eq!(reordered[1], c);
        assert_eq!(reordered[2], a);
    }

    #[test]
    fn persisted_workspace_roundtrip_with_sidebar_fields() {
        let snapshot = PersistedWorkspace {
            version: 1,
            active_index: 0,
            entries: vec![],
            sidebar_pinned: true,
            sidebar_width: Some(280.0),
        };
        let toml_string = toml::to_string_pretty(&snapshot).expect("workspace should serialize");
        let parsed: PersistedWorkspace =
            toml::from_str(&toml_string).expect("workspace should deserialize");
        assert!(parsed.sidebar_pinned);
        assert_eq!(parsed.sidebar_width, Some(280.0));
    }

    #[test]
    fn persisted_workspace_missing_sidebar_fields_default() {
        let toml_string = r#"
version = 1
active_index = 0
"#;
        let parsed: PersistedWorkspace =
            toml::from_str(toml_string).expect("legacy workspace should deserialize");
        assert!(!parsed.sidebar_pinned);
        assert_eq!(parsed.sidebar_width, None);
    }

    #[test]
    fn persisted_workspace_snapshot_filename_roundtrip() {
        let snapshot = PersistedWorkspace {
            version: 1,
            active_index: 0,
            entries: vec![PersistedTab {
                file_path: Some(PathBuf::from("/tmp/test.txt")),
                suggested_file_name: None,
                cursor_offset: 10,
                selection_anchor: Some(5),
                dirty: true,
                scroll_anchor_line: Some(0),
                scroll_anchor_offset: Some(0.0),
                snapshot_filename: Some("abc123.dirty".to_owned()),
                original_file_size: Some(42),
                original_mtime_secs: Some(1_234_567_890),
                original_disk_revision: Some(crate::snapshot::PersistedDiskRevision {
                    size: 42,
                    modified_unix_secs: Some(1_234_567_890),
                    modified_unix_nanos: 123,
                    content_hash_hex: "00".repeat(32),
                    file_device: Some(7),
                    file_inode: Some(8),
                }),
                unsaved_lines: None,
                active_plugin: None,
                preview_anchor_text: None,
                preview_anchor_offset: None,
                clean_untitled_content: None,
            }],
            sidebar_pinned: false,
            sidebar_width: None,
        };
        let toml_string = toml::to_string_pretty(&snapshot).expect("workspace should serialize");
        assert!(toml_string.contains("snapshot_filename"));
        assert!(toml_string.contains("abc123.dirty"));

        let parsed: PersistedWorkspace =
            toml::from_str(&toml_string).expect("workspace should deserialize");
        assert_eq!(parsed.entries[0].snapshot_filename, Some("abc123.dirty".to_owned()));
        assert_eq!(parsed.entries[0].original_file_size, Some(42));
        assert_eq!(parsed.entries[0].original_mtime_secs, Some(1_234_567_890));
        assert_eq!(
            parsed.entries[0]
                .original_disk_revision
                .as_ref()
                .map(|revision| revision.content_hash_hex.as_str()),
            Some("0000000000000000000000000000000000000000000000000000000000000000")
        );
        assert!(parsed.entries[0].unsaved_lines.is_none());
    }

    #[test]
    fn persisted_workspace_backward_compat_no_snapshot_fields() {
        let toml_string = r#"
version = 1
active_index = 0

[[tabs]]
file_path = "/tmp/test.txt"
cursor_offset = 5
dirty = true
unsaved_lines = ["old content line 1", "old content line 2"]
"#;
        let parsed: PersistedWorkspace =
            toml::from_str(toml_string).expect("legacy workspace should deserialize");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].snapshot_filename, None);
        assert_eq!(parsed.entries[0].original_file_size, None);
        assert_eq!(
            parsed.entries[0].unsaved_lines,
            Some(vec!["old content line 1".to_owned(), "old content line 2".to_owned()])
        );
    }

    #[test]
    fn persisted_workspace_golden_toml_roundtrip_preserves_schema() {
        let golden_toml = r#"
version = 1
active_index = 0
sidebar_pinned = true
sidebar_width = 280.0

[[tabs]]
file_path = "/tmp/test.txt"
cursor_offset = 5
selection_anchor = 2
dirty = true
snapshot_filename = "abc123.dirty"
unsaved_lines = ["legacy line"]
active_plugin = "editor"
"#;
        let parsed: PersistedWorkspace =
            toml::from_str(golden_toml).expect("golden workspace should deserialize");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.active_index, 0);
        assert!(parsed.sidebar_pinned);
        assert_eq!(parsed.sidebar_width, Some(280.0));
        assert_eq!(parsed.entries.len(), 1);
        let tab = &parsed.entries[0];
        assert_eq!(tab.file_path, Some(PathBuf::from("/tmp/test.txt")));
        assert_eq!(tab.cursor_offset, 5);
        assert_eq!(tab.selection_anchor, Some(2));
        assert!(tab.dirty);
        assert_eq!(tab.snapshot_filename.as_deref(), Some("abc123.dirty"));
        assert_eq!(tab.unsaved_lines, Some(vec!["legacy line".to_owned()]));
        assert_eq!(tab.active_plugin.as_deref(), Some("editor"));

        let reserialized = toml::to_string_pretty(&parsed).expect("workspace should serialize");
        assert!(reserialized.contains("[[tabs]]"));
        let reparsed: PersistedWorkspace =
            toml::from_str(&reserialized).expect("reserialized workspace should deserialize");
        assert_eq!(reparsed.entries.len(), 1);
        assert_eq!(reparsed.entries[0].file_path, Some(PathBuf::from("/tmp/test.txt")));
        assert_eq!(reparsed.entries[0].snapshot_filename.as_deref(), Some("abc123.dirty"));
        assert_eq!(reparsed.entries[0].unsaved_lines, Some(vec!["legacy line".to_owned()]));
        assert_eq!(reparsed.entries[0].active_plugin.as_deref(), Some("editor"));
    }
}
