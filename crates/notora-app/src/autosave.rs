//! 基于内容 revision 的笔记自动保存调度。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use appkit_core::workspace::types::TabId;
use notora_core::DocumentOrigin;

/// 用户停止编辑后开始自动保存的空闲时长。
pub const AUTO_SAVE_IDLE_DELAY: Duration = Duration::from_millis(800);

/// 为测试注入时间来源，避免依赖真实等待。
pub trait AutoSaveClock {
    fn now(&self) -> Instant;
}

/// 生产环境使用的单调时钟。
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemAutoSaveClock;

impl AutoSaveClock for SystemAutoSaveClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// 单个 tab 的自动保存生命周期。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSaveState {
    Idle,
    Scheduled { deadline: Instant, content_revision: u64 },
    Saving { content_revision: u64 },
    Failed { content_revision: u64 },
}

/// 到期后可交给 `EditorRuntime` 准备保存的不可变请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoSaveRequest {
    pub tab_id: TabId,
    pub content_revision: u64,
}

/// 仅为工作区笔记调度自动保存；外部与未命名文件始终由显式保存流程处理。
#[derive(Debug)]
pub struct AutoSaveScheduler<C = SystemAutoSaveClock> {
    clock: C,
    idle_delay: Duration,
    states: HashMap<TabId, AutoSaveState>,
}

impl AutoSaveScheduler<SystemAutoSaveClock> {
    pub fn new() -> Self {
        Self::with_clock(SystemAutoSaveClock)
    }
}

impl Default for AutoSaveScheduler<SystemAutoSaveClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> AutoSaveScheduler<C>
where
    C: AutoSaveClock,
{
    pub fn with_clock(clock: C) -> Self {
        Self::with_clock_and_idle_delay(clock, AUTO_SAVE_IDLE_DELAY)
    }

    pub fn with_clock_and_idle_delay(clock: C, idle_delay: Duration) -> Self {
        Self { clock, idle_delay, states: HashMap::new() }
    }

    pub fn set_idle_delay(&mut self, idle_delay: Duration) {
        self.idle_delay = idle_delay;
    }

    /// 内容实际提交后刷新 deadline。IME preedit 不应调用这个方法。
    pub fn on_content_changed(
        &mut self,
        origin: &DocumentOrigin,
        tab_id: TabId,
        content_revision: u64,
    ) {
        if !matches!(origin, DocumentOrigin::Note { .. }) {
            self.cancel(tab_id);
            return;
        }
        self.schedule(tab_id, content_revision, self.idle_delay);
    }

    /// 明确记录 preedit 被忽略，避免调用方误把 IME 组合态当成一次内容修改。
    pub fn on_ime_preedit(&mut self, _tab_id: TabId) {}

    /// 将工作区笔记设为立即保存；用于 Cmd/Ctrl+S 与退出流程。
    pub fn request_immediate_save(
        &mut self,
        origin: &DocumentOrigin,
        tab_id: TabId,
        content_revision: u64,
    ) {
        if !matches!(origin, DocumentOrigin::Note { .. }) {
            self.cancel(tab_id);
            return;
        }
        self.schedule(tab_id, content_revision, Duration::ZERO);
    }

    /// 取出所有到期请求，并先将它们标记为 in-flight，避免同一 revision 重复提交。
    pub fn take_due_saves(&mut self) -> Vec<AutoSaveRequest> {
        let now = self.clock.now();
        let due_saves = self
            .states
            .iter()
            .filter_map(|(tab_id, state)| match state {
                AutoSaveState::Scheduled { deadline, content_revision } if *deadline <= now => {
                    Some(AutoSaveRequest { tab_id: *tab_id, content_revision: *content_revision })
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for request in &due_saves {
            self.states.insert(
                request.tab_id,
                AutoSaveState::Saving { content_revision: request.content_revision },
            );
        }
        due_saves
    }

    /// 仅匹配当前 in-flight revision 的 completion 才能清除保存状态。
    pub fn on_save_completed(&mut self, request: AutoSaveRequest) {
        if self.state(request.tab_id)
            == Some(AutoSaveState::Saving { content_revision: request.content_revision })
        {
            self.states.insert(request.tab_id, AutoSaveState::Idle);
        }
    }

    /// 保存失败保留 revision，供明确重试或下次内容修改接管。
    pub fn on_save_failed(&mut self, request: AutoSaveRequest) {
        if self.state(request.tab_id)
            == Some(AutoSaveState::Saving { content_revision: request.content_revision })
        {
            self.states.insert(
                request.tab_id,
                AutoSaveState::Failed { content_revision: request.content_revision },
            );
        }
    }

    /// 只允许重试当前失败的笔记 revision；已被新编辑替代的失败不会覆盖新 deadline。
    pub fn retry_failed_save(&mut self, tab_id: TabId) -> bool {
        let Some(AutoSaveState::Failed { content_revision }) = self.state(tab_id) else {
            return false;
        };
        self.schedule(tab_id, content_revision, Duration::ZERO);
        true
    }

    pub fn cancel(&mut self, tab_id: TabId) {
        self.states.remove(&tab_id);
    }

    /// 工作区切换、关闭或恢复流程接管时取消所有待保存状态。
    pub fn clear(&mut self) {
        self.states.clear();
    }

    pub fn state(&self, tab_id: TabId) -> Option<AutoSaveState> {
        self.states.get(&tab_id).copied()
    }

    /// 返回下一个到期时间，使事件循环可以精确唤醒而无需轮询。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.states
            .values()
            .filter_map(|state| match state {
                AutoSaveState::Scheduled { deadline, .. } => Some(*deadline),
                AutoSaveState::Idle
                | AutoSaveState::Saving { .. }
                | AutoSaveState::Failed { .. } => None,
            })
            .min()
    }

    /// 退出时用于判断是否还有已经交给 shared save worker 的自动保存请求。
    pub fn has_in_flight_save(&self) -> bool {
        self.states.values().any(|state| matches!(state, AutoSaveState::Saving { .. }))
    }

    fn schedule(&mut self, tab_id: TabId, content_revision: u64, delay: Duration) {
        let deadline = self.clock.now() + delay;
        self.states.insert(tab_id, AutoSaveState::Scheduled { deadline, content_revision });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use appkit_core::workspace::types::TabIdAllocator;
    use notora_core::{DocumentKind, DocumentOrigin, ExternalFileId, NoteId, WorkspaceId};

    use super::{
        AUTO_SAVE_IDLE_DELAY, AutoSaveClock, AutoSaveScheduler, AutoSaveState, SystemAutoSaveClock,
    };

    #[derive(Debug)]
    struct ManualClock {
        now: Cell<Instant>,
    }

    impl ManualClock {
        fn new() -> Self {
            Self { now: Cell::new(Instant::now()) }
        }

        fn advance(&self, duration: Duration) {
            self.now.set(self.now.get() + duration);
        }
    }

    impl AutoSaveClock for ManualClock {
        fn now(&self) -> Instant {
            self.now.get()
        }
    }

    fn note_origin() -> DocumentOrigin {
        DocumentOrigin::Note {
            workspace_id: WorkspaceId::generate(),
            note_id: NoteId::generate(),
            relative_path: "ideas.md".into(),
        }
    }

    fn external_origin() -> DocumentOrigin {
        DocumentOrigin::ExternalFile {
            external_file_id: ExternalFileId::generate(),
            canonical_path: "/tmp/external.md".into(),
        }
    }

    #[test]
    fn note_changes_schedule_a_save_after_the_idle_delay() {
        let clock = ManualClock::new();
        let mut scheduler = AutoSaveScheduler::with_clock(clock);
        let tab_id = TabIdAllocator::new().allocate();

        scheduler.on_content_changed(&note_origin(), tab_id, 3);
        assert!(matches!(scheduler.state(tab_id), Some(AutoSaveState::Scheduled { .. })));
        assert!(scheduler.take_due_saves().is_empty());

        scheduler.clock.advance(AUTO_SAVE_IDLE_DELAY);
        let saves = scheduler.take_due_saves();

        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].tab_id, tab_id);
        assert_eq!(saves[0].content_revision, 3);
        assert_eq!(scheduler.state(tab_id), Some(AutoSaveState::Saving { content_revision: 3 }));
    }

    #[test]
    fn later_content_revision_replaces_an_expired_deadline() {
        let clock = ManualClock::new();
        let mut scheduler = AutoSaveScheduler::with_clock(clock);
        let tab_id = TabIdAllocator::new().allocate();

        scheduler.on_content_changed(&note_origin(), tab_id, 3);
        scheduler.clock.advance(AUTO_SAVE_IDLE_DELAY - Duration::from_millis(1));
        scheduler.on_content_changed(&note_origin(), tab_id, 4);
        scheduler.clock.advance(Duration::from_millis(1));
        assert!(scheduler.take_due_saves().is_empty());

        scheduler.clock.advance(AUTO_SAVE_IDLE_DELAY - Duration::from_millis(1));
        assert_eq!(scheduler.take_due_saves()[0].content_revision, 4);
    }

    #[test]
    fn external_and_untitled_documents_never_receive_an_idle_deadline() {
        let clock = ManualClock::new();
        let mut scheduler = AutoSaveScheduler::with_clock(clock);
        let mut tab_ids = TabIdAllocator::new();
        let external_tab = tab_ids.allocate();
        let untitled_tab = tab_ids.allocate();
        let untitled_origin = DocumentOrigin::UntitledExternal {
            external_file_id: ExternalFileId::generate(),
            kind: DocumentKind::Markdown,
        };

        scheduler.on_content_changed(&external_origin(), external_tab, 1);
        scheduler.on_content_changed(&untitled_origin, untitled_tab, 1);
        scheduler.clock.advance(AUTO_SAVE_IDLE_DELAY);

        assert!(scheduler.take_due_saves().is_empty());
        assert_eq!(scheduler.state(external_tab), None);
        assert_eq!(scheduler.state(untitled_tab), None);
    }

    #[test]
    fn ime_preedit_does_not_refresh_the_existing_deadline_but_committed_content_does() {
        let clock = ManualClock::new();
        let mut scheduler = AutoSaveScheduler::with_clock(clock);
        let tab_id = TabIdAllocator::new().allocate();

        scheduler.on_content_changed(&note_origin(), tab_id, 2);
        scheduler.clock.advance(AUTO_SAVE_IDLE_DELAY - Duration::from_millis(1));
        scheduler.on_ime_preedit(tab_id);
        scheduler.clock.advance(Duration::from_millis(1));
        assert_eq!(scheduler.take_due_saves()[0].content_revision, 2);

        scheduler.on_content_changed(&note_origin(), tab_id, 3);
        assert_eq!(
            scheduler.state(tab_id),
            Some(AutoSaveState::Scheduled {
                deadline: scheduler.clock.now() + AUTO_SAVE_IDLE_DELAY,
                content_revision: 3,
            })
        );
    }

    #[test]
    fn failed_save_can_retry_immediately_and_late_completion_cannot_clear_newer_state() {
        let clock = ManualClock::new();
        let mut scheduler = AutoSaveScheduler::with_clock(clock);
        let tab_id = TabIdAllocator::new().allocate();

        scheduler.request_immediate_save(&note_origin(), tab_id, 5);
        let failed_request =
            scheduler.take_due_saves().pop().expect("immediate save should be due");
        scheduler.on_save_failed(failed_request);
        assert!(scheduler.retry_failed_save(tab_id));
        let retry_request = scheduler.take_due_saves().pop().expect("retry should be due");
        scheduler.on_content_changed(&note_origin(), tab_id, 6);
        scheduler.on_save_completed(retry_request);

        assert!(matches!(
            scheduler.state(tab_id),
            Some(AutoSaveState::Scheduled { content_revision: 6, .. })
        ));
    }

    #[test]
    fn system_clock_is_available_for_the_production_scheduler() {
        let scheduler = AutoSaveScheduler::with_clock(SystemAutoSaveClock);
        assert!(scheduler.states.is_empty());
    }

    #[test]
    fn clearing_scheduler_cancels_all_pending_workspace_saves() {
        let clock = ManualClock::new();
        let mut scheduler = AutoSaveScheduler::with_clock(clock);
        let mut tab_ids = TabIdAllocator::new();
        let first_tab = tab_ids.allocate();
        let second_tab = tab_ids.allocate();
        scheduler.on_content_changed(&note_origin(), first_tab, 1);
        scheduler.on_content_changed(&note_origin(), second_tab, 1);

        scheduler.clear();

        assert_eq!(scheduler.state(first_tab), None);
        assert_eq!(scheduler.state(second_tab), None);
        assert_eq!(scheduler.next_deadline(), None);
    }
}
