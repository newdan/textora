//! notora 产品动作与 reducer effect。

use std::path::PathBuf;

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

/// 产品层的类型化用户动作。
#[derive(Clone, Debug, PartialEq)]
pub enum NotoraAction {
    NavigationSelected(NavigationScope),
    SearchCommitted(String),
    CardSelected(DocumentIdentity),
    CreateRequested(DocumentKind),
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
    RequestNoteCreation { kind: DocumentKind, target: NoteCreationTarget },
    PrepareDocument(DocumentIdentity),
    PersistLayout,
    Redraw,
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
