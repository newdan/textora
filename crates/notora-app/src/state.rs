use std::collections::BTreeSet;

use notora_core::{
    CatalogCard, CatalogCardCursor, CatalogNavigationTree, DocumentIdentity, DocumentKind,
    NavigationScope, NoteEditorMetadata, TagSummary, TagWithActiveNoteCount,
};
use serde::{Deserialize, Serialize};
use ui::core::widget::SensitiveText;

use crate::action::{
    CardQuery, ConflictResolution, DocumentLoadRequest, EncryptedNoteUnlockRequest,
    NoteCreationTarget, NotoraAction, NotoraEffect, SaveConflictRequest,
    WorkspaceTransitionRequest, move_note_command,
};
use crate::effect_executor::ExternalOpenRequest;
use crate::external_files::ExternalFileSessions;

pub(crate) fn normalize_notora_title(title: &str) -> String {
    let trimmed_title = title.trim();
    if trimmed_title.is_empty() {
        return "无标题".to_owned();
    }
    trimmed_title.to_owned()
}

/// 当前键盘输入应交给的唯一目标。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusTarget {
    NavigationSearch,
    #[default]
    NavigationTree,
    CardList,
    Editor,
    EditorTitle,
    EditorTag,
    Overlay,
}

/// 不可重叠的产品 overlay 状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayState {
    #[default]
    None,
    Settings,
    NewDocumentMenu,
    EncryptedNoteDialog,
    NewWorkspace,
    TrashPermanentDeletionConfirmation {
        operation: crate::action::TrashOperation,
    },
    TrashRestoreConflictConfirmation {
        note_id: notora_core::NoteId,
    },
    SaveConflict,
}

/// 响应式三栏壳的互斥布局模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResponsiveLayoutMode {
    #[default]
    ThreePane,
    NavigationOverlay,
    EditorOverlay,
}

/// 紧凑布局中间栏与编辑器之间的唯一可见内容。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactContent {
    #[default]
    CardList,
    Editor,
}

/// 紧凑布局的左侧导航抽屉状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactNavigation {
    #[default]
    Hidden,
    Visible,
}

/// 宽屏三栏模式下左侧导航栏的可见状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationPaneVisibility {
    #[default]
    Expanded,
    Collapsed,
}

/// 分隔条影响的相邻 pane。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Navigation,
    CardList,
}

/// 是否已经存在可供笔记读写的工作区根目录。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceRootState {
    #[default]
    Missing,
    Active,
}

/// 同一时刻唯一的目录创建草稿或在途命令。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DirectoryCreationState {
    #[default]
    Inactive,
    Editing {
        parent_relative_path: std::path::PathBuf,
        draft_name: String,
    },
    Submitting {
        parent_relative_path: std::path::PathBuf,
        directory_name: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceCreationState {
    #[default]
    Inactive,
    Editing {
        name: String,
        parent_directory: Option<std::path::PathBuf>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceTransitionState {
    #[default]
    Idle,
    AwaitingDirtySaves {
        request: WorkspaceTransitionRequest,
    },
    Applying {
        request: WorkspaceTransitionRequest,
    },
}

/// 加密笔记创建中的唯一敏感草稿；离开弹窗时整体销毁。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum EncryptedNoteCreationState {
    #[default]
    Inactive,
    Editing {
        target: NoteCreationTarget,
        password: SensitiveText,
        confirmation: SensitiveText,
        error_message: Option<String>,
        failure_generation: u64,
        next_generation: u64,
    },
    Submitting {
        target: NoteCreationTarget,
        generation: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum EncryptedNoteUnlockState {
    #[default]
    Inactive,
    Editing {
        request: DocumentLoadRequest,
        title: String,
        metadata: NoteEditorMetadata,
        tags: Vec<TagSummary>,
        password: SensitiveText,
        error_message: Option<String>,
        failure_generation: u64,
        next_generation: u64,
    },
    Submitting {
        request: DocumentLoadRequest,
        title: String,
        metadata: NoteEditorMetadata,
        tags: Vec<TagSummary>,
        generation: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum EncryptedConflictCopyState {
    #[default]
    Inactive,
    Editing {
        identity: DocumentIdentity,
        target_path: std::path::PathBuf,
        password: SensitiveText,
        confirmation: SensitiveText,
        error_message: Option<String>,
        failure_generation: u64,
        next_generation: u64,
    },
    Submitting {
        identity: DocumentIdentity,
        target_path: std::path::PathBuf,
        generation: u64,
    },
}

/// 只包含产品导航、选择和卡片查询状态；编辑会话保留在 EditorRuntime。
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryState {
    pub navigation_scope: NavigationScope,
    pub search_scope_before_search: Option<NavigationScope>,
    pub search_text: String,
    pub card_page: CardPageState,
    pub card_scroll_offset_px: f32,
    pub selected_card: Option<DocumentIdentity>,
    pub title_draft: Option<String>,
    pub pending_title_commit: Option<PendingTitleCommit>,
    pub selected_document_generation: u64,
    pub active_editor_metadata: Option<ActiveEditorMetadata>,
    pub last_command_error: Option<String>,
    pub save_conflict: Option<SaveConflict>,
    pub navigation_tree: NavigationTreeState,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            navigation_scope: NavigationScope::WorkspaceRoot,
            search_scope_before_search: None,
            search_text: String::new(),
            card_page: CardPageState::Idle,
            card_scroll_offset_px: 0.0,
            selected_card: None,
            title_draft: None,
            pending_title_commit: None,
            selected_document_generation: 0,
            active_editor_metadata: None,
            last_command_error: None,
            save_conflict: None,
            navigation_tree: NavigationTreeState::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingTitleCommit {
    pub identity: DocumentIdentity,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveEditorMetadata {
    pub identity: DocumentIdentity,
    pub selection_generation: u64,
    pub metadata: NoteEditorMetadata,
    pub tags: Vec<TagSummary>,
}

/// 左栏的数据快照，由 app 接收 worker DTO 后存入；render 不读 catalog。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationTreeState {
    pub directories: Vec<std::path::PathBuf>,
    pub tags: Vec<TagWithActiveNoteCount>,
    pub workspace_root_expanded: bool,
    pub tag_root_expanded: bool,
    pub expanded_directories: BTreeSet<std::path::PathBuf>,
}

impl Default for NavigationTreeState {
    fn default() -> Self {
        Self {
            directories: Vec::new(),
            tags: Vec::new(),
            workspace_root_expanded: true,
            tag_root_expanded: false,
            expanded_directories: BTreeSet::new(),
        }
    }
}

/// 中栏数据加载状态；任何时刻只表示一种结果，避免 loading/failed bool 组合。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CardPageState {
    #[default]
    Idle,
    LoadingInitial {
        query: CardQuery,
    },
    LoadingNextPage {
        query: CardQuery,
        cards: Vec<CatalogCard>,
    },
    Refreshing {
        query: CardQuery,
        cards: Vec<CatalogCard>,
        next_cursor: Option<CatalogCardCursor>,
    },
    Empty {
        query: CardQuery,
    },
    Ready {
        query: CardQuery,
        cards: Vec<CatalogCard>,
        next_cursor: Option<CatalogCardCursor>,
    },
    Failed {
        query: CardQuery,
        cards: Vec<CatalogCard>,
        next_cursor: Option<CatalogCardCursor>,
        message: String,
    },
}

/// 供 shell 展示的冲突摘要；tab 映射仍保留在产品层 registry。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveConflict {
    pub identity: DocumentIdentity,
    pub content_revision: u64,
}

/// 仅保存窗口壳的纯布局状态。
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutState {
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
    pub navigation_pane_visibility: NavigationPaneVisibility,
    pub responsive_mode: ResponsiveLayoutMode,
    pub compact_content: CompactContent,
    pub compact_navigation: CompactNavigation,
    pub focus_target: FocusTarget,
    pub overlay: OverlayState,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            navigation_width_logical: 220.0,
            card_list_width_logical: 340.0,
            navigation_pane_visibility: NavigationPaneVisibility::Expanded,
            responsive_mode: ResponsiveLayoutMode::ThreePane,
            compact_content: CompactContent::CardList,
            compact_navigation: CompactNavigation::Hidden,
            focus_target: FocusTarget::NavigationTree,
            overlay: OverlayState::None,
        }
    }
}

/// notora 的纯产品状态；不持有 catalog、文件句柄或 editor session。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotoraState {
    pub library: LibraryState,
    pub workspace_root: WorkspaceRootState,
    pub directory_creation: DirectoryCreationState,
    pub workspace_creation: WorkspaceCreationState,
    pub encrypted_note_creation: EncryptedNoteCreationState,
    pub encrypted_note_unlock: EncryptedNoteUnlockState,
    pub encrypted_conflict_copy: EncryptedConflictCopyState,
    pub workspace_transition: WorkspaceTransitionState,
    pub workspace_root_path: Option<std::path::PathBuf>,
    /// 外部 session 不属于 catalog、搜索、星标、标签或 Trash 的任何一项。
    pub external_files: ExternalFileSessions,
    pub layout: LayoutState,
}

impl NotoraState {
    pub(crate) fn activate_workspace(&mut self) {
        self.library = LibraryState::default();
        self.workspace_root = WorkspaceRootState::Active;
        self.directory_creation = DirectoryCreationState::Inactive;
        self.workspace_creation = WorkspaceCreationState::Inactive;
        self.encrypted_note_creation = EncryptedNoteCreationState::Inactive;
        self.encrypted_note_unlock = EncryptedNoteUnlockState::Inactive;
        self.encrypted_conflict_copy = EncryptedConflictCopyState::Inactive;
        self.layout.overlay = OverlayState::None;
        self.layout.focus_target = FocusTarget::NavigationTree;
        self.layout.compact_navigation = CompactNavigation::Hidden;
        self.layout.compact_content = CompactContent::CardList;
    }

    pub fn reduce(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::NavigationSelected(scope) => self.select_navigation_scope(scope),
            NotoraAction::SearchTextChanged(query) => {
                self.library.search_text = query;
                self.layout.focus_target = FocusTarget::NavigationSearch;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SearchCommitted { query, search_generation } => {
                self.commit_search(query, search_generation)
            }
            NotoraAction::CardQueryCompleted { query, page } => self.apply_card_page(query, page),
            NotoraAction::CardQueryFailed { query, message } => {
                self.apply_card_query_failure(query, message)
            }
            NotoraAction::NavigationTreeLoaded(tree) => self.apply_navigation_tree(tree),
            NotoraAction::NavigationTreeFailed(message) => {
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CatalogRecoveryNotified(message) => {
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceRootExpansionToggled => {
                self.library.navigation_tree.workspace_root_expanded =
                    !self.library.navigation_tree.workspace_root_expanded;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::TagRootExpansionToggled => {
                self.library.navigation_tree.tag_root_expanded =
                    !self.library.navigation_tree.tag_root_expanded;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::NavigationExpansionToggled(relative_path) => {
                self.toggle_navigation_expansion(relative_path)
            }
            NotoraAction::BeginDirectoryCreation { parent_relative_path } => {
                self.begin_directory_creation(parent_relative_path)
            }
            NotoraAction::DirectoryCreationTextChanged(draft_name) => {
                self.update_directory_creation_text(draft_name)
            }
            NotoraAction::DirectoryCreationCommitRequested => self.commit_directory_creation(),
            NotoraAction::DirectoryCreationCancelled => {
                self.directory_creation = DirectoryCreationState::Inactive;
                self.library.last_command_error = None;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::DirectoryCreationCompleted { relative_path } => {
                self.complete_directory_creation(relative_path)
            }
            NotoraAction::DirectoryCreationFailed(message) => self.fail_directory_creation(message),
            NotoraAction::OpenWorkspaceCreationRequested => {
                if self.workspace_transition != WorkspaceTransitionState::Idle {
                    return vec![NotoraEffect::Redraw];
                }
                self.workspace_creation =
                    WorkspaceCreationState::Editing { name: String::new(), parent_directory: None };
                self.library.last_command_error = None;
                self.layout.overlay = OverlayState::NewWorkspace;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationNameChanged(name) => {
                if let WorkspaceCreationState::Editing { name: current_name, .. } =
                    &mut self.workspace_creation
                {
                    *current_name = name;
                    self.library.last_command_error = None;
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationLocationRequested => {
                if !matches!(self.workspace_creation, WorkspaceCreationState::Editing { .. }) {
                    return vec![NotoraEffect::Redraw];
                }
                vec![NotoraEffect::ChooseWorkspaceCreationLocation, NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationLocationSelected(parent_directory) => {
                if let WorkspaceCreationState::Editing {
                    parent_directory: current_parent, ..
                } = &mut self.workspace_creation
                {
                    *current_parent = Some(parent_directory);
                    self.library.last_command_error = None;
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceCreationCommitRequested => self.commit_workspace_creation(),
            NotoraAction::WorkspaceTransitionConfirmed(request) => {
                if self.workspace_transition != WorkspaceTransitionState::Idle {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                self.workspace_transition =
                    WorkspaceTransitionState::AwaitingDirtySaves { request: request.clone() };
                vec![NotoraEffect::PrepareWorkspaceTransition(request), NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceTransitionApplying => {
                let WorkspaceTransitionState::AwaitingDirtySaves { request } =
                    &self.workspace_transition
                else {
                    return vec![NotoraEffect::Redraw];
                };
                self.workspace_transition =
                    WorkspaceTransitionState::Applying { request: request.clone() };
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceTransitionCompleted => {
                self.workspace_transition = WorkspaceTransitionState::Idle;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceTransitionFailed(message) => {
                self.workspace_transition = WorkspaceTransitionState::Idle;
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CardListScrolled { offset_px, near_end } => {
                self.library.card_scroll_offset_px = offset_px;
                if near_end {
                    return self.request_next_card_page();
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CardSelected(identity) => {
                let request = self.select_document(identity);
                self.layout.focus_target = FocusTarget::CardList;
                self.layout.compact_content = CompactContent::CardList;
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::CardActivated(identity) => {
                if self.library.selected_card != Some(identity) {
                    return vec![NotoraEffect::Redraw];
                }
                self.layout.focus_target = FocusTarget::Editor;
                self.layout.compact_content = CompactContent::Editor;
                vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
            }
            NotoraAction::ActiveEditorMetadataLoaded { request, metadata, tags } => {
                self.apply_active_editor_metadata(request, metadata, tags)
            }
            NotoraAction::ActiveEditorSaved { identity, saved_at } => {
                self.apply_active_editor_saved(identity, saved_at)
            }
            NotoraAction::CompactNavigationRequested => {
                self.layout.compact_navigation = CompactNavigation::Visible;
                self.layout.focus_target = FocusTarget::NavigationTree;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::NavigationPaneVisibilityToggled => {
                self.layout.navigation_pane_visibility =
                    match self.layout.navigation_pane_visibility {
                        NavigationPaneVisibility::Expanded => NavigationPaneVisibility::Collapsed,
                        NavigationPaneVisibility::Collapsed => NavigationPaneVisibility::Expanded,
                    };
                self.layout.focus_target = match self.layout.navigation_pane_visibility {
                    NavigationPaneVisibility::Expanded => FocusTarget::NavigationTree,
                    NavigationPaneVisibility::Collapsed => FocusTarget::CardList,
                };
                vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]
            }
            NotoraAction::CompactBackRequested => {
                self.layout.compact_content = CompactContent::CardList;
                self.layout.compact_navigation = CompactNavigation::Hidden;
                self.layout.focus_target = FocusTarget::CardList;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::OpenExternalFileDialogRequested => {
                vec![NotoraEffect::OpenExternalFiles(ExternalOpenRequest::ShowFileDialog)]
            }
            NotoraAction::ExternalPathsReceived(paths) => {
                vec![NotoraEffect::OpenExternalFiles(ExternalOpenRequest::Paths(paths))]
            }
            NotoraAction::ExternalFileOpened(identity) => {
                self.transition_navigation_scope(NavigationScope::ExternalFiles);
                let request = self.select_document(identity);
                self.layout.focus_target = FocusTarget::CardList;
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFileCloseRequested(external_file_id) => {
                if self.external_files.session(external_file_id).is_none() {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::CloseExternalFile(external_file_id), NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFileCloseCompleted(external_file_id) => {
                let identity = DocumentIdentity::ExternalFile(external_file_id);
                let _ = self.external_files.remove(external_file_id);
                if self.library.selected_card == Some(identity) {
                    self.invalidate_document_selection();
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFileCloseFailed(message) => {
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFilesClearRequested => {
                if self.external_files.sessions().is_empty() {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::CloseAllExternalFiles, NotoraEffect::Redraw]
            }
            NotoraAction::ExternalFilesClearCompleted {
                closed_external_file_ids,
                blocked_count,
            } => {
                let selected_external_file_id =
                    self.library.selected_card.and_then(|identity| match identity {
                        DocumentIdentity::ExternalFile(external_file_id) => Some(external_file_id),
                        DocumentIdentity::Note(_) => None,
                    });
                let selected_record_was_closed = selected_external_file_id
                    .is_some_and(|selected| closed_external_file_ids.contains(&selected));
                for external_file_id in closed_external_file_ids {
                    let _ = self.external_files.remove(external_file_id);
                }
                if selected_record_was_closed {
                    self.invalidate_document_selection();
                }
                self.library.last_command_error = (blocked_count > 0).then(|| {
                    format!("有 {blocked_count} 个文件未清空：请先保存未保存内容或取消固定")
                });
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::PromotePreviewRequested => {
                vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
            }
            NotoraAction::WorkspaceRootSelectionRequested => {
                if self.workspace_transition != WorkspaceTransitionState::Idle {
                    return vec![NotoraEffect::Redraw];
                }
                vec![NotoraEffect::ChooseWorkspaceRoot, NotoraEffect::Redraw]
            }
            NotoraAction::OpenNewDocumentMenu => self.open_new_document_menu(),
            NotoraAction::CreateRequested(kind) => self.request_note_creation(kind),
            NotoraAction::BeginEncryptedNoteCreation => self.begin_encrypted_note_creation(),
            NotoraAction::EncryptedNotePasswordChanged(password) => {
                self.update_encrypted_note_password(password)
            }
            NotoraAction::EncryptedNoteConfirmationChanged(confirmation) => {
                self.update_encrypted_note_confirmation(confirmation)
            }
            NotoraAction::EncryptedNoteDialogSubmitRequested => self.submit_encrypted_note_dialog(),
            NotoraAction::EncryptedNoteUnlockRequired { request, title, metadata, tags } => {
                self.require_encrypted_note_unlock(request, title, metadata, tags)
            }
            NotoraAction::EncryptedConflictCopyRequired { identity, target_path } => {
                self.require_encrypted_conflict_copy(identity, target_path)
            }
            NotoraAction::TitleTextChanged(title) => {
                self.library.title_draft = Some(title);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::TitleCommitRequested(title) => self.request_title_commit(title),
            NotoraAction::ToggleSourceViewRequested => {
                self.layout.focus_target = FocusTarget::Editor;
                vec![NotoraEffect::ToggleEditorView]
            }
            NotoraAction::ToggleMindmapStylePanelRequested => {
                vec![NotoraEffect::ToggleMindmapStylePanel, NotoraEffect::Redraw]
            }
            NotoraAction::MindmapStylePanel(action) => {
                vec![NotoraEffect::DispatchMindmapStylePanel(action), NotoraEffect::Redraw]
            }
            NotoraAction::SemanticEditRequested(command) => {
                self.layout.focus_target = FocusTarget::Editor;
                vec![NotoraEffect::ExecuteSemanticEdit(command), NotoraEffect::Redraw]
            }
            NotoraAction::MoveDialogRequested(note_id) => {
                vec![NotoraEffect::ChooseNoteMoveDirectory(note_id), NotoraEffect::Redraw]
            }
            NotoraAction::NoteCommandCompleted(result) => {
                self.apply_note_command_completion(result)
            }
            NotoraAction::NoteCommandFailed(message) => {
                if self.fail_encrypted_note_creation(message.clone()) {
                    return vec![NotoraEffect::Redraw];
                }
                if self.fail_encrypted_note_unlock(message.clone()) {
                    return vec![NotoraEffect::Redraw];
                }
                if self.fail_encrypted_conflict_copy(message.clone()) {
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::MoveRequested { note_id, target_directory } => {
                self.library.last_command_error = None;
                vec![
                    NotoraEffect::ExecuteNoteCommand(move_note_command(note_id, target_directory)),
                    NotoraEffect::Redraw,
                ]
            }
            NotoraAction::MetadataMutationRequested(mutation) => {
                self.library.last_command_error = None;
                vec![NotoraEffect::ExecuteMetadataMutation(mutation), NotoraEffect::Redraw]
            }
            NotoraAction::MetadataMutationCompleted { note_id, metadata, selection_generation } => {
                self.apply_metadata_mutation_completion(note_id, metadata, selection_generation);
                self.library.last_command_error = None;
                if self.library.navigation_scope == NavigationScope::ExternalFiles {
                    return vec![NotoraEffect::Redraw];
                }
                self.request_card_query(CardQuery::from(self.library.navigation_scope.clone()))
            }
            NotoraAction::MetadataMutationFailed(message) => {
                self.library.pending_title_commit = None;
                self.library.last_command_error = Some(message);
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CatalogReindexed => {
                if self.library.navigation_scope == NavigationScope::ExternalFiles {
                    return vec![NotoraEffect::Redraw];
                }
                self.refresh_card_query(CardQuery::from(self.library.navigation_scope.clone()))
            }
            NotoraAction::TrashOperationRequested(operation) => {
                if matches!(
                    operation,
                    crate::action::TrashOperation::PermanentlyDelete { .. }
                        | crate::action::TrashOperation::Empty
                ) {
                    self.layout.overlay =
                        OverlayState::TrashPermanentDeletionConfirmation { operation };
                    self.layout.focus_target = FocusTarget::Overlay;
                    return vec![NotoraEffect::Redraw];
                }
                self.library.last_command_error = None;
                vec![NotoraEffect::ExecuteTrashOperation(operation), NotoraEffect::Redraw]
            }
            NotoraAction::TrashPermanentDeletionConfirmed => {
                let OverlayState::TrashPermanentDeletionConfirmation { operation } =
                    self.layout.overlay
                else {
                    return vec![NotoraEffect::Redraw];
                };
                self.layout.overlay = OverlayState::None;
                self.layout.focus_target = FocusTarget::NavigationTree;
                self.library.last_command_error = None;
                vec![NotoraEffect::ExecuteTrashOperation(operation), NotoraEffect::Redraw]
            }
            NotoraAction::TrashRestoreWithRenamedPathConfirmed => {
                let OverlayState::TrashRestoreConflictConfirmation { note_id } =
                    self.layout.overlay
                else {
                    return vec![NotoraEffect::Redraw];
                };
                self.layout.overlay = OverlayState::None;
                self.layout.focus_target = FocusTarget::NavigationTree;
                self.library.last_command_error = None;
                vec![
                    NotoraEffect::ExecuteTrashOperation(
                        crate::action::TrashOperation::RestoreWithRenamedPath { note_id },
                    ),
                    NotoraEffect::Redraw,
                ]
            }
            NotoraAction::TrashOperationCompleted => {
                self.library.last_command_error = None;
                if self.library.navigation_scope == NavigationScope::Trash {
                    self.invalidate_document_selection();
                }
                self.request_card_query(CardQuery::from(self.library.navigation_scope.clone()))
            }
            NotoraAction::TrashOperationFailed(failure) => match failure {
                crate::action::TrashOperationFailure::RestoreConflict { note_id } => {
                    self.layout.overlay =
                        OverlayState::TrashRestoreConflictConfirmation { note_id };
                    self.layout.focus_target = FocusTarget::Overlay;
                    vec![NotoraEffect::Redraw]
                }
                crate::action::TrashOperationFailure::Message(message) => {
                    self.library.last_command_error = Some(message);
                    vec![NotoraEffect::Redraw]
                }
            },
            NotoraAction::SaveConflictDetected { identity, content_revision } => {
                self.library.save_conflict = Some(SaveConflict { identity, content_revision });
                self.layout.overlay = OverlayState::SaveConflict;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SaveConflictResolutionRequested(resolution) => {
                self.resolve_save_conflict(resolution)
            }
            NotoraAction::SaveConflictResolved { identity } => {
                if self.library.save_conflict.map(|conflict| conflict.identity) == Some(identity) {
                    self.library.save_conflict = None;
                    self.encrypted_conflict_copy = EncryptedConflictCopyState::Inactive;
                    self.layout.overlay = OverlayState::None;
                    self.layout.focus_target = FocusTarget::Editor;
                }
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SplitterDragged { pane, logical_width } => {
                self.set_pane_width(pane, logical_width);
                vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]
            }
            NotoraAction::FocusRequested(focus_target) => {
                self.layout.focus_target = if self.layout.overlay == OverlayState::None {
                    focus_target
                } else {
                    FocusTarget::Overlay
                };
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::OpenSettings => {
                self.layout.overlay = OverlayState::Settings;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::ProductSettingsUpdateRequested(update) => {
                vec![NotoraEffect::ApplyProductSettingsUpdate(update), NotoraEffect::Redraw]
            }
            NotoraAction::RetryProductSettingsPersistence => {
                vec![NotoraEffect::PersistProductSettings, NotoraEffect::Redraw]
            }
            NotoraAction::SettingsViewChanged => vec![NotoraEffect::Redraw],
            NotoraAction::OverlayDismissed => self.dismiss_overlay(),
            NotoraAction::EscapePressed => self.handle_escape(),
        }
    }

    fn select_navigation_scope(&mut self, scope: NavigationScope) -> Vec<NotoraEffect> {
        if !matches!(scope, NavigationScope::Search { .. }) {
            self.library.search_scope_before_search = None;
            self.library.search_text.clear();
        }
        self.transition_navigation_scope(scope.clone());
        self.layout.focus_target = FocusTarget::NavigationTree;
        self.layout.compact_navigation = CompactNavigation::Hidden;
        self.layout.compact_content = CompactContent::CardList;
        if scope == NavigationScope::ExternalFiles {
            return vec![NotoraEffect::Redraw];
        }
        self.request_card_query(CardQuery::from(scope))
    }

    fn commit_search(
        &mut self,
        query: String,
        search_generation: Option<crate::search_controller::SearchGeneration>,
    ) -> Vec<NotoraEffect> {
        self.library.search_text = query.clone();
        if query.is_empty() {
            let scope = self
                .library
                .search_scope_before_search
                .take()
                .unwrap_or(NavigationScope::WorkspaceRoot);
            self.transition_navigation_scope(scope.clone());
            self.layout.focus_target = FocusTarget::NavigationTree;
            return self.request_card_query(CardQuery::from(scope));
        }

        if self.library.search_scope_before_search.is_none()
            && !matches!(self.library.navigation_scope, NavigationScope::Search { .. })
        {
            self.library.search_scope_before_search = Some(self.library.navigation_scope.clone());
        }
        let scope = NavigationScope::Search { query };
        self.transition_navigation_scope(scope.clone());
        self.layout.focus_target = FocusTarget::NavigationSearch;
        let mut card_query = CardQuery::from(scope);
        card_query.search_generation = search_generation;
        self.request_card_query(card_query)
    }

    fn transition_navigation_scope(&mut self, scope: NavigationScope) {
        if self.library.navigation_scope == scope {
            return;
        }
        self.library.navigation_scope = scope;
        self.invalidate_document_selection();
    }

    pub(crate) fn invalidate_document_selection(&mut self) {
        self.library.selected_card = None;
        self.library.title_draft = None;
        self.library.active_editor_metadata = None;
        self.library.selected_document_generation =
            self.library.selected_document_generation.wrapping_add(1);
    }

    fn open_new_document_menu(&mut self) -> Vec<NotoraEffect> {
        if self.workspace_root == WorkspaceRootState::Missing
            || self.library.navigation_scope == NavigationScope::Trash
        {
            return vec![NotoraEffect::Redraw];
        }
        self.library.last_command_error = None;
        self.layout.overlay = OverlayState::NewDocumentMenu;
        self.layout.focus_target = FocusTarget::Overlay;
        vec![NotoraEffect::Redraw]
    }

    fn request_note_creation(&mut self, kind: DocumentKind) -> Vec<NotoraEffect> {
        if self.workspace_root == WorkspaceRootState::Missing {
            return vec![NotoraEffect::Redraw];
        }
        if self.layout.overlay == OverlayState::NewDocumentMenu {
            self.layout.overlay = OverlayState::None;
            self.layout.focus_target = FocusTarget::CardList;
        }
        if self.library.navigation_scope == NavigationScope::ExternalFiles {
            return vec![NotoraEffect::CreateUntitledExternal(kind), NotoraEffect::Redraw];
        }
        let Some(target) = creation_target(&self.library.navigation_scope) else {
            return vec![NotoraEffect::Redraw];
        };
        self.library.last_command_error = None;
        vec![NotoraEffect::RequestNoteCreation { kind, target }, NotoraEffect::Redraw]
    }

    fn begin_encrypted_note_creation(&mut self) -> Vec<NotoraEffect> {
        if self.workspace_root == WorkspaceRootState::Missing
            || self.library.navigation_scope == NavigationScope::ExternalFiles
        {
            return vec![NotoraEffect::Redraw];
        }
        let Some(target) = creation_target(&self.library.navigation_scope) else {
            return vec![NotoraEffect::Redraw];
        };
        self.library.last_command_error = None;
        self.encrypted_note_creation = EncryptedNoteCreationState::Editing {
            target,
            password: SensitiveText::new(String::new()),
            confirmation: SensitiveText::new(String::new()),
            error_message: None,
            failure_generation: 0,
            next_generation: 1,
        };
        self.layout.overlay = OverlayState::EncryptedNoteDialog;
        self.layout.focus_target = FocusTarget::Overlay;
        vec![NotoraEffect::Redraw]
    }

    fn update_encrypted_note_password(&mut self, password: SensitiveText) -> Vec<NotoraEffect> {
        if let EncryptedNoteCreationState::Editing {
            password: current_password,
            error_message,
            ..
        } = &mut self.encrypted_note_creation
        {
            *current_password = password;
            *error_message = None;
            return vec![NotoraEffect::Redraw];
        }
        if let EncryptedNoteUnlockState::Editing {
            password: current_password, error_message, ..
        } = &mut self.encrypted_note_unlock
        {
            *current_password = password;
            *error_message = None;
            return vec![NotoraEffect::Redraw];
        }
        if let EncryptedConflictCopyState::Editing {
            password: current_password,
            error_message,
            ..
        } = &mut self.encrypted_conflict_copy
        {
            *current_password = password;
            *error_message = None;
        }
        vec![NotoraEffect::Redraw]
    }

    fn update_encrypted_note_confirmation(
        &mut self,
        confirmation: SensitiveText,
    ) -> Vec<NotoraEffect> {
        if let EncryptedNoteCreationState::Editing {
            confirmation: current_confirmation,
            error_message,
            ..
        } = &mut self.encrypted_note_creation
        {
            *current_confirmation = confirmation;
            *error_message = None;
            return vec![NotoraEffect::Redraw];
        }
        if let EncryptedConflictCopyState::Editing {
            confirmation: current_confirmation,
            error_message,
            ..
        } = &mut self.encrypted_conflict_copy
        {
            *current_confirmation = confirmation;
            *error_message = None;
        }
        vec![NotoraEffect::Redraw]
    }

    fn submit_encrypted_note_creation(&mut self) -> Vec<NotoraEffect> {
        let EncryptedNoteCreationState::Editing {
            target,
            password,
            confirmation,
            failure_generation,
            next_generation,
            ..
        } = std::mem::take(&mut self.encrypted_note_creation)
        else {
            return vec![NotoraEffect::Redraw];
        };
        let validation_error = validate_password_confirmation(&password, &confirmation);
        if let Some(error_message) = validation_error {
            self.encrypted_note_creation = EncryptedNoteCreationState::Editing {
                target,
                password,
                confirmation,
                error_message: Some(error_message),
                failure_generation,
                next_generation,
            };
            return vec![NotoraEffect::Redraw];
        }
        let encryption_password =
            match textora_encryption::EncryptionPassword::new(password.expose().to_owned()) {
                Ok(password) => password,
                Err(error) => {
                    self.encrypted_note_creation = EncryptedNoteCreationState::Editing {
                        target,
                        password,
                        confirmation,
                        error_message: Some(error.to_string()),
                        failure_generation,
                        next_generation,
                    };
                    return vec![NotoraEffect::Redraw];
                }
            };
        self.encrypted_note_creation = EncryptedNoteCreationState::Submitting {
            target: target.clone(),
            generation: next_generation,
        };
        vec![
            NotoraEffect::ExecuteNoteCommand(
                notora_core::note_command::NoteCommand::CreateConfigured(
                    notora_core::ConfiguredCreateNoteRequest {
                        kind: DocumentKind::Markdown,
                        target_directory: target.directory,
                        storage: notora_core::CreateNoteStorage::encrypted(encryption_password),
                    },
                ),
            ),
            NotoraEffect::Redraw,
        ]
    }

    fn fail_encrypted_note_creation(&mut self, message: String) -> bool {
        let EncryptedNoteCreationState::Submitting { target, generation } =
            std::mem::take(&mut self.encrypted_note_creation)
        else {
            return false;
        };
        self.encrypted_note_creation = EncryptedNoteCreationState::Editing {
            target,
            password: SensitiveText::new(String::new()),
            confirmation: SensitiveText::new(String::new()),
            error_message: Some(message),
            failure_generation: generation,
            next_generation: generation.wrapping_add(1),
        };
        true
    }

    fn fail_encrypted_note_unlock(&mut self, message: String) -> bool {
        let EncryptedNoteUnlockState::Submitting { request, title, metadata, tags, generation } =
            std::mem::take(&mut self.encrypted_note_unlock)
        else {
            return false;
        };
        self.encrypted_note_unlock = EncryptedNoteUnlockState::Editing {
            request,
            title,
            metadata,
            tags,
            password: SensitiveText::new(String::new()),
            error_message: Some(message),
            failure_generation: generation,
            next_generation: generation.wrapping_add(1),
        };
        true
    }

    fn fail_encrypted_conflict_copy(&mut self, message: String) -> bool {
        let EncryptedConflictCopyState::Submitting { identity, target_path, generation } =
            std::mem::take(&mut self.encrypted_conflict_copy)
        else {
            return false;
        };
        self.encrypted_conflict_copy = EncryptedConflictCopyState::Editing {
            identity,
            target_path,
            password: SensitiveText::new(String::new()),
            confirmation: SensitiveText::new(String::new()),
            error_message: Some(message),
            failure_generation: generation,
            next_generation: generation.wrapping_add(1),
        };
        true
    }

    fn submit_encrypted_note_dialog(&mut self) -> Vec<NotoraEffect> {
        if matches!(self.encrypted_note_creation, EncryptedNoteCreationState::Editing { .. }) {
            return self.submit_encrypted_note_creation();
        }
        if matches!(self.encrypted_conflict_copy, EncryptedConflictCopyState::Editing { .. }) {
            return self.submit_encrypted_conflict_copy();
        }
        self.submit_encrypted_note_unlock()
    }

    fn require_encrypted_conflict_copy(
        &mut self,
        identity: DocumentIdentity,
        target_path: std::path::PathBuf,
    ) -> Vec<NotoraEffect> {
        if self.library.save_conflict.map(|conflict| conflict.identity) != Some(identity) {
            return vec![NotoraEffect::Redraw];
        }
        self.encrypted_conflict_copy = EncryptedConflictCopyState::Editing {
            identity,
            target_path,
            password: SensitiveText::new(String::new()),
            confirmation: SensitiveText::new(String::new()),
            error_message: None,
            failure_generation: 0,
            next_generation: 1,
        };
        self.layout.overlay = OverlayState::EncryptedNoteDialog;
        self.layout.focus_target = FocusTarget::Overlay;
        vec![NotoraEffect::Redraw]
    }

    fn submit_encrypted_conflict_copy(&mut self) -> Vec<NotoraEffect> {
        let EncryptedConflictCopyState::Editing {
            identity,
            target_path,
            password,
            confirmation,
            failure_generation,
            next_generation,
            ..
        } = std::mem::take(&mut self.encrypted_conflict_copy)
        else {
            return vec![NotoraEffect::Redraw];
        };
        let validation_error = validate_password_confirmation(&password, &confirmation);
        let Ok(encryption_password) =
            textora_encryption::EncryptionPassword::new(password.expose().to_owned())
        else {
            return self.restore_invalid_conflict_copy(EncryptedConflictCopyState::Editing {
                identity,
                target_path,
                password,
                confirmation,
                error_message: Some(
                    validation_error.unwrap_or_else(|| "密码不符合要求".to_owned()),
                ),
                failure_generation,
                next_generation,
            });
        };
        if let Some(message) = validation_error {
            return self.restore_invalid_conflict_copy(EncryptedConflictCopyState::Editing {
                identity,
                target_path,
                password,
                confirmation,
                error_message: Some(message),
                failure_generation,
                next_generation,
            });
        }
        self.encrypted_conflict_copy = EncryptedConflictCopyState::Submitting {
            identity,
            target_path: target_path.clone(),
            generation: next_generation,
        };
        vec![
            NotoraEffect::SaveEncryptedConflictCopy(crate::action::EncryptedConflictCopyRequest {
                identity,
                target_path,
                password: std::sync::Arc::new(encryption_password),
                generation: next_generation,
            }),
            NotoraEffect::Redraw,
        ]
    }

    fn restore_invalid_conflict_copy(
        &mut self,
        editing: EncryptedConflictCopyState,
    ) -> Vec<NotoraEffect> {
        self.encrypted_conflict_copy = editing;
        vec![NotoraEffect::Redraw]
    }

    fn require_encrypted_note_unlock(
        &mut self,
        request: DocumentLoadRequest,
        title: String,
        metadata: NoteEditorMetadata,
        tags: Vec<TagSummary>,
    ) -> Vec<NotoraEffect> {
        if self.library.selected_card != Some(request.identity)
            || self.library.selected_document_generation != request.selection_generation
        {
            return vec![NotoraEffect::Redraw];
        }
        self.library.active_editor_metadata = Some(ActiveEditorMetadata {
            identity: request.identity,
            selection_generation: request.selection_generation,
            metadata: metadata.clone(),
            tags: tags.clone(),
        });
        self.encrypted_note_unlock = EncryptedNoteUnlockState::Editing {
            request,
            title,
            metadata,
            tags,
            password: SensitiveText::new(String::new()),
            error_message: None,
            failure_generation: 0,
            next_generation: 1,
        };
        self.layout.overlay = OverlayState::EncryptedNoteDialog;
        self.layout.focus_target = FocusTarget::Overlay;
        vec![NotoraEffect::Redraw]
    }

    fn submit_encrypted_note_unlock(&mut self) -> Vec<NotoraEffect> {
        let EncryptedNoteUnlockState::Editing {
            request,
            title,
            metadata,
            tags,
            password,
            failure_generation,
            next_generation,
            ..
        } = std::mem::take(&mut self.encrypted_note_unlock)
        else {
            return vec![NotoraEffect::Redraw];
        };
        if let Some(error_message) = validate_password_minimum(&password) {
            self.encrypted_note_unlock = EncryptedNoteUnlockState::Editing {
                request,
                title,
                metadata,
                tags,
                password,
                error_message: Some(error_message),
                failure_generation,
                next_generation,
            };
            return vec![NotoraEffect::Redraw];
        }
        let encryption_password =
            match textora_encryption::EncryptionPassword::new(password.expose().to_owned()) {
                Ok(password) => password,
                Err(error) => {
                    self.encrypted_note_unlock = EncryptedNoteUnlockState::Editing {
                        request,
                        title,
                        metadata,
                        tags,
                        password,
                        error_message: Some(error.to_string()),
                        failure_generation,
                        next_generation,
                    };
                    return vec![NotoraEffect::Redraw];
                }
            };
        self.encrypted_note_unlock = EncryptedNoteUnlockState::Submitting {
            request,
            title,
            metadata,
            tags,
            generation: next_generation,
        };
        vec![
            NotoraEffect::UnlockEncryptedNote(EncryptedNoteUnlockRequest {
                request,
                password: std::sync::Arc::new(encryption_password),
                generation: next_generation,
            }),
            NotoraEffect::Redraw,
        ]
    }

    fn request_title_update(&mut self, title: String) -> Vec<NotoraEffect> {
        if self.library.navigation_scope == NavigationScope::ExternalFiles
            || self.library.navigation_scope == NavigationScope::Trash
            || !matches!(self.library.selected_card, Some(DocumentIdentity::Note(_)))
        {
            return vec![NotoraEffect::Redraw];
        }
        let Some(identity) = self.library.selected_card else {
            return vec![NotoraEffect::Redraw];
        };
        self.library.pending_title_commit =
            Some(PendingTitleCommit { identity, title: normalize_notora_title(&title) });
        self.library.title_draft = None;
        self.library.last_command_error = None;
        vec![NotoraEffect::CommitTitle(title), NotoraEffect::Redraw]
    }

    fn request_title_commit(&mut self, title: String) -> Vec<NotoraEffect> {
        let effects = self.request_title_update(title);
        if matches!(effects.first(), Some(NotoraEffect::CommitTitle(_))) {
            self.layout.focus_target = FocusTarget::Editor;
        }
        effects
    }

    fn resolve_save_conflict(&mut self, resolution: ConflictResolution) -> Vec<NotoraEffect> {
        let Some(conflict) = self.library.save_conflict else {
            return vec![NotoraEffect::Redraw];
        };
        if resolution == ConflictResolution::Cancel {
            self.library.save_conflict = None;
            self.layout.overlay = OverlayState::None;
            self.layout.focus_target = FocusTarget::Editor;
            return vec![NotoraEffect::Redraw];
        }
        vec![
            NotoraEffect::ResolveSaveConflict(SaveConflictRequest {
                identity: conflict.identity,
                resolution,
            }),
            NotoraEffect::Redraw,
        ]
    }

    fn apply_note_command_completion(
        &mut self,
        result: notora_core::note_command::NoteCommandResult,
    ) -> Vec<NotoraEffect> {
        let identity = DocumentIdentity::Note(result.note.note_id);
        let created_encrypted_note = result.outcome
            == notora_core::note_command::NoteCommandOutcome::Created
            && matches!(
                result.created_access.as_ref(),
                Some(notora_core::CreatedNoteAccess::Encrypted { .. })
            );
        if result.outcome == notora_core::note_command::NoteCommandOutcome::TitleUpdated {
            if let Some(active_metadata) = self
                .library
                .active_editor_metadata
                .as_mut()
                .filter(|metadata| metadata.identity == identity)
            {
                active_metadata.metadata.title_revision =
                    active_metadata.metadata.title_revision.saturating_add(1);
            }
            self.library.last_command_error = None;
            return self.request_card_query(CardQuery::from(self.library.navigation_scope.clone()));
        }
        let request = self.select_document(identity);
        if created_encrypted_note {
            self.encrypted_note_creation = EncryptedNoteCreationState::Inactive;
            self.layout.overlay = OverlayState::None;
        }
        self.library.last_command_error = None;
        if result.outcome == notora_core::note_command::NoteCommandOutcome::Created {
            self.layout.focus_target = FocusTarget::EditorTitle;
            self.layout.compact_content = CompactContent::Editor;
        } else {
            self.layout.focus_target = FocusTarget::CardList;
        }
        let scope = self.library.navigation_scope.clone();
        let mut effects = self.request_card_query(CardQuery::from(scope));
        if !created_encrypted_note {
            effects.insert(1, NotoraEffect::PrepareDocument(request));
        }
        effects
    }

    fn request_card_query(&mut self, query: CardQuery) -> Vec<NotoraEffect> {
        self.library.card_scroll_offset_px = 0.0;
        self.library.card_page = CardPageState::LoadingInitial { query: query.clone() };
        vec![NotoraEffect::QueryCards(query), NotoraEffect::Redraw]
    }

    fn refresh_card_query(&mut self, query: CardQuery) -> Vec<NotoraEffect> {
        let (cards, next_cursor) = match &self.library.card_page {
            CardPageState::Ready { cards, next_cursor, .. }
            | CardPageState::Refreshing { cards, next_cursor, .. }
            | CardPageState::Failed { cards, next_cursor, .. } => {
                (cards.clone(), next_cursor.clone())
            }
            CardPageState::LoadingNextPage { query, cards } => {
                (cards.clone(), query.cursor.clone())
            }
            CardPageState::Idle
            | CardPageState::LoadingInitial { .. }
            | CardPageState::Empty { .. } => return self.request_card_query(query),
        };
        self.library.card_page =
            CardPageState::Refreshing { query: query.clone(), cards, next_cursor };
        vec![NotoraEffect::QueryCards(query), NotoraEffect::Redraw]
    }

    fn apply_navigation_tree(&mut self, tree: CatalogNavigationTree) -> Vec<NotoraEffect> {
        self.library.navigation_tree = NavigationTreeState {
            workspace_root_expanded: self.library.navigation_tree.workspace_root_expanded,
            tag_root_expanded: self.library.navigation_tree.tag_root_expanded,
            expanded_directories: self
                .library
                .navigation_tree
                .expanded_directories
                .iter()
                .filter(|path| tree.directories.contains(path))
                .cloned()
                .collect(),
            directories: tree.directories,
            tags: tree.tags,
        };
        let scope_is_valid = match &self.library.navigation_scope {
            NavigationScope::Directory { relative_path } => self
                .library
                .navigation_tree
                .directories
                .iter()
                .any(|directory| directory == relative_path),
            NavigationScope::Tag { tag_id } => self
                .library
                .navigation_tree
                .tags
                .iter()
                .any(|tag| tag.tag_id == *tag_id && tag.active_note_count > 0),
            _ => true,
        };
        if scope_is_valid {
            return vec![NotoraEffect::Redraw];
        }
        self.select_navigation_scope(NavigationScope::WorkspaceRoot)
    }

    fn toggle_navigation_expansion(
        &mut self,
        relative_path: std::path::PathBuf,
    ) -> Vec<NotoraEffect> {
        if !self.library.navigation_tree.directories.contains(&relative_path) {
            return vec![NotoraEffect::Redraw];
        }
        if !self.library.navigation_tree.expanded_directories.remove(&relative_path) {
            self.library.navigation_tree.expanded_directories.insert(relative_path);
        }
        vec![NotoraEffect::Redraw]
    }

    fn begin_directory_creation(
        &mut self,
        parent_relative_path: std::path::PathBuf,
    ) -> Vec<NotoraEffect> {
        let parent_is_valid = parent_relative_path.as_os_str().is_empty()
            || self.library.navigation_tree.directories.contains(&parent_relative_path);
        if self.workspace_root == WorkspaceRootState::Missing || !parent_is_valid {
            return vec![NotoraEffect::Redraw];
        }
        self.library.last_command_error = None;
        self.library.navigation_tree.workspace_root_expanded = true;
        if !parent_relative_path.as_os_str().is_empty() {
            self.library.navigation_tree.expanded_directories.insert(parent_relative_path.clone());
        }
        self.directory_creation =
            DirectoryCreationState::Editing { parent_relative_path, draft_name: String::new() };
        self.layout.focus_target = FocusTarget::NavigationTree;
        vec![NotoraEffect::Redraw]
    }

    fn update_directory_creation_text(&mut self, draft_name: String) -> Vec<NotoraEffect> {
        let DirectoryCreationState::Editing { parent_relative_path, .. } = &self.directory_creation
        else {
            return vec![NotoraEffect::Redraw];
        };
        self.directory_creation = DirectoryCreationState::Editing {
            parent_relative_path: parent_relative_path.clone(),
            draft_name,
        };
        self.library.last_command_error = None;
        vec![NotoraEffect::Redraw]
    }

    fn commit_directory_creation(&mut self) -> Vec<NotoraEffect> {
        let DirectoryCreationState::Editing { parent_relative_path, draft_name } =
            &self.directory_creation
        else {
            return vec![NotoraEffect::Redraw];
        };
        let directory_name = draft_name.trim().to_owned();
        if directory_name.is_empty() {
            self.directory_creation = DirectoryCreationState::Inactive;
            return vec![NotoraEffect::Redraw];
        }
        let parent_relative_path = parent_relative_path.clone();
        self.directory_creation = DirectoryCreationState::Submitting {
            parent_relative_path: parent_relative_path.clone(),
            directory_name: directory_name.clone(),
        };
        vec![
            NotoraEffect::ExecuteDirectoryCommand(notora_core::WorkspaceDirectoryCommand::Create {
                parent_relative_path,
                name: directory_name,
            }),
            NotoraEffect::Redraw,
        ]
    }

    fn complete_directory_creation(
        &mut self,
        relative_path: std::path::PathBuf,
    ) -> Vec<NotoraEffect> {
        let DirectoryCreationState::Submitting { parent_relative_path, .. } =
            &self.directory_creation
        else {
            return vec![NotoraEffect::Redraw];
        };
        if relative_path.parent().unwrap_or_else(|| std::path::Path::new(""))
            != parent_relative_path
        {
            return vec![NotoraEffect::Redraw];
        }
        let parent_relative_path = parent_relative_path.clone();
        self.directory_creation = DirectoryCreationState::Inactive;
        self.library.last_command_error = None;
        self.library.navigation_tree.workspace_root_expanded = true;
        if !parent_relative_path.as_os_str().is_empty() {
            self.library.navigation_tree.expanded_directories.insert(parent_relative_path);
        }
        if !self.library.navigation_tree.directories.contains(&relative_path) {
            self.library.navigation_tree.directories.push(relative_path.clone());
            self.library.navigation_tree.directories.sort();
        }
        self.select_navigation_scope(NavigationScope::Directory { relative_path })
    }

    fn fail_directory_creation(&mut self, message: String) -> Vec<NotoraEffect> {
        let DirectoryCreationState::Submitting { parent_relative_path, directory_name } =
            &self.directory_creation
        else {
            return vec![NotoraEffect::Redraw];
        };
        self.directory_creation = DirectoryCreationState::Editing {
            parent_relative_path: parent_relative_path.clone(),
            draft_name: directory_name.clone(),
        };
        self.library.last_command_error = Some(message);
        self.layout.focus_target = FocusTarget::NavigationTree;
        vec![NotoraEffect::Redraw]
    }

    fn commit_workspace_creation(&mut self) -> Vec<NotoraEffect> {
        let WorkspaceCreationState::Editing { name, parent_directory } = &self.workspace_creation
        else {
            return vec![NotoraEffect::Redraw];
        };
        let workspace_name = match notora_core::validate_workspace_directory_name(name) {
            Ok(workspace_name) => workspace_name,
            Err(error) => {
                self.library.last_command_error = Some(error.to_string());
                return vec![NotoraEffect::Redraw];
            }
        };
        let Some(parent_directory) = parent_directory.clone() else {
            self.library.last_command_error = Some("请先选择工作区保存位置".to_owned());
            return vec![NotoraEffect::Redraw];
        };
        let request =
            WorkspaceTransitionRequest::Create { root: parent_directory.join(workspace_name) };
        self.workspace_creation = WorkspaceCreationState::Inactive;
        self.workspace_transition =
            WorkspaceTransitionState::AwaitingDirtySaves { request: request.clone() };
        self.layout.overlay = OverlayState::None;
        self.layout.focus_target = FocusTarget::NavigationTree;
        self.library.last_command_error = None;
        vec![NotoraEffect::PrepareWorkspaceTransition(request), NotoraEffect::Redraw]
    }

    fn request_next_card_page(&mut self) -> Vec<NotoraEffect> {
        let CardPageState::Ready { query, cards, next_cursor: Some(cursor) } =
            &self.library.card_page
        else {
            return vec![NotoraEffect::Redraw];
        };
        let next_query = query.next_page(cursor.clone());
        self.library.card_page =
            CardPageState::LoadingNextPage { query: next_query.clone(), cards: cards.clone() };
        vec![NotoraEffect::QueryCards(next_query), NotoraEffect::Redraw]
    }

    fn apply_card_page(
        &mut self,
        query: CardQuery,
        page: notora_core::CatalogCardPage,
    ) -> Vec<NotoraEffect> {
        match &self.library.card_page {
            CardPageState::LoadingInitial { query: pending_query } if pending_query == &query => {
                self.library.card_page = if page.cards.is_empty() {
                    CardPageState::Empty { query }
                } else {
                    CardPageState::Ready { query, cards: page.cards, next_cursor: page.next_cursor }
                };
            }
            CardPageState::Refreshing { query: pending_query, .. } if pending_query == &query => {
                self.library.card_page = if page.cards.is_empty() {
                    CardPageState::Empty { query }
                } else {
                    CardPageState::Ready { query, cards: page.cards, next_cursor: page.next_cursor }
                };
            }
            CardPageState::LoadingNextPage { query: pending_query, cards }
                if pending_query == &query =>
            {
                let mut merged_cards = cards.clone();
                for card in page.cards {
                    if !merged_cards.iter().any(|known_card| known_card.note_id == card.note_id) {
                        merged_cards.push(card);
                    }
                }
                self.library.card_page = CardPageState::Ready {
                    query,
                    cards: merged_cards,
                    next_cursor: page.next_cursor,
                };
            }
            _ => return Vec::new(),
        }
        self.clear_confirmed_title_commit();
        vec![NotoraEffect::Redraw]
    }

    fn clear_confirmed_title_commit(&mut self) {
        let Some(pending_title) = self.library.pending_title_commit.as_ref() else {
            return;
        };
        let DocumentIdentity::Note(note_id) = pending_title.identity else {
            return;
        };
        let cards = match &self.library.card_page {
            CardPageState::Ready { cards, .. }
            | CardPageState::LoadingNextPage { cards, .. }
            | CardPageState::Refreshing { cards, .. }
            | CardPageState::Failed { cards, .. } => cards,
            CardPageState::Idle
            | CardPageState::LoadingInitial { .. }
            | CardPageState::Empty { .. } => return,
        };
        if cards.iter().any(|card| card.note_id == note_id && card.title == pending_title.title) {
            self.library.pending_title_commit = None;
        }
    }

    fn apply_card_query_failure(&mut self, query: CardQuery, message: String) -> Vec<NotoraEffect> {
        let (cards, next_cursor) = match &self.library.card_page {
            CardPageState::LoadingInitial { query: pending_query } if pending_query == &query => {
                (Vec::new(), None)
            }
            CardPageState::LoadingNextPage { query: pending_query, cards }
                if pending_query == &query =>
            {
                let cursor = query.cursor.clone();
                (cards.clone(), cursor)
            }
            CardPageState::Refreshing { query: pending_query, cards, next_cursor }
                if pending_query == &query =>
            {
                (cards.clone(), next_cursor.clone())
            }
            _ => return Vec::new(),
        };
        self.library.card_page = CardPageState::Failed { query, cards, next_cursor, message };
        vec![NotoraEffect::Redraw]
    }

    fn set_pane_width(&mut self, pane: Pane, logical_width: f32) {
        match pane {
            Pane::Navigation => self.layout.navigation_width_logical = logical_width,
            Pane::CardList => self.layout.card_list_width_logical = logical_width,
        }
    }

    fn select_document(&mut self, identity: DocumentIdentity) -> DocumentLoadRequest {
        let encrypted_unlock_was_active =
            !matches!(&self.encrypted_note_unlock, EncryptedNoteUnlockState::Inactive);
        self.encrypted_note_unlock = EncryptedNoteUnlockState::Inactive;
        if encrypted_unlock_was_active && self.layout.overlay == OverlayState::EncryptedNoteDialog {
            self.layout.overlay = OverlayState::None;
        }
        self.library.selected_card = Some(identity);
        self.library.title_draft = None;
        self.library.selected_document_generation =
            self.library.selected_document_generation.wrapping_add(1);
        self.library.active_editor_metadata = None;
        DocumentLoadRequest {
            identity,
            selection_generation: self.library.selected_document_generation,
        }
    }

    fn apply_active_editor_metadata(
        &mut self,
        request: DocumentLoadRequest,
        metadata: NoteEditorMetadata,
        tags: Vec<TagSummary>,
    ) -> Vec<NotoraEffect> {
        let DocumentIdentity::Note(note_id) = request.identity else {
            return vec![NotoraEffect::Redraw];
        };
        if self.library.selected_card != Some(request.identity)
            || self.library.selected_document_generation != request.selection_generation
            || metadata.note_id != note_id
        {
            return vec![NotoraEffect::Redraw];
        }
        self.library.active_editor_metadata = Some(ActiveEditorMetadata {
            identity: request.identity,
            selection_generation: request.selection_generation,
            metadata,
            tags,
        });
        vec![NotoraEffect::Redraw]
    }

    fn apply_metadata_mutation_completion(
        &mut self,
        note_id: notora_core::NoteId,
        metadata: NoteEditorMetadata,
        selection_generation: u64,
    ) {
        let Some(snapshot) = self.library.active_editor_metadata.as_mut() else {
            return;
        };
        if snapshot.identity == DocumentIdentity::Note(note_id)
            && snapshot.selection_generation == selection_generation
            && metadata.note_id == note_id
        {
            snapshot.metadata = metadata;
        }
    }

    fn apply_active_editor_saved(
        &mut self,
        identity: DocumentIdentity,
        saved_at: std::time::SystemTime,
    ) -> Vec<NotoraEffect> {
        let Some(snapshot) = self.library.active_editor_metadata.as_mut() else {
            return vec![NotoraEffect::Redraw];
        };
        if snapshot.identity == identity && self.library.selected_card == Some(identity) {
            snapshot.metadata.modified_at = saved_at;
        }
        vec![NotoraEffect::Redraw]
    }

    fn dismiss_overlay(&mut self) -> Vec<NotoraEffect> {
        if self.layout.overlay == OverlayState::None {
            return vec![NotoraEffect::Redraw];
        }
        let restore_editor_focus = self.layout.overlay == OverlayState::SaveConflict;
        match self.layout.overlay {
            OverlayState::SaveConflict => self.library.save_conflict = None,
            OverlayState::None
            | OverlayState::Settings
            | OverlayState::NewDocumentMenu
            | OverlayState::EncryptedNoteDialog
            | OverlayState::NewWorkspace
            | OverlayState::TrashPermanentDeletionConfirmation { .. }
            | OverlayState::TrashRestoreConflictConfirmation { .. } => {}
        }
        self.layout.overlay = OverlayState::None;
        self.workspace_creation = WorkspaceCreationState::Inactive;
        self.encrypted_note_creation = EncryptedNoteCreationState::Inactive;
        self.encrypted_note_unlock = EncryptedNoteUnlockState::Inactive;
        self.encrypted_conflict_copy = EncryptedConflictCopyState::Inactive;
        self.layout.focus_target =
            if restore_editor_focus { FocusTarget::Editor } else { FocusTarget::NavigationTree };
        vec![NotoraEffect::Redraw]
    }

    fn handle_escape(&mut self) -> Vec<NotoraEffect> {
        if self.layout.overlay != OverlayState::None {
            return self.dismiss_overlay();
        }
        if matches!(self.library.navigation_scope, NavigationScope::Search { .. }) {
            return self.commit_search(String::new(), None);
        }
        if !self.library.search_text.is_empty() {
            self.library.search_text.clear();
            return vec![NotoraEffect::Redraw];
        }
        self.layout.focus_target = FocusTarget::NavigationTree;
        vec![NotoraEffect::Redraw]
    }
}

#[cfg(test)]
mod metadata_actions {
    use std::time::SystemTime;

    use notora_core::{
        CatalogCard, CatalogNavigationTree, DocumentIdentity, DocumentKind, NavigationScope,
        NoteEditorMetadata, NoteEncryption, NoteId, TagId,
    };

    use super::{
        ActiveEditorMetadata, CardPageState, DirectoryCreationState, NotoraState,
        WorkspaceRootState, WorkspaceTransitionState,
    };
    use crate::action::{CardQuery, MetadataMutation, NotoraAction, NotoraEffect};

    #[test]
    fn stale_editor_metadata_cannot_cross_a_selection_generation() {
        let first_note_id = NoteId::generate();
        let second_note_id = NoteId::generate();
        let mut state = NotoraState::default();
        let first_request = crate::action::DocumentLoadRequest {
            identity: DocumentIdentity::Note(first_note_id),
            selection_generation: 1,
        };
        state.library.selected_card = Some(first_request.identity);
        state.library.selected_document_generation = first_request.selection_generation;
        let first_metadata = NoteEditorMetadata {
            note_id: first_note_id,
            created_at: SystemTime::UNIX_EPOCH,
            modified_at: SystemTime::UNIX_EPOCH,
            encryption: NoteEncryption::Unencrypted,
            title_initialization: notora_core::TitleInitialization::Independent,
            file_name_binding: notora_core::NoteFileNameBinding::TitleBound { disambiguator: 1 },
            title_revision: 0,
        };

        state.reduce(NotoraAction::ActiveEditorMetadataLoaded {
            request: first_request,
            metadata: first_metadata.clone(),
            tags: Vec::new(),
        });
        assert_eq!(
            state.library.active_editor_metadata.as_ref().map(|snapshot| snapshot.metadata.clone()),
            Some(first_metadata.clone())
        );

        state.reduce(NotoraAction::CardSelected(DocumentIdentity::Note(second_note_id)));
        state.reduce(NotoraAction::ActiveEditorMetadataLoaded {
            request: first_request,
            metadata: first_metadata,
            tags: Vec::new(),
        });
        assert_eq!(state.library.active_editor_metadata, None);
    }

    #[test]
    fn metadata_completion_refreshes_the_current_catalog_scope_without_optimistic_mutation() {
        let note_id = NoteId::generate();
        let mut state = NotoraState::default();
        let effects =
            state.reduce(NotoraAction::MetadataMutationRequested(MetadataMutation::ToggleStar {
                note_id,
            }));
        assert_eq!(
            effects,
            vec![
                NotoraEffect::ExecuteMetadataMutation(MetadataMutation::ToggleStar { note_id }),
                NotoraEffect::Redraw
            ]
        );
        assert_eq!(state.library.last_command_error, None);

        let effects = state.reduce(NotoraAction::MetadataMutationCompleted {
            note_id,
            metadata: NoteEditorMetadata {
                note_id,
                created_at: SystemTime::UNIX_EPOCH,
                modified_at: SystemTime::UNIX_EPOCH,
                encryption: NoteEncryption::Unencrypted,
                title_initialization: notora_core::TitleInitialization::Independent,
                file_name_binding: notora_core::NoteFileNameBinding::TitleBound {
                    disambiguator: 1,
                },
                title_revision: 0,
            },
            selection_generation: state.library.selected_document_generation,
        });
        assert_eq!(
            effects,
            vec![
                NotoraEffect::QueryCards(NavigationScope::WorkspaceRoot.into()),
                NotoraEffect::Redraw
            ]
        );
    }

    #[test]
    fn matching_save_completion_updates_only_the_active_editor_modified_time() {
        let note_id = NoteId::generate();
        let other_note_id = NoteId::generate();
        let mut state = NotoraState::default();
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));
        state.library.selected_document_generation = 3;
        state.library.active_editor_metadata = Some(ActiveEditorMetadata {
            identity: DocumentIdentity::Note(note_id),
            selection_generation: 3,
            metadata: NoteEditorMetadata {
                note_id,
                created_at: SystemTime::UNIX_EPOCH,
                modified_at: SystemTime::UNIX_EPOCH,
                encryption: NoteEncryption::Unencrypted,
                title_initialization: notora_core::TitleInitialization::Independent,
                file_name_binding: notora_core::NoteFileNameBinding::TitleBound {
                    disambiguator: 1,
                },
                title_revision: 0,
            },
            tags: Vec::new(),
        });
        let saved_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(120);

        state.reduce(NotoraAction::ActiveEditorSaved {
            identity: DocumentIdentity::Note(other_note_id),
            saved_at,
        });
        assert_eq!(
            state
                .library
                .active_editor_metadata
                .as_ref()
                .map(|snapshot| snapshot.metadata.modified_at),
            Some(SystemTime::UNIX_EPOCH)
        );

        state.reduce(NotoraAction::ActiveEditorSaved {
            identity: DocumentIdentity::Note(note_id),
            saved_at,
        });
        assert_eq!(
            state
                .library
                .active_editor_metadata
                .as_ref()
                .map(|snapshot| snapshot.metadata.modified_at),
            Some(saved_at)
        );
    }

    #[test]
    fn catalog_reindex_refreshes_the_current_scope_after_content_tags_change() {
        let tag_id = TagId::generate();
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::Tag { tag_id };

        let effects = state.reduce(NotoraAction::CatalogReindexed);

        assert!(matches!(
            effects.as_slice(),
            [NotoraEffect::QueryCards(query), NotoraEffect::Redraw]
                if query.scope == NavigationScope::Tag { tag_id }
        ));
    }

    #[test]
    fn catalog_reindex_keeps_middle_and_editor_panes_stable_while_refreshing() {
        let note_id = NoteId::generate();
        let query = CardQuery::from(NavigationScope::WorkspaceRoot);
        let mut state = NotoraState::default();
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));
        state.library.card_scroll_offset_px = 180.0;
        state.library.card_page = CardPageState::Ready {
            query,
            cards: vec![CatalogCard {
                note_id,
                relative_path: "项目路线图.md".into(),
                kind: DocumentKind::Markdown,
                encryption: notora_core::NoteEncryption::Unencrypted,
                title: "项目路线图".to_owned(),
                excerpt: "第三季度计划".to_owned(),
                modified_nanoseconds: 42,
                starred: false,
                tags: Vec::new(),
            }],
            next_cursor: None,
        };

        let effects = state.reduce(NotoraAction::CatalogReindexed);
        let render_model = crate::render::NotoraRenderModel::from_state(&state);

        assert!(matches!(
            effects.as_slice(),
            [NotoraEffect::QueryCards(query), NotoraEffect::Redraw]
                if query.scope == NavigationScope::WorkspaceRoot
        ));
        assert_eq!(render_model.cards.len(), 1, "refresh must retain the visible middle pane");
        assert_eq!(render_model.editor_chrome.header.title, "项目路线图");
        assert_eq!(state.library.card_scroll_offset_px, 180.0);
    }

    #[test]
    fn navigation_tree_preserves_valid_expansion_and_falls_back_after_scope_removal() {
        let mut state = NotoraState::default();
        let plans_directory = std::path::PathBuf::from("plans");
        state.library.navigation_scope =
            NavigationScope::Directory { relative_path: plans_directory.clone() };
        state.library.navigation_tree.expanded_directories.insert(plans_directory.clone());

        assert_eq!(
            state.reduce(NotoraAction::NavigationTreeLoaded(CatalogNavigationTree {
                directories: vec![plans_directory.clone(), "plans/q3".into()],
                tags: Vec::new(),
            })),
            vec![NotoraEffect::Redraw]
        );
        assert!(state.library.navigation_tree.expanded_directories.contains(&plans_directory));

        assert_eq!(
            state.reduce(NotoraAction::NavigationTreeLoaded(CatalogNavigationTree {
                directories: Vec::new(),
                tags: Vec::new(),
            })),
            vec![
                NotoraEffect::QueryCards(NavigationScope::WorkspaceRoot.into()),
                NotoraEffect::Redraw,
            ]
        );
        assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);
    }

    #[test]
    fn navigation_falls_back_when_the_selected_tag_has_no_active_notes() {
        let tag_id = TagId::generate();
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::Tag { tag_id };

        assert_eq!(
            state.reduce(NotoraAction::NavigationTreeLoaded(CatalogNavigationTree {
                directories: Vec::new(),
                tags: vec![notora_core::TagWithActiveNoteCount {
                    tag_id,
                    display_name: "Unused".to_owned(),
                    active_note_count: 0,
                }],
            })),
            vec![
                NotoraEffect::QueryCards(NavigationScope::WorkspaceRoot.into()),
                NotoraEffect::Redraw,
            ]
        );
        assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);
    }

    #[test]
    fn workspace_root_expansion_is_independent_from_directory_paths() {
        let mut state = NotoraState::default();

        assert!(state.library.navigation_tree.workspace_root_expanded);
        assert_eq!(
            state.reduce(NotoraAction::WorkspaceRootExpansionToggled),
            vec![NotoraEffect::Redraw]
        );
        assert!(!state.library.navigation_tree.workspace_root_expanded);
        assert!(state.library.navigation_tree.expanded_directories.is_empty());
    }

    #[test]
    fn tag_root_expansion_is_independent_from_workspace_navigation() {
        let mut state = NotoraState::default();

        assert!(!state.library.navigation_tree.tag_root_expanded);
        assert_eq!(state.reduce(NotoraAction::TagRootExpansionToggled), vec![NotoraEffect::Redraw]);
        assert!(state.library.navigation_tree.tag_root_expanded);
        assert!(state.library.navigation_tree.workspace_root_expanded);
    }

    #[test]
    fn directory_creation_uses_one_typed_state_machine_and_restores_failed_drafts() {
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.library.navigation_tree.directories = vec!["docs".into()];

        assert_eq!(
            state.reduce(NotoraAction::BeginDirectoryCreation {
                parent_relative_path: "docs".into(),
            }),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(
            state.directory_creation,
            DirectoryCreationState::Editing {
                parent_relative_path: "docs".into(),
                draft_name: String::new(),
            }
        );
        assert!(
            state
                .library
                .navigation_tree
                .expanded_directories
                .contains(std::path::Path::new("docs"))
        );

        state.reduce(NotoraAction::DirectoryCreationTextChanged(" plans ".to_owned()));
        assert_eq!(
            state.reduce(NotoraAction::DirectoryCreationCommitRequested),
            vec![
                NotoraEffect::ExecuteDirectoryCommand(
                    notora_core::WorkspaceDirectoryCommand::Create {
                        parent_relative_path: "docs".into(),
                        name: "plans".to_owned(),
                    },
                ),
                NotoraEffect::Redraw,
            ]
        );
        assert_eq!(
            state.directory_creation,
            DirectoryCreationState::Submitting {
                parent_relative_path: "docs".into(),
                directory_name: "plans".to_owned(),
            }
        );

        assert_eq!(
            state.reduce(NotoraAction::DirectoryCreationFailed("目标已存在".to_owned())),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(
            state.directory_creation,
            DirectoryCreationState::Editing {
                parent_relative_path: "docs".into(),
                draft_name: "plans".to_owned(),
            }
        );
        assert_eq!(state.library.last_command_error.as_deref(), Some("目标已存在"));
    }

    #[test]
    fn completed_directory_creation_expands_parent_and_selects_the_new_directory() {
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.library.navigation_tree.directories = vec!["docs".into()];
        state.reduce(NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() });
        state.reduce(NotoraAction::DirectoryCreationTextChanged("plans".to_owned()));
        state.reduce(NotoraAction::DirectoryCreationCommitRequested);

        let effects = state.reduce(NotoraAction::DirectoryCreationCompleted {
            relative_path: "docs/plans".into(),
        });

        assert_eq!(state.directory_creation, DirectoryCreationState::Inactive);
        assert!(state.library.navigation_tree.directories.contains(&"docs/plans".into()));
        assert_eq!(
            state.library.navigation_scope,
            NavigationScope::Directory { relative_path: "docs/plans".into() }
        );
        assert!(matches!(
            effects.as_slice(),
            [NotoraEffect::QueryCards(query), NotoraEffect::Redraw]
                if query.scope == NavigationScope::Directory {
                    relative_path: "docs/plans".into()
                }
        ));
    }

    #[test]
    fn workspace_transition_state_tracks_save_barrier_apply_and_failure() {
        let mut state = NotoraState::default();
        let request = crate::action::WorkspaceTransitionRequest::OpenExisting {
            root: "/workspace/two".into(),
        };

        assert_eq!(
            state.reduce(NotoraAction::WorkspaceTransitionConfirmed(request.clone())),
            vec![NotoraEffect::PrepareWorkspaceTransition(request.clone()), NotoraEffect::Redraw,]
        );
        assert_eq!(
            state.workspace_transition,
            WorkspaceTransitionState::AwaitingDirtySaves { request: request.clone() }
        );
        assert_eq!(
            state.reduce(NotoraAction::WorkspaceTransitionApplying),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.workspace_transition, WorkspaceTransitionState::Applying { request });
        assert_eq!(
            state.reduce(NotoraAction::WorkspaceTransitionFailed("保存失败".to_owned())),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.workspace_transition, WorkspaceTransitionState::Idle);
        assert_eq!(state.library.last_command_error.as_deref(), Some("保存失败"));
    }
}

#[cfg(test)]
mod trash_actions {
    use crate::action::{NotoraAction, NotoraEffect, TrashOperation};
    use crate::{FocusTarget, NotoraState, OverlayState};

    #[test]
    fn permanent_trash_operations_require_explicit_confirmation() {
        let note_id = notora_core::NoteId::generate();
        let operation = TrashOperation::PermanentlyDelete { note_id };
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::TrashOperationRequested(operation)),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(
            state.layout.overlay,
            OverlayState::TrashPermanentDeletionConfirmation { operation }
        );
        assert_eq!(state.layout.focus_target, FocusTarget::Overlay);

        assert_eq!(
            state.reduce(NotoraAction::TrashPermanentDeletionConfirmed),
            vec![NotoraEffect::ExecuteTrashOperation(operation), NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.overlay, OverlayState::None);
    }

    #[test]
    fn recoverable_trash_moves_do_not_require_the_permanent_deletion_confirmation() {
        let operation = TrashOperation::MoveToTrash { note_id: notora_core::NoteId::generate() };
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::TrashOperationRequested(operation)),
            vec![NotoraEffect::ExecuteTrashOperation(operation), NotoraEffect::Redraw]
        );
    }

    #[test]
    fn restore_conflict_offers_an_explicit_renamed_restore_or_cancel() {
        let note_id = notora_core::NoteId::generate();
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::TrashOperationFailed(
                crate::action::TrashOperationFailure::RestoreConflict { note_id },
            )),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(
            state.layout.overlay,
            OverlayState::TrashRestoreConflictConfirmation { note_id }
        );

        assert_eq!(
            state.reduce(NotoraAction::TrashRestoreWithRenamedPathConfirmed),
            vec![
                NotoraEffect::ExecuteTrashOperation(TrashOperation::RestoreWithRenamedPath {
                    note_id,
                }),
                NotoraEffect::Redraw,
            ]
        );
    }
}

fn creation_target(scope: &NavigationScope) -> Option<NoteCreationTarget> {
    match scope {
        NavigationScope::Trash | NavigationScope::ExternalFiles => None,
        NavigationScope::Directory { relative_path } => {
            Some(NoteCreationTarget { directory: Some(relative_path.clone()) })
        }
        NavigationScope::Tag { .. } => Some(NoteCreationTarget { directory: None }),
        NavigationScope::Search { .. }
        | NavigationScope::WorkspaceRoot
        | NavigationScope::Starred => Some(NoteCreationTarget { directory: None }),
    }
}

fn validate_password_confirmation(
    password: &SensitiveText,
    confirmation: &SensitiveText,
) -> Option<String> {
    if let Some(error_message) = validate_password_minimum(password) {
        return Some(error_message);
    }
    (password != confirmation).then(|| "两次输入的密码不一致".to_owned())
}

fn validate_password_minimum(password: &SensitiveText) -> Option<String> {
    let minimum_characters = textora_encryption::EncryptionPassword::MINIMUM_CHARACTERS;
    (password.expose().chars().count() < minimum_characters)
        .then(|| format!("密码至少需要 {minimum_characters} 个字符"))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, SystemTime};

    use super::{
        ActiveEditorMetadata, CardPageState, CompactContent, CompactNavigation,
        EncryptedConflictCopyState, EncryptedNoteCreationState, EncryptedNoteUnlockState,
        FocusTarget, LibraryState, NavigationPaneVisibility, NotoraState, OverlayState,
        WorkspaceRootState,
    };
    use crate::action::{CardQuery, DocumentLoadRequest, NotoraAction, NotoraEffect};
    use crate::search_controller::{SEARCH_DEBOUNCE_DELAY, SearchController};
    use notora_core::{
        CatalogCard, CatalogCardCursor, CatalogCardPage, DocumentIdentity, DocumentKind,
        NavigationScope, NoteEditorMetadata, NoteId, TagId,
    };

    fn card(note_id: NoteId, title: &str, modified_nanoseconds: i64) -> CatalogCard {
        CatalogCard {
            note_id,
            relative_path: format!("notes/{title}.md").into(),
            kind: DocumentKind::Markdown,
            encryption: notora_core::NoteEncryption::Unencrypted,
            title: title.to_owned(),
            excerpt: format!("{title} excerpt"),
            modified_nanoseconds,
            starred: false,
            tags: vec!["计划".to_owned()],
        }
    }

    fn state_with_active_workspace() -> NotoraState {
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() }
    }

    #[test]
    fn starts_in_workspace_root_scope() {
        assert_eq!(LibraryState::default().navigation_scope, NavigationScope::WorkspaceRoot);
    }

    #[test]
    fn settings_view_actions_request_only_their_owned_effects() {
        let mut state = NotoraState::default();

        assert_eq!(state.reduce(NotoraAction::SettingsViewChanged), vec![NotoraEffect::Redraw]);
        assert_eq!(
            state.reduce(NotoraAction::RetryProductSettingsPersistence),
            vec![NotoraEffect::PersistProductSettings, NotoraEffect::Redraw]
        );
    }

    #[test]
    fn title_commit_is_only_effectful_for_a_selected_workspace_note() {
        let note_id = NoteId::generate();
        let mut state = NotoraState::default();
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));

        assert_eq!(
            state.reduce(NotoraAction::TitleCommitRequested("项目路线图".to_owned())),
            vec![NotoraEffect::CommitTitle("项目路线图".to_owned()), NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.focus_target, FocusTarget::Editor);
        assert_eq!(
            state
                .library
                .pending_title_commit
                .as_ref()
                .map(|pending_title| (pending_title.identity, pending_title.title.as_str())),
            Some((DocumentIdentity::Note(note_id), "项目路线图"))
        );

        state.library.navigation_scope = NavigationScope::Trash;
        assert_eq!(
            state.reduce(NotoraAction::TitleCommitRequested("不能提交".to_owned())),
            vec![NotoraEffect::Redraw]
        );

        state.library.navigation_scope = NavigationScope::ExternalFiles;
        state.library.selected_card =
            Some(DocumentIdentity::ExternalFile(notora_core::ExternalFileId::generate()));
        assert_eq!(
            state.reduce(NotoraAction::TitleCommitRequested("外部文件".to_owned())),
            vec![NotoraEffect::Redraw]
        );
    }

    #[test]
    fn external_file_record_is_removed_only_after_runtime_close_completes() {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::ExternalFiles;
        let identity = state.external_files.create_untitled(DocumentKind::Markdown);
        let DocumentIdentity::ExternalFile(external_file_id) = identity else {
            panic!("an external session must have an external identity");
        };
        state.library.selected_card = Some(identity);

        assert_eq!(
            state.reduce(NotoraAction::ExternalFileCloseRequested(external_file_id)),
            vec![NotoraEffect::CloseExternalFile(external_file_id), NotoraEffect::Redraw]
        );
        assert!(state.external_files.session(external_file_id).is_some());

        assert_eq!(
            state.reduce(NotoraAction::ExternalFileCloseCompleted(external_file_id)),
            vec![NotoraEffect::Redraw]
        );
        assert!(state.external_files.session(external_file_id).is_none());
        assert_eq!(state.library.selected_card, None);
    }

    #[test]
    fn failed_external_file_close_keeps_the_record_and_reports_the_reason() {
        let mut state = NotoraState::default();
        let identity = state.external_files.create_untitled(DocumentKind::Text);
        let DocumentIdentity::ExternalFile(external_file_id) = identity else {
            panic!("an external session must have an external identity");
        };

        let effects =
            state.reduce(NotoraAction::ExternalFileCloseFailed("文件仍有未保存修改".to_owned()));

        assert_eq!(effects, vec![NotoraEffect::Redraw]);
        assert!(state.external_files.session(external_file_id).is_some());
        assert_eq!(state.library.last_command_error.as_deref(), Some("文件仍有未保存修改"));
    }

    #[test]
    fn clearing_external_files_removes_completed_records_and_keeps_blocked_records() {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::ExternalFiles;
        let closed_identity = state.external_files.create_untitled(DocumentKind::Markdown);
        let blocked_identity = state.external_files.create_untitled(DocumentKind::Text);
        let DocumentIdentity::ExternalFile(closed_external_file_id) = closed_identity else {
            panic!("external fixture must have an external identity");
        };
        let DocumentIdentity::ExternalFile(blocked_external_file_id) = blocked_identity else {
            panic!("external fixture must have an external identity");
        };
        state.library.selected_card = Some(closed_identity);

        assert_eq!(
            state.reduce(NotoraAction::ExternalFilesClearRequested),
            vec![NotoraEffect::CloseAllExternalFiles, NotoraEffect::Redraw]
        );
        assert_eq!(
            state.reduce(NotoraAction::ExternalFilesClearCompleted {
                closed_external_file_ids: vec![closed_external_file_id],
                blocked_count: 1,
            }),
            vec![NotoraEffect::Redraw]
        );

        assert!(state.external_files.session(closed_external_file_id).is_none());
        assert!(state.external_files.session(blocked_external_file_id).is_some());
        assert_eq!(state.library.selected_card, None);
        assert_eq!(
            state.library.last_command_error.as_deref(),
            Some("有 1 个文件未清空：请先保存未保存内容或取消固定")
        );
    }

    #[test]
    fn refreshed_catalog_title_confirms_the_pending_title_commit() {
        let note_id = NoteId::generate();
        let query = CardQuery::from(NavigationScope::WorkspaceRoot);
        let mut state = NotoraState::default();
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));

        let _ = state.reduce(NotoraAction::TitleCommitRequested("  项目路线图  ".to_owned()));
        state.library.card_page = CardPageState::LoadingInitial { query: query.clone() };
        let _ = state.reduce(NotoraAction::CardQueryCompleted {
            query,
            page: CatalogCardPage {
                cards: vec![card(note_id, "项目路线图", 1)],
                next_cursor: None,
            },
        });

        assert_eq!(state.library.pending_title_commit, None);
    }

    #[test]
    fn title_text_changes_remain_a_local_draft_until_commit() {
        let note_id = NoteId::generate();
        let mut state = NotoraState::default();
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));
        state.layout.focus_target = FocusTarget::EditorTitle;

        assert_eq!(
            state.reduce(NotoraAction::TitleTextChanged("项目路线图".to_owned())),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.focus_target, FocusTarget::EditorTitle);
    }

    #[test]
    fn source_view_toggle_is_a_typed_editor_effect() {
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::ToggleSourceViewRequested),
            vec![NotoraEffect::ToggleEditorView]
        );
        assert_eq!(state.layout.focus_target, FocusTarget::Editor);
    }

    #[test]
    fn empty_search_restores_the_scope_before_search() {
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Starred));
        let _ = state.reduce(NotoraAction::SearchCommitted {
            query: "roadmap".to_owned(),
            search_generation: None,
        });

        assert_eq!(
            state.reduce(NotoraAction::SearchCommitted {
                query: String::new(),
                search_generation: None,
            }),
            vec![
                NotoraEffect::QueryCards(CardQuery::from(NavigationScope::Starred)),
                NotoraEffect::Redraw,
            ]
        );
        assert_eq!(state.library.navigation_scope, NavigationScope::Starred);
    }

    #[test]
    fn search_text_updates_immediately_without_querying_until_the_debounce_commits() {
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::SearchTextChanged("roadmap".to_owned())),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.library.search_text, "roadmap");
        assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);
    }

    #[test]
    fn non_empty_search_keeps_keyboard_focus_in_the_search_box() {
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::FocusRequested(FocusTarget::NavigationSearch));

        let _ = state.reduce(NotoraAction::SearchCommitted {
            query: "roadmap".to_owned(),
            search_generation: None,
        });

        assert_eq!(state.layout.focus_target, FocusTarget::NavigationSearch);
    }

    #[test]
    fn trash_scope_cannot_request_note_creation() {
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Trash));

        assert_eq!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
            vec![NotoraEffect::Redraw]
        );
    }

    #[test]
    fn external_files_scope_does_not_query_the_workspace_catalog() {
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::NavigationSelected(NavigationScope::ExternalFiles)),
            vec![NotoraEffect::Redraw]
        );
    }

    #[test]
    fn changing_navigation_scope_invalidates_the_previous_card_selection() {
        let mut state = NotoraState::default();
        let note_id = NoteId::generate();
        let request = state.select_document(DocumentIdentity::Note(note_id));
        state.library.active_editor_metadata = Some(ActiveEditorMetadata {
            identity: request.identity,
            selection_generation: request.selection_generation,
            metadata: NoteEditorMetadata {
                note_id,
                created_at: SystemTime::UNIX_EPOCH,
                modified_at: SystemTime::UNIX_EPOCH,
                encryption: notora_core::NoteEncryption::Unencrypted,
                title_initialization: notora_core::TitleInitialization::Independent,
                file_name_binding: notora_core::NoteFileNameBinding::TitleBound {
                    disambiguator: 1,
                },
                title_revision: 0,
            },
            tags: Vec::new(),
        });

        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Trash));

        assert_eq!(state.library.selected_card, None);
        assert_eq!(state.library.active_editor_metadata, None);
        assert_eq!(state.library.selected_document_generation, 2);
    }

    #[test]
    fn reselecting_the_current_navigation_scope_preserves_the_active_selection() {
        let mut state = NotoraState::default();
        let identity = DocumentIdentity::Note(NoteId::generate());
        let _ = state.select_document(identity);

        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot));

        assert_eq!(state.library.selected_card, Some(identity));
        assert_eq!(state.library.selected_document_generation, 1);
    }

    #[test]
    fn tag_scope_requires_content_hashtags_instead_of_implicit_attachment() {
        let mut state = state_with_active_workspace();
        let tag_id = TagId::generate();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id }));

        assert_eq!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
            vec![
                NotoraEffect::RequestNoteCreation {
                    kind: DocumentKind::Markdown,
                    target: crate::action::NoteCreationTarget { directory: None },
                },
                NotoraEffect::Redraw,
            ]
        );
    }

    #[test]
    fn note_requests_reduce_to_a_typed_domain_command_effect() {
        let mut state = state_with_active_workspace();

        assert!(matches!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)).as_slice(),
            [
                NotoraEffect::RequestNoteCreation {
                    kind: DocumentKind::Markdown,
                    target: crate::action::NoteCreationTarget { directory: None },
                },
                NotoraEffect::Redraw
            ]
        ));
    }

    #[test]
    fn created_note_is_selected_and_enters_editor_state() {
        let mut state = NotoraState::default();
        let note_id = NoteId::generate();
        let result = notora_core::note_command::NoteCommandResult {
            note: notora_core::CatalogNote {
                note_id,
                relative_path: "无标题.md".into(),
                kind: DocumentKind::Markdown,
                title: "无标题".to_owned(),
                excerpt: String::new(),
                modified_at: SystemTime::UNIX_EPOCH,
                file_size: 0,
                content_hash: Vec::new(),
                starred: false,
            },
            previous_relative_path: None,
            outcome: notora_core::note_command::NoteCommandOutcome::Created,
            created_access: Some(notora_core::CreatedNoteAccess::Unencrypted),
        };

        let effects = state.reduce(NotoraAction::NoteCommandCompleted(result));

        assert_eq!(state.library.selected_card, Some(DocumentIdentity::Note(note_id)));
        assert_eq!(state.layout.focus_target, FocusTarget::EditorTitle);
        assert_eq!(state.layout.compact_content, CompactContent::Editor);
        assert!(matches!(effects.get(1), Some(NotoraEffect::PrepareDocument(_))));
    }

    #[test]
    fn move_dialog_entry_stays_behind_a_typed_effect() {
        let mut state = NotoraState::default();
        let note_id = NoteId::generate();

        assert_eq!(
            state.reduce(NotoraAction::MoveDialogRequested(note_id)),
            vec![NotoraEffect::ChooseNoteMoveDirectory(note_id), NotoraEffect::Redraw]
        );
    }

    #[test]
    fn command_failure_preserves_the_existing_selection_and_exposes_a_recoverable_message() {
        let mut state = NotoraState::default();
        let selected_note = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        let _ = state.reduce(NotoraAction::CardSelected(selected_note));

        assert_eq!(
            state.reduce(NotoraAction::NoteCommandFailed("destination exists".to_owned())),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.library.selected_card, Some(selected_note));
        assert_eq!(state.library.last_command_error.as_deref(), Some("destination exists"));
    }

    #[test]
    fn preview_promotion_is_an_explicit_runtime_effect() {
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::PromotePreviewRequested),
            vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
        );
    }

    #[test]
    fn opening_new_document_menu_only_opens_the_menu() {
        let mut state = state_with_active_workspace();

        assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
        assert_eq!(state.layout.overlay, OverlayState::NewDocumentMenu);
        assert_eq!(state.layout.focus_target, FocusTarget::Overlay);
    }

    #[test]
    fn encrypted_note_creation_validates_and_clears_sensitive_draft() {
        use ui::core::widget::SensitiveText;

        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);
        let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);

        assert_eq!(state.layout.overlay, OverlayState::EncryptedNoteDialog);
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "valid-password".to_owned(),
        )));
        let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "different-password".to_owned(),
        )));
        assert_eq!(
            state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
            vec![NotoraEffect::Redraw]
        );
        assert!(matches!(
            &state.encrypted_note_creation,
            EncryptedNoteCreationState::Editing { error_message: Some(message), .. }
                if message == "两次输入的密码不一致"
        ));

        let _ = state.reduce(NotoraAction::OverlayDismissed);

        assert_eq!(state.encrypted_note_creation, EncryptedNoteCreationState::Inactive);
        assert_eq!(state.layout.overlay, OverlayState::None);
    }

    #[test]
    fn encrypted_note_creation_rejects_five_character_password() {
        use ui::core::widget::SensitiveText;

        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "abc12".to_owned(),
        )));
        let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "abc12".to_owned(),
        )));

        assert_eq!(
            state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
            vec![NotoraEffect::Redraw]
        );
        assert!(matches!(
            state.encrypted_note_creation,
            EncryptedNoteCreationState::Editing { error_message: Some(message), .. }
                if message == "密码至少需要 6 个字符"
        ));
    }

    #[test]
    fn encrypted_note_creation_prevents_duplicate_submission() {
        use ui::core::widget::SensitiveText;

        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "abc123".to_owned(),
        )));
        let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "abc123".to_owned(),
        )));

        let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);
        assert!(matches!(
            effects.as_slice(),
            [NotoraEffect::ExecuteNoteCommand(
                notora_core::note_command::NoteCommand::CreateConfigured(request)
            ), NotoraEffect::Redraw]
                if request.kind == DocumentKind::Markdown
                    && matches!(request.storage, notora_core::CreateNoteStorage::Encrypted { .. })
        ));
        assert!(matches!(
            state.encrypted_note_creation,
            EncryptedNoteCreationState::Submitting { generation: 1, .. }
        ));
        assert_eq!(
            state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
            vec![NotoraEffect::Redraw]
        );
    }

    #[test]
    fn encrypted_note_creation_effect_debug_redacts_password() {
        use ui::core::widget::SensitiveText;

        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "debug-secret-password".to_owned(),
        )));
        let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "debug-secret-password".to_owned(),
        )));

        let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);
        let rendered = format!("{effects:?}");

        assert!(!rendered.contains("debug-secret-password"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn encrypted_conflict_copy_requires_confirmation_and_redacts_its_effect() {
        use ui::core::widget::SensitiveText;

        let identity = DocumentIdentity::Note(NoteId::generate());
        let target_path = std::path::PathBuf::from("冲突副本.md");
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 9 });
        let _ = state.reduce(NotoraAction::EncryptedConflictCopyRequired {
            identity,
            target_path: target_path.clone(),
        });
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "abc123".to_owned(),
        )));
        let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "abc123".to_owned(),
        )));

        let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);

        assert!(matches!(
            effects.as_slice(),
            [NotoraEffect::SaveEncryptedConflictCopy(request), NotoraEffect::Redraw]
                if request.identity == identity
                    && request.target_path == target_path
                    && request.generation == 1
        ));
        assert!(!format!("{effects:?}").contains("abc123"));
        assert!(matches!(
            state.encrypted_conflict_copy,
            EncryptedConflictCopyState::Submitting { generation: 1, .. }
        ));
    }

    #[test]
    fn encrypted_conflict_copy_rejects_five_character_password() {
        use ui::core::widget::SensitiveText;

        let identity = DocumentIdentity::Note(NoteId::generate());
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 9 });
        let _ = state.reduce(NotoraAction::EncryptedConflictCopyRequired {
            identity,
            target_path: "冲突副本.md".into(),
        });
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "abc12".to_owned(),
        )));
        let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "abc12".to_owned(),
        )));

        assert_eq!(
            state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
            vec![NotoraEffect::Redraw]
        );
        assert!(matches!(
            state.encrypted_conflict_copy,
            EncryptedConflictCopyState::Editing { error_message: Some(message), .. }
                if message == "密码至少需要 6 个字符"
        ));
    }

    #[test]
    fn encrypted_note_unlock_closes_cleanly_on_cancel_or_document_selection() {
        use ui::core::widget::SensitiveText;

        let note_id = NoteId::generate();
        let request = DocumentLoadRequest {
            identity: DocumentIdentity::Note(note_id),
            selection_generation: 4,
        };
        let mut state = state_with_active_workspace();
        state.library.selected_card = Some(request.identity);
        state.library.selected_document_generation = request.selection_generation;

        let _ = state.reduce(NotoraAction::EncryptedNoteUnlockRequired {
            request,
            title: "秘密笔记".to_owned(),
            metadata: NoteEditorMetadata {
                note_id,
                created_at: SystemTime::UNIX_EPOCH,
                modified_at: SystemTime::UNIX_EPOCH,
                encryption: notora_core::NoteEncryption::Encrypted,
                title_initialization: notora_core::TitleInitialization::Independent,
                file_name_binding: notora_core::NoteFileNameBinding::TitleBound {
                    disambiguator: 1,
                },
                title_revision: 0,
            },
            tags: Vec::new(),
        });
        assert_eq!(state.layout.overlay, OverlayState::EncryptedNoteDialog);
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "abc123".to_owned(),
        )));
        let mut switched_state = state.clone();
        let other_identity = DocumentIdentity::Note(NoteId::generate());
        let _ = switched_state.reduce(NotoraAction::CardSelected(other_identity));
        assert_eq!(switched_state.encrypted_note_unlock, EncryptedNoteUnlockState::Inactive);
        assert_eq!(switched_state.layout.overlay, OverlayState::None);

        let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);
        assert!(matches!(
            effects.as_slice(),
            [NotoraEffect::UnlockEncryptedNote(unlock_request), NotoraEffect::Redraw]
                if unlock_request.request == request && unlock_request.generation == 1
        ));
        assert!(matches!(
            state.encrypted_note_unlock,
            EncryptedNoteUnlockState::Submitting {
                request: pending_request,
                generation: 1,
                ..
            } if pending_request == request
        ));

        let _ = state.reduce(NotoraAction::OverlayDismissed);

        assert_eq!(state.encrypted_note_unlock, EncryptedNoteUnlockState::Inactive);
    }

    #[test]
    fn encrypted_note_unlock_rejects_five_character_password() {
        use ui::core::widget::SensitiveText;

        let note_id = NoteId::generate();
        let request = DocumentLoadRequest {
            identity: DocumentIdentity::Note(note_id),
            selection_generation: 4,
        };
        let mut state = state_with_active_workspace();
        state.library.selected_card = Some(request.identity);
        state.library.selected_document_generation = request.selection_generation;
        let _ = state.reduce(NotoraAction::EncryptedNoteUnlockRequired {
            request,
            title: "秘密笔记".to_owned(),
            metadata: NoteEditorMetadata {
                note_id,
                created_at: SystemTime::UNIX_EPOCH,
                modified_at: SystemTime::UNIX_EPOCH,
                encryption: notora_core::NoteEncryption::Encrypted,
                title_initialization: notora_core::TitleInitialization::Independent,
                file_name_binding: notora_core::NoteFileNameBinding::TitleBound {
                    disambiguator: 1,
                },
                title_revision: 0,
            },
            tags: Vec::new(),
        });
        let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "abc12".to_owned(),
        )));

        assert_eq!(
            state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
            vec![NotoraEffect::Redraw]
        );
        assert!(matches!(
            state.encrypted_note_unlock,
            EncryptedNoteUnlockState::Editing { error_message: Some(message), .. }
                if message == "密码至少需要 6 个字符"
        ));
    }

    #[test]
    fn focus_requests_cannot_escape_an_open_product_overlay() {
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);

        let _ = state.reduce(NotoraAction::FocusRequested(FocusTarget::Editor));

        assert_eq!(state.layout.overlay, OverlayState::NewDocumentMenu);
        assert_eq!(state.layout.focus_target, FocusTarget::Overlay);
    }

    #[test]
    fn missing_workspace_root_cannot_open_the_new_document_menu() {
        let mut state = NotoraState::default();

        assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
        assert_eq!(state.layout.overlay, OverlayState::None);
    }

    #[test]
    fn workspace_root_selection_is_separate_from_note_creation() {
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::WorkspaceRootSelectionRequested),
            vec![NotoraEffect::ChooseWorkspaceRoot, NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.overlay, OverlayState::None);
    }

    #[test]
    fn escape_closes_the_new_document_menu() {
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);

        assert_eq!(state.reduce(NotoraAction::EscapePressed), vec![NotoraEffect::Redraw]);
        assert_eq!(state.layout.overlay, OverlayState::None);
        assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
    }

    #[test]
    fn trash_scope_cannot_open_the_new_document_menu() {
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Trash));

        assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
        assert_eq!(state.layout.overlay, OverlayState::None);
        assert_ne!(state.layout.focus_target, FocusTarget::Overlay);
    }

    #[test]
    fn activating_the_selected_card_promotes_preview_and_focuses_the_editor() {
        let mut state = NotoraState::default();
        let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        let _ = state.reduce(NotoraAction::CardSelected(identity));

        assert_eq!(
            state.reduce(NotoraAction::CardActivated(identity)),
            vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.focus_target, FocusTarget::Editor);
    }

    #[test]
    fn repeated_selection_of_the_same_document_uses_distinct_load_generations() {
        let mut state = NotoraState::default();
        let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());

        let first_effects = state.reduce(NotoraAction::CardSelected(identity));
        let second_effects = state.reduce(NotoraAction::CardSelected(identity));

        assert!(matches!(
            first_effects.as_slice(),
            [
                NotoraEffect::PrepareDocument(crate::action::DocumentLoadRequest {
                    identity: first_identity,
                    selection_generation: 1,
                }),
                NotoraEffect::Redraw,
            ] if *first_identity == identity
        ));
        assert!(matches!(
            second_effects.as_slice(),
            [
                NotoraEffect::PrepareDocument(crate::action::DocumentLoadRequest {
                    identity: second_identity,
                    selection_generation: 2,
                }),
                NotoraEffect::Redraw,
            ] if *second_identity == identity
        ));
        assert_eq!(state.library.selected_document_generation, 2);
    }

    #[test]
    fn stale_a_b_a_search_completion_cannot_replace_the_latest_generation() {
        let mut controller = SearchController::default();
        controller.set_active_workspace(notora_core::WorkspaceId::generate(), 1);
        let start = Instant::now();
        controller.schedule_committed_query("same".to_owned(), start);
        let first_request = controller
            .take_due_request(start + SEARCH_DEBOUNCE_DELAY)
            .expect("first search should become due");
        controller.schedule_committed_query("other".to_owned(), start);
        let _ = controller
            .take_due_request(start + SEARCH_DEBOUNCE_DELAY)
            .expect("intermediate search should become due");
        controller.schedule_committed_query("same".to_owned(), start + Duration::from_millis(1));
        let latest_request = controller
            .take_due_request(start + SEARCH_DEBOUNCE_DELAY + Duration::from_millis(1))
            .expect("latest repeated search should become due");
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::SearchCommitted {
            query: latest_request.query,
            search_generation: Some(latest_request.search_generation),
        });
        let stale_query = CardQuery {
            scope: NavigationScope::Search { query: first_request.query },
            cursor: None,
            page_size: crate::action::DEFAULT_CARD_PAGE_SIZE,
            search_generation: Some(first_request.search_generation),
        };

        assert!(
            state
                .reduce(NotoraAction::CardQueryCompleted {
                    query: stale_query,
                    page: CatalogCardPage { cards: Vec::new(), next_cursor: None },
                })
                .is_empty()
        );
        assert!(matches!(
            state.library.card_page,
            CardPageState::LoadingInitial { query }
                if query.search_generation == Some(latest_request.search_generation)
        ));
    }

    #[test]
    fn escape_closes_overlay_then_clears_search_then_focuses_navigation() {
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::OpenSettings);
        assert_eq!(state.layout.overlay, OverlayState::Settings);
        let _ = state.reduce(NotoraAction::EscapePressed);
        assert_eq!(state.layout.overlay, OverlayState::None);

        let _ = state.reduce(NotoraAction::SearchCommitted {
            query: "idea".to_owned(),
            search_generation: None,
        });
        let _ = state.reduce(NotoraAction::EscapePressed);
        assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);

        state.layout.focus_target = FocusTarget::Editor;
        let _ = state.reduce(NotoraAction::EscapePressed);
        assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
    }

    #[test]
    fn compact_navigation_and_back_use_explicit_mutually_exclusive_layout_state() {
        let mut state = NotoraState::default();
        let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        state.library.selected_card = Some(identity);

        let _ = state.reduce(NotoraAction::CompactNavigationRequested);
        assert_eq!(state.layout.compact_navigation, CompactNavigation::Visible);

        let _ = state.reduce(NotoraAction::CardActivated(identity));
        assert_eq!(state.layout.compact_content, CompactContent::Editor);

        let _ = state.reduce(NotoraAction::CompactBackRequested);
        assert_eq!(state.layout.compact_content, CompactContent::CardList);
        assert_eq!(state.layout.compact_navigation, CompactNavigation::Hidden);
    }

    #[test]
    fn navigation_pane_toggle_preserves_width_and_moves_focus_to_visible_content() {
        let mut state = NotoraState::default();
        state.layout.navigation_width_logical = 268.0;
        state.layout.focus_target = FocusTarget::NavigationSearch;

        let collapse_effects = state.reduce(NotoraAction::NavigationPaneVisibilityToggled);

        assert_eq!(state.layout.navigation_pane_visibility, NavigationPaneVisibility::Collapsed);
        assert_eq!(state.layout.navigation_width_logical, 268.0);
        assert_eq!(state.layout.focus_target, FocusTarget::CardList);
        assert_eq!(collapse_effects, vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]);

        let _ = state.reduce(NotoraAction::NavigationPaneVisibilityToggled);

        assert_eq!(state.layout.navigation_pane_visibility, NavigationPaneVisibility::Expanded);
        assert_eq!(state.layout.navigation_width_logical, 268.0);
        assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
    }

    #[test]
    fn concurrent_save_requires_an_explicit_typed_resolution() {
        let mut state = NotoraState::default();
        let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 7 });

        assert_eq!(state.layout.overlay, OverlayState::SaveConflict);
        assert_eq!(state.layout.focus_target, FocusTarget::Overlay);

        assert_eq!(
            state.reduce(NotoraAction::SaveConflictResolutionRequested(
                crate::action::ConflictResolution::RetrySave,
            )),
            vec![
                NotoraEffect::ResolveSaveConflict(crate::action::SaveConflictRequest {
                    identity,
                    resolution: crate::action::ConflictResolution::RetrySave,
                }),
                NotoraEffect::Redraw,
            ]
        );
    }

    #[test]
    fn files_scope_creates_an_untitled_external_document() {
        let mut state = state_with_active_workspace();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::ExternalFiles));

        assert_eq!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
            vec![
                NotoraEffect::CreateUntitledExternal(DocumentKind::Markdown),
                NotoraEffect::Redraw,
            ]
        );
    }

    #[test]
    fn card_query_next_page_merges_cards_and_preserves_selection() {
        let mut state = NotoraState::default();
        let first_note_id = NoteId::generate();
        let second_note_id = NoteId::generate();
        let initial_query = CardQuery::from(NavigationScope::WorkspaceRoot);
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot));
        let _ = state.reduce(NotoraAction::CardQueryCompleted {
            query: initial_query.clone(),
            page: CatalogCardPage {
                cards: vec![card(first_note_id, "first", 20)],
                next_cursor: Some(CatalogCardCursor {
                    modified_nanoseconds: 20,
                    relative_path: "notes/first.md".into(),
                    note_id: first_note_id,
                }),
            },
        });
        let _ = state.reduce(NotoraAction::CardSelected(DocumentIdentity::Note(first_note_id)));

        let effects =
            state.reduce(NotoraAction::CardListScrolled { offset_px: 240.0, near_end: true });
        let next_query = initial_query.next_page(CatalogCardCursor {
            modified_nanoseconds: 20,
            relative_path: "notes/first.md".into(),
            note_id: first_note_id,
        });
        assert_eq!(
            effects,
            vec![NotoraEffect::QueryCards(next_query.clone()), NotoraEffect::Redraw]
        );
        assert!(matches!(
            &state.library.card_page,
            CardPageState::LoadingNextPage { query, cards }
                if query == &next_query && cards.len() == 1
        ));

        let _ = state.reduce(NotoraAction::CardQueryCompleted {
            query: next_query,
            page: CatalogCardPage {
                cards: vec![card(second_note_id, "second", 10)],
                next_cursor: None,
            },
        });
        assert_eq!(state.library.selected_card, Some(DocumentIdentity::Note(first_note_id)));
        assert!(matches!(
            &state.library.card_page,
            CardPageState::Ready { cards, next_cursor: None, .. }
                if cards.iter().map(|card| card.note_id).collect::<Vec<_>>()
                    == vec![first_note_id, second_note_id]
        ));
    }

    #[test]
    fn card_query_empty_result_uses_an_explicit_empty_state() {
        let mut state = NotoraState::default();
        let query = CardQuery::from(NavigationScope::Starred);
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Starred));

        let _ = state.reduce(NotoraAction::CardQueryCompleted {
            query: query.clone(),
            page: CatalogCardPage { cards: Vec::new(), next_cursor: None },
        });

        assert_eq!(state.library.card_page, CardPageState::Empty { query });
    }

    #[test]
    fn new_workspace_form_builds_a_create_transition_from_name_and_location() {
        let mut state = NotoraState::default();

        assert_eq!(
            state.reduce(NotoraAction::OpenWorkspaceCreationRequested),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.overlay, OverlayState::NewWorkspace);
        let _ = state.reduce(NotoraAction::WorkspaceCreationNameChanged("我的工作区".to_owned()));
        let _ = state.reduce(NotoraAction::WorkspaceCreationLocationSelected("/tmp".into()));

        let effects = state.reduce(NotoraAction::WorkspaceCreationCommitRequested);

        assert_eq!(
            effects,
            vec![
                NotoraEffect::PrepareWorkspaceTransition(
                    crate::action::WorkspaceTransitionRequest::Create {
                        root: "/tmp/我的工作区".into(),
                    },
                ),
                NotoraEffect::Redraw,
            ]
        );
        assert_eq!(state.layout.overlay, OverlayState::None);
    }

    #[test]
    fn invalid_workspace_name_keeps_the_form_open_without_starting_a_transition() {
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::OpenWorkspaceCreationRequested);
        let _ = state.reduce(NotoraAction::WorkspaceCreationNameChanged("a/b".to_owned()));
        let _ = state.reduce(NotoraAction::WorkspaceCreationLocationSelected("/tmp".into()));

        assert_eq!(
            state.reduce(NotoraAction::WorkspaceCreationCommitRequested),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.overlay, OverlayState::NewWorkspace);
        assert!(state.library.last_command_error.is_some());
    }
}
