use notora_core::{
    CatalogCard, CatalogCardCursor, DocumentIdentity, DocumentKind, NavigationScope,
};

use crate::action::{
    CardQuery, ConflictResolution, DocumentLoadRequest, NoteCreationTarget, NotoraAction,
    NotoraEffect, SaveConflictRequest, move_note_command, rename_note_command,
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
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryState {
    pub navigation_scope: NavigationScope,
    pub search_scope_before_search: Option<NavigationScope>,
    pub search_text: String,
    pub card_page: CardPageState,
    pub card_scroll_offset_px: f32,
    pub selected_card: Option<DocumentIdentity>,
    pub selected_document_generation: u64,
    pub last_command_error: Option<String>,
    pub save_conflict: Option<SaveConflict>,
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
            selected_document_generation: 0,
            last_command_error: None,
            save_conflict: None,
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
                vec![NotoraEffect::PrepareDocument(request), NotoraEffect::Redraw]
            }
            NotoraAction::CardActivated(identity) => {
                if self.library.selected_card != Some(identity) {
                    return vec![NotoraEffect::Redraw];
                }
                self.layout.focus_target = FocusTarget::Editor;
                vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
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
            NotoraAction::OpenNewDocumentMenu => {
                self.layout.overlay = OverlayState::NewDocumentMenu;
                self.layout.focus_target = FocusTarget::Overlay;
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::CreateRequested(kind) => self.request_note_creation(kind),
            NotoraAction::RenameDialogRequested(note_id) => {
                vec![NotoraEffect::ChooseNoteRenameDestination(note_id), NotoraEffect::Redraw]
            }
            NotoraAction::MoveDialogRequested(note_id) => {
                vec![NotoraEffect::ChooseNoteMoveDirectory(note_id), NotoraEffect::Redraw]
            }
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
            NotoraAction::SaveConflictDetected { identity, content_revision } => {
                self.library.save_conflict = Some(SaveConflict { identity, content_revision });
                vec![NotoraEffect::Redraw]
            }
            NotoraAction::SaveConflictResolutionRequested(resolution) => {
                self.resolve_save_conflict(resolution)
            }
            NotoraAction::SaveConflictResolved { identity } => {
                if self.library.save_conflict.map(|conflict| conflict.identity) == Some(identity) {
                    self.library.save_conflict = None;
                }
                vec![NotoraEffect::Redraw]
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
            self.library.search_text.clear();
        }
        self.library.navigation_scope = scope.clone();
        self.layout.focus_target = FocusTarget::NavigationTree;
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
            self.library.navigation_scope = scope.clone();
            self.layout.focus_target = FocusTarget::NavigationTree;
            return self.request_card_query(CardQuery::from(scope));
        }

        if self.library.search_scope_before_search.is_none()
            && !matches!(self.library.navigation_scope, NavigationScope::Search { .. })
        {
            self.library.search_scope_before_search = Some(self.library.navigation_scope.clone());
        }
        let scope = NavigationScope::Search { query };
        self.library.navigation_scope = scope.clone();
        self.layout.focus_target = FocusTarget::NavigationSearch;
        let mut card_query = CardQuery::from(scope);
        card_query.search_generation = search_generation;
        self.request_card_query(card_query)
    }

    fn request_note_creation(&mut self, kind: DocumentKind) -> Vec<NotoraEffect> {
        if self.layout.overlay == OverlayState::NewDocumentMenu {
            self.layout.overlay = OverlayState::None;
            self.layout.focus_target = FocusTarget::CardList;
        }
        let Some(target) = creation_target(&self.library.navigation_scope) else {
            return vec![NotoraEffect::Redraw];
        };
        self.library.last_command_error = None;
        vec![NotoraEffect::ExecuteNoteCommand(target.create_command(kind)), NotoraEffect::Redraw]
    }

    fn resolve_save_conflict(&mut self, resolution: ConflictResolution) -> Vec<NotoraEffect> {
        let Some(conflict) = self.library.save_conflict else {
            return vec![NotoraEffect::Redraw];
        };
        if resolution == ConflictResolution::Cancel {
            self.library.save_conflict = None;
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
        let request = self.select_document(identity);
        self.library.last_command_error = None;
        self.layout.focus_target = FocusTarget::CardList;
        let scope = self.library.navigation_scope.clone();
        let mut effects = self.request_card_query(CardQuery::from(scope));
        effects.insert(1, NotoraEffect::PrepareDocument(request));
        effects
    }

    fn request_card_query(&mut self, query: CardQuery) -> Vec<NotoraEffect> {
        self.library.card_scroll_offset_px = 0.0;
        self.library.card_page = CardPageState::LoadingInitial { query: query.clone() };
        vec![NotoraEffect::QueryCards(query), NotoraEffect::Redraw]
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
        vec![NotoraEffect::Redraw]
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
    use std::time::{Duration, Instant};

    use super::{CardPageState, FocusTarget, LibraryState, NotoraState, OverlayState};
    use crate::action::{CardQuery, NotoraAction, NotoraEffect};
    use crate::search_controller::{SEARCH_DEBOUNCE_DELAY, SearchController};
    use notora_core::{
        CatalogCard, CatalogCardCursor, CatalogCardPage, DocumentIdentity, DocumentKind,
        NavigationScope, NoteId, TagId,
    };

    fn card(note_id: NoteId, title: &str, modified_nanoseconds: i64) -> CatalogCard {
        CatalogCard {
            note_id,
            relative_path: format!("notes/{title}.md").into(),
            kind: DocumentKind::Markdown,
            title: title.to_owned(),
            excerpt: format!("{title} excerpt"),
            modified_nanoseconds,
            starred: false,
            tags: vec!["计划".to_owned()],
        }
    }

    #[test]
    fn starts_in_workspace_root_scope() {
        assert_eq!(LibraryState::default().navigation_scope, NavigationScope::WorkspaceRoot);
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
    fn rename_and_move_dialog_entries_stay_behind_typed_effects() {
        let mut state = NotoraState::default();
        let note_id = NoteId::generate();

        assert_eq!(
            state.reduce(NotoraAction::RenameDialogRequested(note_id)),
            vec![NotoraEffect::ChooseNoteRenameDestination(note_id), NotoraEffect::Redraw]
        );
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
    fn new_document_menu_owns_focus_until_dismissed() {
        let mut state = NotoraState::default();

        assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
        assert_eq!(state.layout.overlay, OverlayState::NewDocumentMenu);
        assert_eq!(state.layout.focus_target, FocusTarget::Overlay);
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
    fn concurrent_save_requires_an_explicit_typed_resolution() {
        let mut state = NotoraState::default();
        let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 7 });

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
}
