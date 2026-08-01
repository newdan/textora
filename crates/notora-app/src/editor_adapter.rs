//! notora 产品层的 editor runtime 插件与路径路由组装。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use appkit_shell::editor_plugin::EditorPluginFactory;
use appkit_shell::editor_runtime::EditorRuntime;
use appkit_shell::prepared_tab::PreparedTab;
use appkit_shell::tab_runtime::TabRuntime;
use appkit_shell::view_route::{ViewPathMatcher, ViewRouteError, ViewRouteRule, ViewRouteTable};
use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP};

const MINDMAP_ROUTE_PRIORITY: u16 = 300;
const MARKDOWN_ROUTE_PRIORITY: u16 = 200;
const TEXT_ROUTE_PRIORITY: u16 = 100;

/// 读取完成、尚未触及 UI/runtime 的不可变文档输入。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "N3-7 installs these completed inputs after preview selection")]
pub struct LoadedDocument {
    pub path: PathBuf,
    pub contents: String,
    pub disk_revision: Option<appkit_core::file_safety::DiskRevision>,
}

/// 文件读取和 `PreparedTab` 构造失败。
#[derive(Debug)]
#[allow(dead_code, reason = "N3-7 surfaces preparation failures through product events")]
pub(crate) enum DocumentPreparationError {
    Read { path: PathBuf, source: std::io::Error },
    Revision { path: PathBuf, source: appkit_core::file_safety::FileSafetyError },
    Buffer { message: String },
}

impl std::fmt::Display for DocumentPreparationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::Revision { path, source } => {
                write!(
                    formatter,
                    "could not capture disk revision for {}: {source}",
                    path.display()
                )
            }
            Self::Buffer { message } => {
                write!(formatter, "could not prepare document buffer: {message}")
            }
        }
    }
}

impl std::error::Error for DocumentPreparationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Revision { source, .. } => Some(source),
            Self::Buffer { .. } => None,
        }
    }
}

/// 此函数可在后台 worker 执行；失败时不会创建 runtime tab。
#[allow(dead_code, reason = "N3-7 schedules file reads before tab installation")]
pub(crate) fn load_document(path: &Path) -> Result<LoadedDocument, DocumentPreparationError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|source| DocumentPreparationError::Read { path: path.to_path_buf(), source })?;
    let disk_revision = appkit_core::file_safety::capture_revision(path).map_err(|source| {
        DocumentPreparationError::Revision { path: path.to_path_buf(), source }
    })?;
    Ok(LoadedDocument { path: path.to_path_buf(), contents, disk_revision: Some(disk_revision) })
}

/// 仅在主线程调用：将已读取文本与产品路由组合为完整 `PreparedTab`。
#[allow(dead_code, reason = "N3-7 installs preview tabs from this main-thread adapter")]
pub(crate) fn prepare_loaded_document(
    runtime: &EditorRuntime,
    loaded: LoadedDocument,
) -> Result<PreparedTab, DocumentPreparationError> {
    let mut text_buffer = core::buffer::TextBuffer::new(false)
        .map_err(|error| DocumentPreparationError::Buffer { message: error.to_string() })?;
    text_buffer.write_raw(loaded.contents.as_bytes());
    let mut document = appkit_core::document::DocumentModel::new(text_buffer);
    document.file_path = Some(loaded.path.clone());
    document.disk_revision = loaded.disk_revision;
    document.set_language_from_path(&loaded.path);
    Ok(PreparedTab::new(document, TabRuntime::new(runtime.create_plugin_for_path(&loaded.path))))
}

/// 创建 notora 专属的 plugin registry 与有序路径路由。
pub(crate) fn build_editor_plugins()
-> Result<(ui::plugin::PluginRegistry, ViewRouteTable), ViewRouteError> {
    let mut plugin_registry = ui::plugin::PluginRegistry::new();
    plugin_registry.register(Box::new(EditorPluginFactory));
    plugin_registry.register(Box::new(textora_markdown::mindmap_view::MindmapPluginFactory));
    plugin_registry.register(Box::new(textora_markdown::view::MarkdownEditorViewFactory));
    let registered_plugin_ids =
        HashSet::from([PLUGIN_EDITOR, PLUGIN_MINDMAP, PLUGIN_MARKDOWN_EDITOR]);
    let view_routes = ViewRouteTable::new(
        vec![
            ViewRouteRule {
                matcher: ViewPathMatcher::FileNameSuffix(".mmap.md"),
                default_plugin: PLUGIN_MINDMAP,
                toggle_target: Some(PLUGIN_MARKDOWN_EDITOR),
                priority: MINDMAP_ROUTE_PRIORITY,
            },
            ViewRouteRule {
                matcher: ViewPathMatcher::Extension("md"),
                default_plugin: PLUGIN_MARKDOWN_EDITOR,
                toggle_target: Some(PLUGIN_EDITOR),
                priority: MARKDOWN_ROUTE_PRIORITY,
            },
            ViewRouteRule {
                matcher: ViewPathMatcher::Extension("txt"),
                default_plugin: PLUGIN_EDITOR,
                toggle_target: None,
                priority: TEXT_ROUTE_PRIORITY,
            },
        ],
        &registered_plugin_ids,
    )?;
    Ok((plugin_registry, view_routes))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::build_editor_plugins;
    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP};

    #[test]
    fn mindmap_suffix_has_priority_over_markdown_extension() {
        let (_, routes) = build_editor_plugins().expect("product routes should be valid");

        assert_eq!(
            routes.resolve(Path::new("mindmap.mmap.md")).map(|route| route.default_plugin),
            Some(PLUGIN_MINDMAP)
        );
        assert_eq!(
            routes.resolve(Path::new("document.md")).map(|route| route.default_plugin),
            Some(PLUGIN_MARKDOWN_EDITOR)
        );
        assert_eq!(
            routes.resolve(Path::new("document.txt")).map(|route| route.default_plugin),
            Some(PLUGIN_EDITOR)
        );
    }
}
