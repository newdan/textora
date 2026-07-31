use notora_core::{DocumentIdentity, DocumentKind, NavigationScope};

use crate::action::{
    CardQuery, DocumentLoadRequest, NoteCreationTarget, NotoraAction, NotoraEffect,
    move_note_command, rename_note_command,
};
use crate::effect_executor::ExternalOpenRequest;
use crate::external_files::ExternalFileSessions;

/// 当前键盘输入应交给的唯一目标。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusTarget {
    NavigationSearch,
    #[default]
    NavigationTree,
    CardList,
    Editor,
    Overlay,
}

/// 不可重叠的产品 overlay 状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayState {
    #[default]
    None,
    Settings,
    NewDocumentMenu,
}

/// 响应式三栏壳的互斥布局模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResponsiveLayoutMode {
    #[default]
    ThreePane,
    NavigationOverlay,
    EditorOverlay,
}

/// 分隔条影响的相邻 pane。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Navigation,
    CardList,
}

/// 只包含产品导航、选择和卡片查询状态；编辑会话保留在 EditorRuntime。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryState {
    pub navigation_scope: NavigationScope,
    pub search_scope_before_search: Option<NavigationScope>,
    pub selected_card: Option<DocumentIdentity>,
    pub selected_document_generation: u64,
    pub last_command_error: Option<String>,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            navigation_scope: NavigationScope::WorkspaceRoot,
            search_scope_before_search: None,
            selected_card: None,
            selected_document_generation: 0,
            last_command_error: None,
        }
    }
}

/// 仅保存窗口壳的纯布局状态。
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutState {
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
    pub responsive_mode: ResponsiveLayoutMode,
    pub focus_target: FocusTarget,
    pub overlay: OverlayState,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            navigation_width_logical: 220.0,
            card_list_width_logical: 340.0,
            responsive_mode: ResponsiveLayoutMode::ThreePane,
            focus_target: FocusTarget::NavigationTree,
            overlay: OverlayState::None,
        }
    }
}

/// notora 的纯产品状态；不持有 catalog、文件句柄或 editor session。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotoraState {
    pub library: LibraryState,
    /// 外部 session 不属于 catalog、搜索、星标、标签或 Trash 的任何一项。
    pub external_files: ExternalFileSessions,
    pub layout: LayoutState,
}

impl NotoraState {
    pub fn reduce(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::NavigationSelected(scope) => self.select_navigation_scope(scope),
            NotoraAction::SearchCommitted(query) => self.commit_search(query),
            NotoraAction::CardSelected(identity) => {
                let request = self.select_document(identity);
                self.layout.focus_target = FocusTarget::CardList;
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::OpenExternalFileDialogRequested => {
                vec![NotoraEffect::OpenExternalFiles(ExternalOpenRequest::ShowFileDialog)]
            }
            NotoraAction::ExternalPathsReceived(paths) => {
                vec![NotoraEffect::OpenExternalFiles(ExternalOpenRequest::Paths(paths))]
            }
            NotoraAction::ExternalFileOpened(identity) => {
                let request = self.select_document(identity);
                self.library.navigation_scope = NavigationScope::ExternalFiles;
                self.layout.focus_target = FocusTarget::CardList;
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::PromotePreviewRequested => {
                vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
            }
            NotoraAction::CreateRequested(kind) => self.request_note_creation(kind),
            NotoraAction::RenameRequested { note_id, new_file_name } => {
                self.library.last_command_error = None;
                vec![
                    NotoraEffect::ExecuteNoteCommand(rename_note_command(note_id, new_file_name)),
                    NotoraEffect::Redraw,
                ]
            }
            NotoraAction::NoteCommandCompleted(result) => {
                self.apply_note_command_completion(result)
            }
            NotoraAction::NoteCommandFailed(message) => {
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
            NotoraAction::SplitterDragged { pane, logical_width } => {
                self.set_pane_width(pane, logical_width);
                vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]
            }
            NotoraAction::FocusRequested(focus_target) => {
                self.layout.focus_target = focus_target;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::OpenSettings => {
                self.layout.overlay = OverlayState::Settings;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::OverlayDismissed => self.dismiss_overlay(),
            NotoraAction::EscapePressed => self.handle_escape(),
        }
    }

    fn select_navigation_scope(&mut self, scope: NavigationScope) -> Vec<NotoraEffect> {
        if !matches!(scope, NavigationScope::Search { .. }) {
            self.library.search_scope_before_search = None;
        }
        self.library.navigation_scope = scope.clone();
        self.layout.focus_target = FocusTarget::NavigationTree;
        if scope == NavigationScope::ExternalFiles {
            return vec![NotoraEffect::Redraw];
        }
        vec![NotoraEffect::QueryCards(CardQuery::from(scope)), NotoraEffect::Redraw]
    }

    fn commit_search(&mut self, query: String) -> Vec<NotoraEffect> {
        if query.is_empty() {
            let scope = self
                .library
                .search_scope_before_search
                .take()
                .unwrap_or(NavigationScope::WorkspaceRoot);
            self.library.navigation_scope = scope.clone();
            self.layout.focus_target = FocusTarget::NavigationTree;
            return vec![NotoraEffect::QueryCards(CardQuery::from(scope)), NotoraEffect::Redraw];
        }

        if self.library.search_scope_before_search.is_none()
            && !matches!(self.library.navigation_scope, NavigationScope::Search { .. })
        {
            self.library.search_scope_before_search = Some(self.library.navigation_scope.clone());
        }
        let scope = NavigationScope::Search { query };
        self.library.navigation_scope = scope.clone();
        self.layout.focus_target = FocusTarget::CardList;
        vec![NotoraEffect::QueryCards(CardQuery::from(scope)), NotoraEffect::Redraw]
    }

    fn request_note_creation(&mut self, kind: DocumentKind) -> Vec<NotoraEffect> {
        let Some(target) = creation_target(&self.library.navigation_scope) else {
            return vec![NotoraEffect::Redraw];
        };
        self.library.last_command_error = None;
        vec![NotoraEffect::ExecuteNoteCommand(target.create_command(kind)), NotoraEffect::Redraw]
    }

    fn apply_note_command_completion(
        &mut self,
        result: notora_core::note_command::NoteCommandResult,
    ) -> Vec<NotoraEffect> {
        let identity = DocumentIdentity::Note(result.note.note_id);
        let request = self.select_document(identity);
        self.library.last_command_error = None;
        self.layout.focus_target = FocusTarget::CardList;
        let scope = self.library.navigation_scope.clone();
        vec![
            NotoraEffect::QueryCards(CardQuery::from(scope)),
            NotoraEffect::PrepareDocument(request),
            NotoraEffect::Redraw,
        ]
    }

    fn set_pane_width(&mut self, pane: Pane, logical_width: f32) {
        match pane {
            Pane::Navigation => self.layout.navigation_width_logical = logical_width,
            Pane::CardList => self.layout.card_list_width_logical = logical_width,
        }
    }

    fn select_document(&mut self, identity: DocumentIdentity) -> DocumentLoadRequest {
        self.library.selected_card = Some(identity);
        self.library.selected_document_generation =
            self.library.selected_document_generation.wrapping_add(1);
        DocumentLoadRequest {
            identity,
            selection_generation: self.library.selected_document_generation,
        }
    }

    fn dismiss_overlay(&mut self) -> Vec<NotoraEffect> {
        if self.layout.overlay == OverlayState::None {
            return vec![NotoraEffect::Redraw];
        }
        self.layout.overlay = OverlayState::None;
        self.layout.focus_target = FocusTarget::NavigationTree;
        vec![NotoraEffect::Redraw]
    }

    fn handle_escape(&mut self) -> Vec<NotoraEffect> {
        if self.layout.overlay != OverlayState::None {
            return self.dismiss_overlay();
        }
        if matches!(self.library.navigation_scope, NavigationScope::Search { .. }) {
            return self.commit_search(String::new());
        }
        self.layout.focus_target = FocusTarget::NavigationTree;
        vec![NotoraEffect::Redraw]
    }
}

fn creation_target(scope: &NavigationScope) -> Option<NoteCreationTarget> {
    match scope {
        NavigationScope::Trash | NavigationScope::ExternalFiles => None,
        NavigationScope::Directory { relative_path } => {
            Some(NoteCreationTarget { directory: Some(relative_path.clone()), tag_to_attach: None })
        }
        NavigationScope::Tag { tag_id } => {
            Some(NoteCreationTarget { directory: None, tag_to_attach: Some(*tag_id) })
        }
        NavigationScope::Search { .. }
        | NavigationScope::WorkspaceRoot
        | NavigationScope::Starred => {
            Some(NoteCreationTarget { directory: None, tag_to_attach: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FocusTarget, LibraryState, NotoraState, OverlayState};
    use crate::action::{CardQuery, NotoraAction, NotoraEffect};
    use notora_core::{DocumentKind, NavigationScope, TagId};

    #[test]
    fn starts_in_workspace_root_scope() {
        assert_eq!(LibraryState::default().navigation_scope, NavigationScope::WorkspaceRoot);
    }

    #[test]
    fn empty_search_restores_the_scope_before_search() {
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Starred));
        let _ = state.reduce(NotoraAction::SearchCommitted("roadmap".to_owned()));

        assert_eq!(
            state.reduce(NotoraAction::SearchCommitted(String::new())),
            vec![
                NotoraEffect::QueryCards(CardQuery::from(NavigationScope::Starred)),
                NotoraEffect::Redraw,
            ]
        );
        assert_eq!(state.library.navigation_scope, NavigationScope::Starred);
    }

    #[test]
    fn trash_scope_cannot_request_note_creation() {
        let mut state = NotoraState::default();
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
    fn tag_scope_requests_tag_attachment_for_new_notes() {
        let mut state = NotoraState::default();
        let tag_id = TagId::generate();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id }));

        assert_eq!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
            vec![
                NotoraEffect::ExecuteNoteCommand(notora_core::note_command::NoteCommand::Create(
                    notora_core::note_command::CreateNoteRequest {
                        kind: DocumentKind::Markdown,
                        target_directory: None,
                        tag_to_attach: Some(tag_id),
                    },
                ),),
                NotoraEffect::Redraw,
            ]
        );
    }

    #[test]
    fn note_requests_reduce_to_a_typed_domain_command_effect() {
        let mut state = NotoraState::default();

        assert!(matches!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)).as_slice(),
            [
                NotoraEffect::ExecuteNoteCommand(notora_core::note_command::NoteCommand::Create(
                    notora_core::note_command::CreateNoteRequest {
                        kind: DocumentKind::Markdown,
                        ..
                    }
                )),
                NotoraEffect::Redraw
            ]
        ));
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
    fn escape_closes_overlay_then_clears_search_then_focuses_navigation() {
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::OpenSettings);
        assert_eq!(state.layout.overlay, OverlayState::Settings);
        let _ = state.reduce(NotoraAction::EscapePressed);
        assert_eq!(state.layout.overlay, OverlayState::None);

        let _ = state.reduce(NotoraAction::SearchCommitted("idea".to_owned()));
        let _ = state.reduce(NotoraAction::EscapePressed);
        assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);

        state.layout.focus_target = FocusTarget::Editor;
        let _ = state.reduce(NotoraAction::EscapePressed);
        assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
    }
}
