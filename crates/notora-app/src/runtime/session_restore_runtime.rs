use std::time::Instant;

use crate::session::ProductSession;

/// 会话恢复结束后的焦点策略。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SessionRestoreFocusPolicy {
    #[default]
    RestoreSavedSelection,
    PreserveSystemOpenTarget,
}

/// 首帧之后正在进行的会话恢复；仅保存跨异步 workspace bootstrap 所需的状态。
pub(super) struct SessionRestore {
    pub(super) session: ProductSession,
    pub(super) workspace_generation: u64,
    pub(super) restore_started_at: Instant,
    pub(super) workspace_started_at: Instant,
}

#[derive(Default)]
pub(super) struct SessionRestoreRuntime {
    active_restore: Option<SessionRestore>,
    focus_policy: SessionRestoreFocusPolicy,
}

impl SessionRestoreRuntime {
    pub(super) fn start(&mut self, restore: SessionRestore) {
        self.active_restore = Some(restore);
    }

    pub(super) fn preserve_system_open_target(&mut self) {
        self.focus_policy = SessionRestoreFocusPolicy::PreserveSystemOpenTarget;
    }

    pub(super) fn take_focus_policy(&mut self) -> SessionRestoreFocusPolicy {
        std::mem::take(&mut self.focus_policy)
    }

    pub(super) fn cancel(&mut self) {
        self.active_restore = None;
        self.focus_policy = SessionRestoreFocusPolicy::RestoreSavedSelection;
    }

    pub(super) fn take(&mut self) -> Option<SessionRestore> {
        self.active_restore.take()
    }

    pub(super) fn is_active(&self) -> bool {
        self.active_restore.is_some()
    }
}
