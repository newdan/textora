use std::time::{Duration, Instant};

use notora_core::WorkspaceId;

pub const SEARCH_DEBOUNCE_DELAY: Duration = Duration::from_millis(120);

/// 能够唯一识别一个工作区搜索结果的 generation。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchGeneration(u64);

/// 后台搜索请求必须携带的工作区与搜索 generation。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRequest {
    pub workspace_id: WorkspaceId,
    pub workspace_generation: u64,
    pub search_generation: SearchGeneration,
    pub query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SearchWorkspace {
    workspace_id: WorkspaceId,
    workspace_generation: u64,
}

#[derive(Clone, Debug)]
struct PendingSearch {
    workspace: SearchWorkspace,
    search_generation: SearchGeneration,
    query: String,
    deadline: Instant,
}

/// 将全局搜索输入去抖，并拒绝旧工作区或旧查询的异步完成。
#[derive(Debug, Default)]
pub struct SearchController {
    active_workspace: Option<SearchWorkspace>,
    pending_search: Option<PendingSearch>,
    active_search_generation: SearchGeneration,
    next_search_generation: u64,
}

impl SearchController {
    pub fn set_active_workspace(&mut self, workspace_id: WorkspaceId, workspace_generation: u64) {
        self.active_workspace = Some(SearchWorkspace { workspace_id, workspace_generation });
        self.invalidate_pending_search();
    }

    pub fn clear_active_workspace(&mut self) {
        self.active_workspace = None;
        self.invalidate_pending_search();
    }

    /// IME preedit 绝不进入此入口；仅由已经提交的文本更新调用。
    /// 返回 false 表示尚未打开工作区，调用方可只更新本地导航状态而不等待后台查询。
    pub fn schedule_committed_query(&mut self, query: String, now: Instant) -> bool {
        let Some(workspace) = self.active_workspace else {
            return false;
        };
        let search_generation = self.advance_search_generation();
        let deadline = if query.is_empty() { now } else { now + SEARCH_DEBOUNCE_DELAY };
        self.pending_search = Some(PendingSearch { workspace, search_generation, query, deadline });
        true
    }

    pub fn take_due_request(&mut self, now: Instant) -> Option<SearchRequest> {
        let pending_search = self.pending_search.take()?;
        if pending_search.deadline > now {
            self.pending_search = Some(pending_search);
            return None;
        }
        Some(SearchRequest {
            workspace_id: pending_search.workspace.workspace_id,
            workspace_generation: pending_search.workspace.workspace_generation,
            search_generation: pending_search.search_generation,
            query: pending_search.query,
        })
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending_search.as_ref().map(|pending_search| pending_search.deadline)
    }

    pub fn accepts_completion(&self, request: &SearchRequest) -> bool {
        self.active_workspace
            == Some(SearchWorkspace {
                workspace_id: request.workspace_id,
                workspace_generation: request.workspace_generation,
            })
            && self.active_search_generation == request.search_generation
    }

    fn invalidate_pending_search(&mut self) {
        self.pending_search = None;
        self.advance_search_generation();
    }

    fn advance_search_generation(&mut self) -> SearchGeneration {
        self.next_search_generation = self.next_search_generation.wrapping_add(1);
        self.active_search_generation = SearchGeneration(self.next_search_generation);
        self.active_search_generation
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use notora_core::WorkspaceId;

    use super::{SEARCH_DEBOUNCE_DELAY, SearchController};

    #[test]
    fn rapid_commits_only_dispatch_the_latest_query_after_the_debounce_delay() {
        let mut controller = SearchController::default();
        controller.set_active_workspace(WorkspaceId::generate(), 1);
        let start = Instant::now();
        controller.schedule_committed_query("road".to_owned(), start);
        controller
            .schedule_committed_query("roadmap".to_owned(), start + Duration::from_millis(10));

        assert!(controller.take_due_request(start + SEARCH_DEBOUNCE_DELAY).is_none());
        let request = controller
            .take_due_request(start + Duration::from_millis(130))
            .expect("latest query should dispatch after its debounce delay");
        assert_eq!(request.query, "roadmap");
    }

    #[test]
    fn empty_committed_query_dispatches_immediately_to_restore_the_previous_scope() {
        let mut controller = SearchController::default();
        controller.set_active_workspace(WorkspaceId::generate(), 1);
        let now = Instant::now();
        controller.schedule_committed_query(String::new(), now);

        assert_eq!(
            controller
                .take_due_request(now)
                .expect("empty query should dispatch immediately")
                .query,
            ""
        );
    }

    #[test]
    fn workspace_switch_and_newer_query_discard_out_of_order_completions() {
        let mut controller = SearchController::default();
        let first_workspace_id = WorkspaceId::generate();
        let second_workspace_id = WorkspaceId::generate();
        let now = Instant::now();
        controller.set_active_workspace(first_workspace_id, 1);
        controller.schedule_committed_query("first".to_owned(), now);
        let first_request = controller
            .take_due_request(now + SEARCH_DEBOUNCE_DELAY)
            .expect("first request should dispatch");

        controller.set_active_workspace(second_workspace_id, 2);
        controller.schedule_committed_query("second".to_owned(), now);
        let second_request = controller
            .take_due_request(now + SEARCH_DEBOUNCE_DELAY)
            .expect("second request should dispatch");

        assert!(!controller.accepts_completion(&first_request));
        assert!(controller.accepts_completion(&second_request));
    }

    #[test]
    fn controller_without_a_committed_query_has_no_ime_preedit_side_effect() {
        let mut controller = SearchController::default();
        controller.set_active_workspace(WorkspaceId::generate(), 1);

        assert!(controller.take_due_request(Instant::now()).is_none());
    }
}
