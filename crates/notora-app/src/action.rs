//! notora 产品动作与 reducer effect。

use std::path::PathBuf;

use notora_core::note_command::{MoveNoteRequest, NoteCommand, NoteCommandResult};
use notora_core::{
    DocumentIdentity, DocumentKind, NavigationScope, NoteEditorMetadata, NoteId, TagId, TagSummary,
};

use crate::search_controller::SearchGeneration;
use crate::settings_overlay::ProductSettingsUpdate;
use crate::state::{FocusTarget, Pane};

/// 中栏查询的纯输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardQuery {
    pub scope: NavigationScope,
    pub cursor: Option<CardPageCursor>,
    pub page_size: usize,
    pub search_generation: Option<SearchGeneration>,
}

/// 由稳定排序键组成的下一页游标；绝不依赖会因实时更新漂移的裸 offset。
pub use notora_core::CatalogCardCursor as CardPageCursor;

pub const DEFAULT_CARD_PAGE_SIZE: usize = 50;

impl CardQuery {
    pub fn next_page(&self, cursor: CardPageCursor) -> Self {
        Self {
            scope: self.scope.clone(),
            cursor: Some(cursor),
            page_size: self.page_size,
            search_generation: self.search_generation,
        }
    }
}

/// 新建笔记的有效目标。回收站不是有效目标。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteCreationTarget {
    pub directory: Option<PathBuf>,
}

/// 一次卡片选择触发的后台加载请求。
///
/// `selection_generation` 使 A→B→A 的两次同 identity 选择保持可区分，旧读取结果
/// 不会覆盖第二次选择。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentLoadRequest {
    pub identity: DocumentIdentity,
    pub selection_generation: u64,
}

/// 外部修改与本地 dirty 内容冲突后的显式用户决策。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictResolution {
    ReloadFromDisk,
    SaveCopy,
    RetrySave,
    Cancel,
}

/// 不依赖 runtime tab 的冲突处理请求；产品边界负责从 identity 解析活动 tab。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveConflictRequest {
    pub identity: DocumentIdentity,
    pub resolution: ConflictResolution,
}

/// 仅描述用户意图的 catalog metadata 变更；SQL 只允许由后台 effect 执行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataMutation {
    ToggleStar { note_id: NoteId },
    AttachTagByName { note_id: NoteId, display_name: String },
    DetachTag { note_id: NoteId, tag_id: TagId },
    SetTitle { note_id: NoteId, title: String },
    CompleteTitleInitializationFromHeader { note_id: NoteId, title: String },
    CompleteTitleInitializationFromDocument { note_id: NoteId, title: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataMutationOutcome {
    Applied,
    TitleInitializationWon,
    TitleInitializationLost,
}

/// 只针对工作区 NoteId 的回收站操作；外部文件没有可表达的变体。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrashOperation {
    MoveToTrash { note_id: NoteId },
    Restore { note_id: NoteId },
    RestoreWithRenamedPath { note_id: NoteId },
    PermanentlyDelete { note_id: NoteId },
    Empty,
}

/// 回收站后台操作失败的可恢复分类；仅在用户必须作出下一步决定时保留结构化信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashOperationFailure {
    RestoreConflict { note_id: NoteId },
    Message(String),
}

/// 产品层的类型化用户动作。
#[derive(Clone, Debug, PartialEq)]
pub enum NotoraAction {
    NavigationSelected(NavigationScope),
    SearchTextChanged(String),
    SearchCommitted {
        query: String,
        search_generation: Option<SearchGeneration>,
    },
    CardQueryCompleted {
        query: CardQuery,
        page: notora_core::CatalogCardPage,
    },
    CardQueryFailed {
        query: CardQuery,
        message: String,
    },
    NavigationTreeLoaded(notora_core::CatalogNavigationTree),
    NavigationTreeFailed(String),
    CatalogReindexed,
    CatalogRecoveryNotified(String),
    NavigationExpansionToggled(PathBuf),
    CardListScrolled {
        offset_px: f32,
        near_end: bool,
    },
    CardSelected(DocumentIdentity),
    CardActivated(DocumentIdentity),
    ActiveEditorMetadataLoaded {
        request: DocumentLoadRequest,
        metadata: NoteEditorMetadata,
        tags: Vec<TagSummary>,
    },
    ActiveEditorSaved {
        identity: DocumentIdentity,
        saved_at: std::time::SystemTime,
    },
    CompactNavigationRequested,
    CompactBackRequested,
    OpenExternalFileDialogRequested,
    ExternalPathsReceived(Vec<PathBuf>),
    ExternalFileOpened(DocumentIdentity),
    PromotePreviewRequested,
    WorkspaceRootSelectionRequested,
    OpenNewDocumentMenu,
    CreateRequested(DocumentKind),
    TitleTextChanged(String),
    TitleCommitRequested(String),
    ToggleSourceViewRequested,
    ToggleMindmapStylePanelRequested,
    MindmapStylePanel(ui::core::widget::MindmapStylePanelAction),
    SemanticEditRequested(ui::plugin::SemanticEditCommand),
    MoveDialogRequested(notora_core::NoteId),
    MoveRequested {
        note_id: notora_core::NoteId,
        target_directory: PathBuf,
    },
    MetadataMutationRequested(MetadataMutation),
    MetadataMutationCompleted {
        note_id: NoteId,
        metadata: NoteEditorMetadata,
        selection_generation: u64,
    },
    MetadataMutationFailed(String),
    TrashOperationRequested(TrashOperation),
    TrashPermanentDeletionConfirmed,
    TrashRestoreWithRenamedPathConfirmed,
    TrashOperationCompleted,
    TrashOperationFailed(TrashOperationFailure),
    SaveConflictDetected {
        identity: DocumentIdentity,
        content_revision: u64,
    },
    SaveConflictResolutionRequested(ConflictResolution),
    SaveConflictResolved {
        identity: DocumentIdentity,
    },
    NoteCommandCompleted(NoteCommandResult),
    NoteCommandFailed(String),
    SplitterDragged {
        pane: Pane,
        logical_width: f32,
    },
    FocusRequested(FocusTarget),
    OpenSettings,
    ProductSettingsUpdateRequested(ProductSettingsUpdate),
    RetryProductSettingsPersistence,
    SettingsViewChanged,
    OverlayDismissed,
    EscapePressed,
}

/// 纯 reducer 输出；所有外部 I/O 由后续 effect executor 执行。
#[derive(Clone, Debug, PartialEq)]
pub enum NotoraEffect {
    QueryCards(CardQuery),
    RequestNoteCreation { kind: DocumentKind, target: NoteCreationTarget },
    ExecuteNoteCommand(NoteCommand),
    CommitTitle(String),
    ToggleEditorView,
    ToggleMindmapStylePanel,
    DispatchMindmapStylePanel(ui::core::widget::MindmapStylePanelAction),
    ExecuteSemanticEdit(ui::plugin::SemanticEditCommand),
    ExecuteMetadataMutation(MetadataMutation),
    ExecuteTrashOperation(TrashOperation),
    ChooseNoteMoveDirectory(notora_core::NoteId),
    PrepareDocument(DocumentLoadRequest),
    PromoteActivePreview,
    ChooseWorkspaceRoot,
    OpenExternalFiles(crate::effect_executor::ExternalOpenRequest),
    CreateUntitledExternal(DocumentKind),
    ResolveSaveConflict(SaveConflictRequest),
    ApplyProductSettingsUpdate(ProductSettingsUpdate),
    PersistProductSettings,
    PersistLayout,
    Redraw,
}

pub fn move_note_command(note_id: notora_core::NoteId, target_directory: PathBuf) -> NoteCommand {
    NoteCommand::Move(MoveNoteRequest { note_id, target_directory })
}

impl From<NavigationScope> for CardQuery {
    fn from(scope: NavigationScope) -> Self {
        Self { scope, cursor: None, page_size: DEFAULT_CARD_PAGE_SIZE, search_generation: None }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CardPageCursor, CardQuery, DEFAULT_CARD_PAGE_SIZE, MetadataMutation, NotoraAction,
        NotoraEffect,
    };
    use notora_core::NavigationScope;

    #[test]
    fn card_query_keeps_navigation_scope_typed() {
        let query = CardQuery::from(NavigationScope::Starred);
        assert_eq!(query.scope, NavigationScope::Starred);
        assert_eq!(query.cursor, None);
        assert_eq!(query.page_size, DEFAULT_CARD_PAGE_SIZE);
    }

    #[test]
    fn next_page_retains_scope_and_uses_a_typed_stable_cursor() {
        let cursor = CardPageCursor {
            modified_nanoseconds: 42,
            relative_path: "notes/roadmap.md".into(),
            note_id: notora_core::NoteId::generate(),
        };
        let next_query = CardQuery::from(NavigationScope::WorkspaceRoot).next_page(cursor.clone());

        assert_eq!(next_query.scope, NavigationScope::WorkspaceRoot);
        assert_eq!(next_query.cursor, Some(cursor));
    }

    #[test]
    fn metadata_mutations_keep_tag_changes_typed_and_note_scoped() {
        let note_id = notora_core::NoteId::generate();
        let tag_id = notora_core::TagId::generate();

        assert_eq!(
            MetadataMutation::AttachTagByName { note_id, display_name: "产品/Notora".to_owned() },
            MetadataMutation::AttachTagByName { note_id, display_name: "产品/Notora".to_owned() }
        );
        assert_ne!(
            MetadataMutation::DetachTag { note_id, tag_id },
            MetadataMutation::ToggleStar { note_id }
        );
    }

    #[test]
    fn semantic_edit_requests_remain_typed_until_the_runtime_boundary() {
        let mut state = crate::state::NotoraState::default();
        let effects = state.reduce(NotoraAction::SemanticEditRequested(
            ui::plugin::SemanticEditCommand::ToggleBold,
        ));

        assert!(effects.contains(&NotoraEffect::ExecuteSemanticEdit(
            ui::plugin::SemanticEditCommand::ToggleBold,
        )));
    }
}
