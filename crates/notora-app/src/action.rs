//! notora 产品动作与 reducer effect。

use std::path::PathBuf;

use notora_core::note_command::{
    CreateNoteRequest, MoveNoteRequest, NoteCommand, NoteCommandResult, RenameNoteRequest,
};
use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, NoteId, TagId};

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
    pub tag_to_attach: Option<TagId>,
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
    CreateTag { display_name: String },
    RenameTag { tag_id: TagId, display_name: String },
    DeleteTag { tag_id: TagId },
    AttachTag { note_id: NoteId, tag_id: TagId },
    DetachTag { note_id: NoteId, tag_id: TagId },
}

/// 标签名称弹层的单一编辑目标；领域标签身份不交给 UI widget 保存。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagEditorMode {
    Create,
    Rename { tag_id: TagId },
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
    SearchCommitted { query: String, search_generation: Option<SearchGeneration> },
    CardQueryCompleted { query: CardQuery, page: notora_core::CatalogCardPage },
    CardQueryFailed { query: CardQuery, message: String },
    NavigationTreeLoaded(notora_core::CatalogNavigationTree),
    NavigationTreeFailed(String),
    CatalogRecoveryNotified(String),
    NavigationExpansionToggled(PathBuf),
    CardListScrolled { offset_px: f32, near_end: bool },
    CardSelected(DocumentIdentity),
    CardActivated(DocumentIdentity),
    CompactNavigationRequested,
    CompactBackRequested,
    OpenExternalFileDialogRequested,
    ExternalPathsReceived(Vec<PathBuf>),
    ExternalFileOpened(DocumentIdentity),
    PromotePreviewRequested,
    OpenNewDocumentMenu,
    CreateRequested(DocumentKind),
    RenameDialogRequested(notora_core::NoteId),
    MoveDialogRequested(notora_core::NoteId),
    RenameRequested { note_id: notora_core::NoteId, new_file_name: PathBuf },
    MoveRequested { note_id: notora_core::NoteId, target_directory: PathBuf },
    MetadataMutationRequested(MetadataMutation),
    MetadataMutationCompleted,
    MetadataMutationFailed(String),
    TagEditorRequested(TagEditorMode),
    TagEditorNameChanged(String),
    TagEditorConfirmed,
    TagDeletionRequested(TagId),
    TagDeletionConfirmed,
    TrashOperationRequested(TrashOperation),
    TrashPermanentDeletionConfirmed,
    TrashRestoreWithRenamedPathConfirmed,
    TrashOperationCompleted,
    TrashOperationFailed(TrashOperationFailure),
    SaveConflictDetected { identity: DocumentIdentity, content_revision: u64 },
    SaveConflictResolutionRequested(ConflictResolution),
    SaveConflictResolved { identity: DocumentIdentity },
    NoteCommandCompleted(NoteCommandResult),
    NoteCommandFailed(String),
    SplitterDragged { pane: Pane, logical_width: f32 },
    FocusRequested(FocusTarget),
    OpenSettings,
    ProductSettingsUpdateRequested(ProductSettingsUpdate),
    OverlayDismissed,
    EscapePressed,
}

/// 纯 reducer 输出；所有外部 I/O 由后续 effect executor 执行。
#[derive(Clone, Debug, PartialEq)]
pub enum NotoraEffect {
    QueryCards(CardQuery),
    ExecuteNoteCommand(NoteCommand),
    ExecuteMetadataMutation(MetadataMutation),
    ExecuteTrashOperation(TrashOperation),
    ChooseNoteRenameDestination(notora_core::NoteId),
    ChooseNoteMoveDirectory(notora_core::NoteId),
    PrepareDocument(DocumentLoadRequest),
    PromoteActivePreview,
    OpenExternalFiles(crate::effect_executor::ExternalOpenRequest),
    CreateUntitledExternal(DocumentKind),
    ResolveSaveConflict(SaveConflictRequest),
    ApplyProductSettingsUpdate(ProductSettingsUpdate),
    PersistLayout,
    Redraw,
}

impl NoteCreationTarget {
    pub fn create_command(self, kind: DocumentKind) -> NoteCommand {
        NoteCommand::Create(CreateNoteRequest {
            kind,
            target_directory: self.directory,
            tag_to_attach: self.tag_to_attach,
        })
    }
}

pub fn rename_note_command(note_id: notora_core::NoteId, new_file_name: PathBuf) -> NoteCommand {
    NoteCommand::Rename(RenameNoteRequest { note_id, new_file_name })
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
    use super::{CardPageCursor, CardQuery, DEFAULT_CARD_PAGE_SIZE};
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
}
