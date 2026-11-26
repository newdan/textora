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

impl PersistedWorkspace {
    /// Parse a workspace TOML document without generic `serde` monomorphization.
    pub fn from_toml_str(source: &str) -> Result<Self, String> {
        let value = source.parse::<toml::Value>().map_err(|e| e.to_string())?;
        let table = value.as_table().ok_or_else(|| "workspace root must be a table".to_owned())?;
        let version = req_u32(table, "version")?;
        let active_index = req_usize(table, "active_index")?;
        let sidebar_pinned = opt_bool(table, "sidebar_pinned")?.unwrap_or(false);
        let sidebar_width = opt_f32(table, "sidebar_width")?;
        let entries = match table.get("tabs") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| "workspace field `tabs` must be an array".to_owned())?
                .iter()
                .map(|item| {
                    let tab_table = item
                        .as_table()
                        .ok_or_else(|| "workspace `tabs` entry must be a table".to_owned())?;
                    parse_persisted_tab(tab_table)
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        Ok(Self { version, active_index, sidebar_pinned, sidebar_width, entries })
    }

    /// Serialize a workspace TOML document without generic `serde` monomorphization.
    pub fn to_toml_string_pretty(&self) -> Result<String, String> {
        let mut table = toml::value::Table::new();
        table.insert("version".to_owned(), toml::Value::Integer(i64::from(self.version)));
        table.insert("active_index".to_owned(), toml_int_usize("active_index", self.active_index)?);
        table.insert("sidebar_pinned".to_owned(), toml::Value::Boolean(self.sidebar_pinned));
        if let Some(width) = self.sidebar_width {
            table.insert("sidebar_width".to_owned(), toml::Value::Float(f64::from(width)));
        }
        let tabs = self.entries.iter().map(persisted_tab_to_toml).collect::<Result<Vec<_>, _>>()?;
        table.insert("tabs".to_owned(), toml::Value::Array(tabs));
        toml::to_string_pretty(&toml::Value::Table(table)).map_err(|e| e.to_string())
    }
}

fn parse_persisted_tab(table: &toml::value::Table) -> Result<PersistedTab, String> {
    let original_disk_revision = match table.get("original_disk_revision") {
        Some(v) => Some(parse_disk_revision(
            v.as_table().ok_or_else(|| "`original_disk_revision` must be a table".to_owned())?,
        )?),
        None => None,
    };
    Ok(PersistedTab {
        file_path: opt_str(table, "file_path")?.map(PathBuf::from),
        suggested_file_name: opt_str(table, "suggested_file_name")?.map(str::to_owned),
        cursor_offset: req_usize(table, "cursor_offset")?,
        selection_anchor: opt_usize(table, "selection_anchor")?,
        dirty: req_bool(table, "dirty")?,
        scroll_anchor_line: opt_usize(table, "scroll_anchor_line")?,
        scroll_anchor_offset: opt_f32(table, "scroll_anchor_offset")?,
        snapshot_filename: opt_str(table, "snapshot_filename")?.map(str::to_owned),
        original_file_size: opt_u64(table, "original_file_size")?,
        original_mtime_secs: opt_i64(table, "original_mtime_secs")?,
        original_disk_revision,
        unsaved_lines: opt_str_vec(table, "unsaved_lines")?,
        active_plugin: opt_str(table, "active_plugin")?.map(str::to_owned),
        preview_anchor_text: opt_str(table, "preview_anchor_text")?.map(str::to_owned),
        preview_anchor_offset: opt_f32(table, "preview_anchor_offset")?,
        clean_untitled_content: opt_str(table, "clean_untitled_content")?.map(str::to_owned),
    })
}

fn persisted_tab_to_toml(tab: &PersistedTab) -> Result<toml::Value, String> {
    let mut table = toml::value::Table::new();
    if let Some(path) = &tab.file_path {
        table.insert("file_path".to_owned(), path_to_toml("file_path", path)?);
    }
    if let Some(name) = &tab.suggested_file_name {
        table.insert("suggested_file_name".to_owned(), toml::Value::String(name.clone()));
    }
    table.insert("cursor_offset".to_owned(), toml_int_usize("cursor_offset", tab.cursor_offset)?);
    if let Some(anchor) = tab.selection_anchor {
        table.insert("selection_anchor".to_owned(), toml_int_usize("selection_anchor", anchor)?);
    }
    table.insert("dirty".to_owned(), toml::Value::Boolean(tab.dirty));
    if let Some(line) = tab.scroll_anchor_line {
        table.insert("scroll_anchor_line".to_owned(), toml_int_usize("scroll_anchor_line", line)?);
    }
    if let Some(offset) = tab.scroll_anchor_offset {
        table.insert("scroll_anchor_offset".to_owned(), toml::Value::Float(f64::from(offset)));
    }
    if let Some(name) = &tab.snapshot_filename {
        table.insert("snapshot_filename".to_owned(), toml::Value::String(name.clone()));
    }
    if let Some(size) = tab.original_file_size {
        table.insert("original_file_size".to_owned(), toml_int_u64("original_file_size", size)?);
    }
    if let Some(secs) = tab.original_mtime_secs {
        table.insert("original_mtime_secs".to_owned(), toml::Value::Integer(secs));
    }
    if let Some(rev) = &tab.original_disk_revision {
        table.insert("original_disk_revision".to_owned(), disk_revision_to_toml(rev)?);
    }
    if let Some(lines) = &tab.unsaved_lines {
        let items = lines.iter().cloned().map(toml::Value::String).collect();
        table.insert("unsaved_lines".to_owned(), toml::Value::Array(items));
    }
    if let Some(plugin) = &tab.active_plugin {
        table.insert("active_plugin".to_owned(), toml::Value::String(plugin.clone()));
    }
    if let Some(text) = &tab.preview_anchor_text {
        table.insert("preview_anchor_text".to_owned(), toml::Value::String(text.clone()));
    }
    if let Some(offset) = tab.preview_anchor_offset {
        table.insert("preview_anchor_offset".to_owned(), toml::Value::Float(f64::from(offset)));
    }
    if let Some(content) = &tab.clean_untitled_content {
        table.insert("clean_untitled_content".to_owned(), toml::Value::String(content.clone()));
    }
    Ok(toml::Value::Table(table))
}

fn parse_disk_revision(
    table: &toml::value::Table,
) -> Result<crate::snapshot::PersistedDiskRevision, String> {
    Ok(crate::snapshot::PersistedDiskRevision {
        size: req_u64(table, "size")?,
        modified_unix_secs: opt_i64(table, "modified_unix_secs")?,
        modified_unix_nanos: opt_u32(table, "modified_unix_nanos")?.unwrap_or(0),
        content_hash_hex: opt_str(table, "content_hash_hex")?.unwrap_or_default().to_owned(),
        file_device: opt_u64(table, "file_device")?,
        file_inode: opt_u64(table, "file_inode")?,
    })
}

fn disk_revision_to_toml(
    rev: &crate::snapshot::PersistedDiskRevision,
) -> Result<toml::Value, String> {
    let mut table = toml::value::Table::new();
    table.insert("size".to_owned(), toml_int_u64("size", rev.size)?);
    if let Some(secs) = rev.modified_unix_secs {
        table.insert("modified_unix_secs".to_owned(), toml::Value::Integer(secs));
    }
    table.insert(
        "modified_unix_nanos".to_owned(),
        toml::Value::Integer(i64::from(rev.modified_unix_nanos)),
    );
    table.insert("content_hash_hex".to_owned(), toml::Value::String(rev.content_hash_hex.clone()));
    if let Some(device) = rev.file_device {
        table.insert("file_device".to_owned(), toml_int_u64("file_device", device)?);
    }
    if let Some(inode) = rev.file_inode {
        table.insert("file_inode".to_owned(), toml_int_u64("file_inode", inode)?);
    }
    Ok(toml::Value::Table(table))
}

fn req_u32(table: &toml::value::Table, key: &str) -> Result<u32, String> {
    opt_u32(table, key)?.ok_or_else(|| format!("missing required field `{key}`"))
}

fn req_u64(table: &toml::value::Table, key: &str) -> Result<u64, String> {
    opt_u64(table, key)?.ok_or_else(|| format!("missing required field `{key}`"))
}

fn req_usize(table: &toml::value::Table, key: &str) -> Result<usize, String> {
    opt_usize(table, key)?.ok_or_else(|| format!("missing required field `{key}`"))
}

fn req_bool(table: &toml::value::Table, key: &str) -> Result<bool, String> {
    opt_bool(table, key)?.ok_or_else(|| format!("missing required field `{key}`"))
}

fn opt_u32(table: &toml::value::Table, key: &str) -> Result<Option<u32>, String> {
    table
        .get(key)
        .map(|v| {
            v.as_integer()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("field `{key}` must be a u32 integer"))
        })
        .transpose()
}

fn opt_u64(table: &toml::value::Table, key: &str) -> Result<Option<u64>, String> {
    table
        .get(key)
        .map(|v| {
            v.as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .ok_or_else(|| format!("field `{key}` must be a u64 integer"))
        })
        .transpose()
}

fn opt_i64(table: &toml::value::Table, key: &str) -> Result<Option<i64>, String> {
    table
        .get(key)
        .map(|v| v.as_integer().ok_or_else(|| format!("field `{key}` must be an integer")))
        .transpose()
}

fn opt_usize(table: &toml::value::Table, key: &str) -> Result<Option<usize>, String> {
    table
        .get(key)
        .map(|v| {
            v.as_integer()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| format!("field `{key}` must be a non-negative integer"))
        })
        .transpose()
}

fn opt_bool(table: &toml::value::Table, key: &str) -> Result<Option<bool>, String> {
    table
        .get(key)
        .map(|v| v.as_bool().ok_or_else(|| format!("field `{key}` must be a boolean")))
        .transpose()
}

fn opt_f32(table: &toml::value::Table, key: &str) -> Result<Option<f32>, String> {
    table
        .get(key)
        .map(|v| match v {
            toml::Value::Float(n) => Ok(*n as f32),
            toml::Value::Integer(n) => Ok(*n as f32),
            _ => Err(format!("field `{key}` must be a number")),
        })
        .transpose()
}

fn opt_str<'a>(table: &'a toml::value::Table, key: &str) -> Result<Option<&'a str>, String> {
    table
        .get(key)
        .map(|v| v.as_str().ok_or_else(|| format!("field `{key}` must be a string")))
        .transpose()
}

fn opt_str_vec(table: &toml::value::Table, key: &str) -> Result<Option<Vec<String>>, String> {
    table
        .get(key)
        .map(|v| {
            v.as_array()
                .ok_or_else(|| format!("field `{key}` must be an array"))?
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("field `{key}` entries must be strings"))
                })
                .collect()
        })
        .transpose()
}

fn path_to_toml(field: &str, path: &std::path::Path) -> Result<toml::Value, String> {
    let text = path.to_str().ok_or_else(|| format!("field `{field}` path is not valid UTF-8"))?;
    Ok(toml::Value::String(text.to_owned()))
}

fn toml_int_usize(field: &str, value: usize) -> Result<toml::Value, String> {
    i64::try_from(value)
        .map(toml::Value::Integer)
        .map_err(|_| format!("field `{field}` exceeds TOML integer range"))
}

fn toml_int_u64(field: &str, value: u64) -> Result<toml::Value, String> {
    i64::try_from(value)
        .map(toml::Value::Integer)
        .map_err(|_| format!("field `{field}` exceeds TOML integer range"))
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

    #[test]
    fn non_generic_toml_roundtrip_and_golden_compat() {
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
original_file_size = 42
original_mtime_secs = 1234567890
unsaved_lines = ["legacy line"]
active_plugin = "editor"

[tabs.original_disk_revision]
size = 42
modified_unix_secs = 1234567890
modified_unix_nanos = 123
content_hash_hex = "0000"
file_device = 7
file_inode = 8
"#;
        let parsed =
            PersistedWorkspace::from_toml_str(golden_toml).expect("golden TOML should parse");
        assert_eq!(parsed.version, 1);
        assert!(parsed.sidebar_pinned);
        assert_eq!(parsed.sidebar_width, Some(280.0));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].original_disk_revision.as_ref().map(|r| r.size), Some(42));

        let reserialized = parsed.to_toml_string_pretty().expect("workspace should serialize");
        assert!(reserialized.contains("[[tabs]]"));
        let reparsed =
            PersistedWorkspace::from_toml_str(&reserialized).expect("roundtrip should parse");
        assert_eq!(reparsed.entries.len(), 1);
        assert_eq!(reparsed.entries[0].file_path, Some(PathBuf::from("/tmp/test.txt")));
        assert_eq!(
            reparsed.entries[0].original_disk_revision,
            parsed.entries[0].original_disk_revision
        );

        let minimal = PersistedWorkspace::from_toml_str("version = 1\nactive_index = 0\n")
            .expect("legacy minimal workspace should parse");
        assert!(!minimal.sidebar_pinned);
        assert!(minimal.entries.is_empty());
    }
}
