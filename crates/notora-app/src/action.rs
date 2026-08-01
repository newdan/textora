//! notora 产品动作与 reducer effect。

use std::path::PathBuf;

use notora_core::note_command::{
    CreateNoteRequest, MoveNoteRequest, NoteCommand, NoteCommandResult, RenameNoteRequest,
};
use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, TagId};

use crate::state::{FocusTarget, Pane};

/// 中栏查询的纯输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardQuery {
    pub scope: NavigationScope,
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

/// 产品层的类型化用户动作。
#[derive(Clone, Debug, PartialEq)]
pub enum NotoraAction {
    NavigationSelected(NavigationScope),
    SearchCommitted(String),
    CardSelected(DocumentIdentity),
    OpenExternalFileDialogRequested,
    ExternalPathsReceived(Vec<PathBuf>),
    ExternalFileOpened(DocumentIdentity),
    PromotePreviewRequested,
    CreateRequested(DocumentKind),
    RenameRequested { note_id: notora_core::NoteId, new_file_name: PathBuf },
    MoveRequested { note_id: notora_core::NoteId, target_directory: PathBuf },
    SaveConflictDetected { identity: DocumentIdentity, content_revision: u64 },
    SaveConflictResolutionRequested(ConflictResolution),
    SaveConflictResolved { identity: DocumentIdentity },
    NoteCommandCompleted(NoteCommandResult),
    NoteCommandFailed(String),
    SplitterDragged { pane: Pane, logical_width: f32 },
    FocusRequested(FocusTarget),
    OpenSettings,
    OverlayDismissed,
    EscapePressed,
}

/// 纯 reducer 输出；所有外部 I/O 由后续 effect executor 执行。
#[derive(Clone, Debug, PartialEq)]
pub enum NotoraEffect {
    QueryCards(CardQuery),
    ExecuteNoteCommand(NoteCommand),
    PrepareDocument(DocumentLoadRequest),
    PromoteActivePreview,
    OpenExternalFiles(crate::effect_executor::ExternalOpenRequest),
    ResolveSaveConflict(SaveConflictRequest),
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
        Self { scope }
    }
}

#[cfg(test)]
mod tests {
    use super::CardQuery;
    use notora_core::NavigationScope;

    #[test]
    fn card_query_keeps_navigation_scope_typed() {
        let query = CardQuery::from(NavigationScope::Starred);
        assert_eq!(query.scope, NavigationScope::Starred);
    }
}
