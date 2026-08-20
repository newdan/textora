use std::time::Instant;

use crate::session::ProductSession;

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
}

impl SessionRestoreRuntime {
    pub(super) fn start(&mut self, restore: SessionRestore) {
        self.active_restore = Some(restore);
    }

    pub(super) fn cancel(&mut self) {
        self.active_restore = None;
    }

    pub(super) fn take(&mut self) -> Option<SessionRestore> {
        self.active_restore.take()
    }

    #[cfg(test)]
    pub(super) fn is_active(&self) -> bool {
        self.active_restore.is_some()
    }
}
