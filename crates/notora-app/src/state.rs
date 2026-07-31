use notora_core::{DocumentIdentity, DocumentKind, NavigationScope};

use crate::action::{CardQuery, NoteCreationTarget, NotoraAction, NotoraEffect};

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
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            navigation_scope: NavigationScope::WorkspaceRoot,
            search_scope_before_search: None,
            selected_card: None,
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
    pub layout: LayoutState,
}

impl NotoraState {
    pub fn reduce(&mut self, action: NotoraAction) -> Vec<NotoraEffect> {
        match action {
            NotoraAction::NavigationSelected(scope) => self.select_navigation_scope(scope),
            NotoraAction::SearchCommitted(query) => self.commit_search(query),
            NotoraAction::CardSelected(identity) => {
                self.library.selected_card = Some(identity);
                self.layout.focus_target = FocusTarget::CardList;
                vec![NotoraEffect::PrepareDocument(identity), NotoraEffect::Redraw]
            }
            NotoraAction::CreateRequested(kind) => self.request_note_creation(kind),
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
        vec![NotoraEffect::RequestNoteCreation { kind, target }, NotoraEffect::Redraw]
    }

    fn set_pane_width(&mut self, pane: Pane, logical_width: f32) {
        match pane {
            Pane::Navigation => self.layout.navigation_width_logical = logical_width,
            Pane::CardList => self.layout.card_list_width_logical = logical_width,
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
    use crate::action::{CardQuery, NoteCreationTarget, NotoraAction, NotoraEffect};
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
    fn tag_scope_requests_tag_attachment_for_new_notes() {
        let mut state = NotoraState::default();
        let tag_id = TagId::generate();
        let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id }));

        assert_eq!(
            state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
            vec![
                NotoraEffect::RequestNoteCreation {
                    kind: DocumentKind::Markdown,
                    target: NoteCreationTarget { directory: None, tag_to_attach: Some(tag_id) },
                },
                NotoraEffect::Redraw,
            ]
        );
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
