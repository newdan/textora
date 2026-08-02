//! editor runtime 的有界驻留策略。

use std::num::NonZeroUsize;

use appkit_core::workspace::types::TabId;
use notora_core::DocumentIdentity;

use crate::document_registry::{DocumentRegistry, RegisteredDocument};

/// 每个候选的运行时状态，由 app 层从 EditorRuntime 和 autosave 显式映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeTabState {
    pub tab_id: TabId,
    pub is_dirty: bool,
    pub is_saving: bool,
    pub is_pinned: bool,
    pub is_active: bool,
}

/// 一个干净、非活动且非 pinned 的 runtime 淘汰候选。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeEvictionCandidate {
    pub identity: DocumentIdentity,
    pub tab_id: TabId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeLru {
    maximum_resident_tabs: NonZeroUsize,
}

impl RuntimeLru {
    pub fn new(maximum_resident_tabs: NonZeroUsize) -> Self {
        Self { maximum_resident_tabs }
    }

    pub fn maximum_resident_tabs(self) -> usize {
        self.maximum_resident_tabs.get()
    }

    /// 只在超过上限时选择候选。预览 tab 一律保留，避免升级过程被淘汰。
    pub fn select_evictions(
        self,
        registry: &DocumentRegistry,
        runtime_tabs: &[RuntimeTabState],
    ) -> Vec<RuntimeEvictionCandidate> {
        let excess = runtime_tabs.len().saturating_sub(self.maximum_resident_tabs());
        if excess == 0 {
            return Vec::new();
        }
        registry
            .documents_by_least_recently_used()
            .into_iter()
            .filter(|document| is_evictable(*document, runtime_tabs))
            .take(excess)
            .map(|document| RuntimeEvictionCandidate {
                identity: document.identity,
                tab_id: document.tab_id,
            })
            .collect()
    }
}

fn is_evictable(document: RegisteredDocument, runtime_tabs: &[RuntimeTabState]) -> bool {
    if document.is_preview {
        return false;
    }
    let Some(runtime_state) = runtime_tabs.iter().find(|state| state.tab_id == document.tab_id)
    else {
        return false;
    };
    !runtime_state.is_dirty
        && !runtime_state.is_saving
        && !runtime_state.is_pinned
        && !runtime_state.is_active
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use appkit_core::workspace::types::TabIdAllocator;
    use notora_core::{DocumentIdentity, NoteId};

    use super::{RuntimeLru, RuntimeTabState};
    use crate::document_registry::DocumentRegistry;

    #[test]
    fn lru_evicts_only_clean_non_active_non_pinned_persistent_tabs() {
        let mut tabs = TabIdAllocator::new();
        let first_tab = tabs.allocate();
        let dirty_tab = tabs.allocate();
        let active_tab = tabs.allocate();
        let preview_tab = tabs.allocate();
        let mut registry = DocumentRegistry::default();
        let first_identity = DocumentIdentity::Note(NoteId::generate());
        let dirty_identity = DocumentIdentity::Note(NoteId::generate());
        let active_identity = DocumentIdentity::Note(NoteId::generate());
        let preview_identity = DocumentIdentity::Note(NoteId::generate());
        let _ = registry.register(first_identity, first_tab);
        let _ = registry.register(dirty_identity, dirty_tab);
        let _ = registry.register(active_identity, active_tab);
        let _ = registry.register_preview(preview_identity, preview_tab);
        let lru = RuntimeLru::new(NonZeroUsize::new(2).expect("positive limit should exist"));

        assert_eq!(
            lru.select_evictions(
                &registry,
                &[
                    RuntimeTabState {
                        tab_id: first_tab,
                        is_dirty: false,
                        is_saving: false,
                        is_pinned: false,
                        is_active: false
                    },
                    RuntimeTabState {
                        tab_id: dirty_tab,
                        is_dirty: true,
                        is_saving: false,
                        is_pinned: false,
                        is_active: false
                    },
                    RuntimeTabState {
                        tab_id: active_tab,
                        is_dirty: false,
                        is_saving: false,
                        is_pinned: false,
                        is_active: true
                    },
                    RuntimeTabState {
                        tab_id: preview_tab,
                        is_dirty: false,
                        is_saving: false,
                        is_pinned: false,
                        is_active: false
                    },
                ],
            ),
            vec![super::RuntimeEvictionCandidate { identity: first_identity, tab_id: first_tab }]
        );
    }

    #[test]
    fn runtime_lru_keeps_all_tabs_when_every_candidate_is_protected() {
        let mut tabs = TabIdAllocator::new();
        let dirty_tab = tabs.allocate();
        let active_tab = tabs.allocate();
        let mut registry = DocumentRegistry::default();
        let dirty_identity = DocumentIdentity::Note(NoteId::generate());
        let active_identity = DocumentIdentity::Note(NoteId::generate());
        let _ = registry.register(dirty_identity, dirty_tab);
        let _ = registry.register(active_identity, active_tab);
        let lru = RuntimeLru::new(NonZeroUsize::new(1).expect("positive limit should exist"));

        assert!(
            lru.select_evictions(
                &registry,
                &[
                    RuntimeTabState {
                        tab_id: dirty_tab,
                        is_dirty: true,
                        is_saving: false,
                        is_pinned: false,
                        is_active: false,
                    },
                    RuntimeTabState {
                        tab_id: active_tab,
                        is_dirty: false,
                        is_saving: false,
                        is_pinned: false,
                        is_active: true,
                    },
                ],
            )
            .is_empty()
        );
    }

    #[test]
    fn runtime_lru_ignores_late_runtime_state_after_registry_cleanup() {
        let mut tabs = TabIdAllocator::new();
        let tab_id = tabs.allocate();
        let identity = DocumentIdentity::Note(NoteId::generate());
        let mut registry = DocumentRegistry::default();
        let _ = registry.register(identity, tab_id);
        let _ = registry.remove_tab(tab_id);
        let lru = RuntimeLru::new(NonZeroUsize::new(1).expect("positive limit should exist"));

        assert!(
            lru.select_evictions(
                &registry,
                &[RuntimeTabState {
                    tab_id,
                    is_dirty: false,
                    is_saving: false,
                    is_pinned: false,
                    is_active: false,
                }],
            )
            .is_empty()
        );
    }
}
