//! `EditorRuntime` 的产品无关公共契约。

use std::ops::Range;
use std::path::PathBuf;

use appkit_core::workspace::types::TabId;
use smallvec::SmallVec;

use crate::event::ShellEffect;
use crate::view_route::{ViewRouteError, ViewRouteTable};
use crate::workspace::CloseTabDecision;

/// Runtime 的构造输入，由产品层负责解析并注入。
pub struct EditorRuntimeConfig {
    pub plugin_registry: ui::plugin::PluginRegistry,
    pub view_routes: ViewRouteTable,
    pub initial_settings: ui::settings::Settings,
    pub initial_theme: ui::Theme,
    pub snapshots_directory: PathBuf,
}

/// 打开文档时的 tab 生命周期策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenDisposition {
    Preview,
    Persistent,
}

/// 产品传入的编辑器键盘焦点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorFocus {
    Inactive,
    Active,
}

/// 一次窗口事件的编辑器输入上下文。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorInputContext {
    pub editor_rect: ui::Rect,
    pub focus: EditorFocus,
    pub modal_blocked: bool,
}

/// 关闭确认结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseConfirmation {
    Saved,
    Discard,
    Cancel,
}

/// 产品用于展示和保存调度的文档摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorDocumentSummary {
    pub tab_id: TabId,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub content_revision: u64,
    pub disk_revision: Option<appkit_core::file_safety::DiskRevision>,
    pub pinned: bool,
}

/// 产品读取的不可变正文快照；revision 与文本必须成对使用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTextSnapshot {
    pub tab_id: TabId,
    pub text: String,
    pub content_revision: u64,
}

/// 产品提交的 revision-checked 正文范围替换请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTextReplacement {
    pub tab_id: TabId,
    pub content_revision: u64,
    pub range: Range<usize>,
    pub replacement: String,
}

/// 正文替换被拒绝时的稳定错误分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentTextEditError {
    UnknownTab { tab_id: TabId },
    StaleRevision { expected: u64, actual: u64 },
    InvalidByteRange { range: Range<usize>, text_length: usize },
    ReadOnly { tab_id: TabId },
}

/// 产品持久化适配器所需的单个编辑器 tab 只读快照。
#[derive(Debug, Clone, PartialEq)]
pub struct EditorTabSnapshot {
    pub tab_id: TabId,
    pub path: Option<PathBuf>,
    pub suggested_file_name: Option<String>,
    pub cursor_offset: usize,
    pub selection_anchor: Option<usize>,
    pub cursor_line: usize,
    pub cursor_column: usize,
    pub dirty: bool,
    pub disk_revision: Option<appkit_core::file_safety::DiskRevision>,
    pub dirty_snapshot_id: Option<String>,
    pub scroll_anchor_line: usize,
    pub scroll_anchor_offset: f32,
    pub preview_anchor_text: Option<String>,
    pub plugin_name: String,
    pub default_plugin_name: Option<String>,
    pub allows_editing: bool,
    pub content_lines: Vec<String>,
    pub clean_untitled_content: Option<String>,
}

/// runtime 持有的 workspace 只读快照，不暴露 Workspace 或 TabRuntimeStore。
#[derive(Debug, Clone, PartialEq)]
pub struct EditorWorkspaceSnapshot {
    pub active_index: usize,
    pub tabs: Vec<EditorTabSnapshot>,
}

/// Runtime 向产品发送的类型化通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorNotification {
    ActiveDocumentChanged { tab_id: Option<TabId> },
    ContentChanged { tab_id: TabId, content_revision: u64 },
    PathChanged { tab_id: TabId, path: PathBuf },
    DirtyChanged { tab_id: TabId, dirty: bool },
    SaveCompleted { tab_id: TabId, content_revision: u64 },
    SaveFailed { tab_id: TabId, message: String },
    CloseRequested { tab_id: TabId, decision: CloseTabDecision },
}

/// 一次 runtime 操作产生的通用 effect 与产品通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorOutcome {
    pub shell_effect: ShellEffect,
    pub notifications: SmallVec<[EditorNotification; 4]>,
}

impl Default for EditorOutcome {
    fn default() -> Self {
        Self { shell_effect: ShellEffect::NONE, notifications: SmallVec::new() }
    }
}

impl EditorOutcome {
    pub fn with_notification(notification: EditorNotification) -> Self {
        Self { notifications: SmallVec::from_vec(vec![notification]), ..Self::default() }
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.shell_effect = self.shell_effect.merge(other.shell_effect);
        self.notifications.extend(other.notifications);
        self
    }
}

/// 一次编辑器指针路由产生的模型副作用和语义光标。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EditorPointerOutcome {
    pub editor: EditorOutcome,
    pub cursor_icon: Option<winit::window::CursorIcon>,
}

impl EditorPointerOutcome {
    pub fn from_editor(
        editor: EditorOutcome,
        cursor_icon: Option<winit::window::CursorIcon>,
    ) -> Self {
        Self { editor, cursor_icon }
    }
}

/// Runtime 构造和生命周期错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorRuntimeError {
    InvalidRoute(ViewRouteError),
    WindowCreation { message: String },
    GpuInitialization { message: String },
    FontInitialization { message: String },
    Domain { message: String },
}

impl std::fmt::Display for EditorRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRoute(error) => write!(formatter, "invalid editor route: {error:?}"),
            Self::WindowCreation { message } => {
                write!(formatter, "window creation failed: {message}")
            }
            Self::GpuInitialization { message } => {
                write!(formatter, "GPU initialization failed: {message}")
            }
            Self::FontInitialization { message } => {
                write!(formatter, "font initialization failed: {message}")
            }
            Self::Domain { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for EditorRuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::workspace::types::TabIdAllocator;

    fn runtime_config() -> EditorRuntimeConfig {
        let registry = ui::plugin::PluginRegistry::new();
        let registered_plugin_ids = std::collections::HashSet::new();
        let view_routes = ViewRouteTable::new(Vec::new(), &registered_plugin_ids)
            .expect("an empty route table must be valid");
        EditorRuntimeConfig {
            plugin_registry: registry,
            view_routes,
            initial_settings: ui::Settings::new(),
            initial_theme: ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark()),
            snapshots_directory: PathBuf::from("snapshots"),
        }
    }

    #[test]
    fn config_owns_product_supplied_runtime_inputs() {
        let config = runtime_config();

        assert_eq!(config.initial_settings.tab_width, 4);
        assert_eq!(config.snapshots_directory, PathBuf::from("snapshots"));
    }

    #[test]
    fn input_context_distinguishes_focus_and_modal_gate() {
        let context = EditorInputContext {
            editor_rect: ui::Rect::new(17.0, 23.0, 640.0, 480.0),
            focus: EditorFocus::Active,
            modal_blocked: true,
        };

        assert_eq!(context.focus, EditorFocus::Active);
        assert!(context.modal_blocked);
        assert_eq!(context.editor_rect.x, 17.0);
    }

    #[test]
    fn outcome_merges_effects_and_keeps_typed_notifications() {
        let mut allocator = TabIdAllocator::new();
        let tab_id = allocator.allocate();
        let outcome = EditorOutcome::with_notification(EditorNotification::ContentChanged {
            tab_id,
            content_revision: 3,
        })
        .merge(EditorOutcome {
            shell_effect: ShellEffect::RESHAPE,
            notifications: SmallVec::new(),
        });

        assert!(outcome.shell_effect.reshape);
        assert_eq!(outcome.notifications.len(), 1);
        assert!(matches!(
            outcome.notifications[0],
            EditorNotification::ContentChanged { content_revision: 3, .. }
        ));
    }

    #[test]
    fn close_confirmation_is_explicit_and_non_boolean() {
        assert_ne!(CloseConfirmation::Saved, CloseConfirmation::Discard);
        assert_ne!(CloseConfirmation::Discard, CloseConfirmation::Cancel);
    }
}
