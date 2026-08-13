use std::collections::HashMap;

use appkit_core::workspace::types::TabId;

use super::document_runtime::WorkspaceNoteSaveCandidate;
use crate::action::WorkspaceTransitionRequest;

#[derive(Debug)]
struct PendingWorkspaceTransition {
    request: WorkspaceTransitionRequest,
    pending_saves: HashMap<TabId, WorkspaceNoteSaveCandidate>,
}

/// 安全工作区切换的保存屏障状态所有者。
#[derive(Debug, Default)]
pub(super) struct WorkspaceTransitionRuntime {
    pending: Option<PendingWorkspaceTransition>,
}

impl WorkspaceTransitionRuntime {
    pub(super) fn is_active(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn begin(
        &mut self,
        request: WorkspaceTransitionRequest,
        candidates: &[WorkspaceNoteSaveCandidate],
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let pending_saves =
            candidates.iter().copied().map(|candidate| (candidate.tab_id, candidate)).collect();
        self.pending = Some(PendingWorkspaceTransition { request, pending_saves });
        true
    }

    pub(super) fn save_candidates(&self) -> Vec<WorkspaceNoteSaveCandidate> {
        self.pending
            .as_ref()
            .map(|transition| transition.pending_saves.values().copied().collect())
            .unwrap_or_default()
    }

    pub(super) fn complete_saves(&mut self, tab_ids: impl IntoIterator<Item = TabId>) {
        let Some(transition) = self.pending.as_mut() else {
            return;
        };
        for tab_id in tab_ids {
            transition.pending_saves.remove(&tab_id);
        }
    }

    pub(super) fn take_ready_request(&mut self) -> Option<WorkspaceTransitionRequest> {
        if self.pending.as_ref().is_none_or(|transition| !transition.pending_saves.is_empty()) {
            return None;
        }
        self.pending.take().map(|transition| transition.request)
    }

    pub(super) fn cancel(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use appkit_core::workspace::types::TabIdAllocator;

    use super::WorkspaceTransitionRuntime;
    use crate::action::WorkspaceTransitionRequest;
    use crate::runtime::document_runtime::WorkspaceNoteSaveCandidate;

    #[test]
    fn transition_becomes_ready_only_after_every_save_candidate_completes() {
        let mut tabs = TabIdAllocator::new();
        let first = WorkspaceNoteSaveCandidate { tab_id: tabs.allocate(), content_revision: 1 };
        let second = WorkspaceNoteSaveCandidate { tab_id: tabs.allocate(), content_revision: 2 };
        let request = WorkspaceTransitionRequest::OpenExisting { root: "/workspace".into() };
        let mut runtime = WorkspaceTransitionRuntime::default();

        assert!(runtime.begin(request.clone(), &[first, second]));
        runtime.complete_saves([first.tab_id]);
        assert_eq!(runtime.take_ready_request(), None);
        runtime.complete_saves([second.tab_id]);
        assert_eq!(runtime.take_ready_request(), Some(request));
    }
}
