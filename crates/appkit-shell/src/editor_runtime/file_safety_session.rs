//! 文件安全 worker 的 runtime 会话和稳定 tab 关联。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use appkit_core::file_safety::{
    DiskRevision, FileSafetyCommand, FileSafetyError, FileSafetyOutcome, FileSafetyResult,
    FileSafetyWorker,
};
use appkit_core::workspace::types::TabId;

const FILE_SAFETY_CHECK_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct FileSafetyCandidate {
    pub tab_id: TabId,
    pub path: PathBuf,
    pub dirty: bool,
    pub content_revision: u64,
    pub current_content: String,
    pub baseline: Option<DiskRevision>,
}

#[derive(Debug)]
pub struct FileSafetyObservation {
    pub tab_id: TabId,
    pub path: PathBuf,
    pub dirty: bool,
    pub content_revision: u64,
    pub outcome: Result<FileSafetyOutcome, FileSafetyError>,
}

#[derive(Debug, Clone, Copy)]
struct PendingObservation {
    tab_id: TabId,
    content_revision: u64,
    dirty: bool,
}

/// 统一管理文件安全的 worker、request ID 和 tab/path 关联。
pub(crate) struct FileSafetySession {
    worker: Option<FileSafetyWorker>,
    tracked_paths: HashMap<TabId, PathBuf>,
    path_to_tab: HashMap<PathBuf, TabId>,
    pending_requests: HashMap<u64, PendingObservation>,
    next_request_id: u64,
    next_check: Instant,
}

impl FileSafetySession {
    pub(crate) fn new() -> Self {
        Self {
            worker: None,
            tracked_paths: HashMap::new(),
            path_to_tab: HashMap::new(),
            pending_requests: HashMap::new(),
            next_request_id: 1,
            next_check: Instant::now(),
        }
    }

    pub(crate) fn start_worker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        if self.worker.is_none() {
            self.worker = Some(FileSafetyWorker::spawn(wake));
        }
    }

    pub(crate) fn worker_started(&self) -> bool {
        self.worker.is_some()
    }

    pub(crate) fn next_check(&self) -> Instant {
        self.next_check
    }

    pub(crate) fn should_check(&self, now: Instant) -> bool {
        now >= self.next_check
    }

    pub(crate) fn schedule_next_check(&mut self, now: Instant) {
        self.next_check = now + FILE_SAFETY_CHECK_INTERVAL;
    }

    pub(crate) fn request_check_now(&mut self, now: Instant) {
        self.next_check = now;
    }

    pub(crate) fn submit_candidates(
        &mut self,
        candidates: impl IntoIterator<Item = FileSafetyCandidate>,
        local_device_short_id: &str,
    ) -> usize {
        if self.worker.is_none() {
            return 0;
        }
        let mut submitted_count = 0;
        for candidate in candidates {
            self.replace_path_mapping(candidate.tab_id, candidate.path.clone());
            let needs_initial_reconciliation = candidate.dirty && candidate.baseline.is_some();
            let is_tracked = self.tracked_paths.get(&candidate.tab_id) == Some(&candidate.path);

            if !is_tracked {
                let initial_request_id = if needs_initial_reconciliation {
                    Some(self.allocate_request_id(candidate.tab_id, &candidate))
                } else {
                    None
                };
                let command = initial_request_id.map_or_else(
                    || FileSafetyCommand::Track { path: candidate.path.clone() },
                    |request_id| FileSafetyCommand::ReconcileDirtySnapshot {
                        request_id,
                        path: candidate.path.clone(),
                        baseline: candidate
                            .baseline
                            .clone()
                            .expect("dirty candidate must provide a disk baseline"),
                        content_revision: candidate.content_revision,
                        current_content: candidate.current_content.clone(),
                        local_device_short_id: local_device_short_id.to_owned(),
                    },
                );
                let submitted = self
                    .worker
                    .as_ref()
                    .expect("worker presence was checked before submitting candidates")
                    .submit(command)
                    .is_ok();
                if submitted {
                    self.tracked_paths.insert(candidate.tab_id, candidate.path.clone());
                    if needs_initial_reconciliation {
                        submitted_count += 1;
                        continue;
                    }
                } else if let Some(request_id) = initial_request_id {
                    self.pending_requests.remove(&request_id);
                }
            }

            if self.pending_requests.values().any(|pending| {
                pending.tab_id == candidate.tab_id
                    && pending.content_revision == candidate.content_revision
                    && pending.dirty == candidate.dirty
            }) {
                continue;
            }
            let request_id = self.allocate_request_id(candidate.tab_id, &candidate);
            let command = FileSafetyCommand::Observe {
                request_id,
                path: candidate.path,
                dirty: candidate.dirty,
                content_revision: candidate.content_revision,
                current_content: candidate.current_content,
                local_device_short_id: local_device_short_id.to_owned(),
            };
            if self
                .worker
                .as_ref()
                .expect("worker presence was checked before submitting candidates")
                .submit(command)
                .is_ok()
            {
                submitted_count += 1;
            } else {
                self.pending_requests.remove(&request_id);
            }
        }
        submitted_count
    }

    pub(crate) fn forget_tab(&mut self, tab_id: TabId) {
        if let Some(path) = self.tracked_paths.remove(&tab_id) {
            self.path_to_tab.remove(&path);
        }
        self.pending_requests.retain(|_, pending| pending.tab_id != tab_id);
    }

    pub(crate) fn drain_observations(&mut self) -> Vec<FileSafetyObservation> {
        let mut observations = Vec::new();
        let Some(worker) = self.worker.as_ref() else {
            return observations;
        };
        while let Some(result) = worker.try_recv() {
            match result {
                FileSafetyResult::Tracked { path, outcome } => {
                    if outcome.is_err()
                        && let Some(tab_id) = self.path_to_tab.remove(&path)
                    {
                        self.tracked_paths.remove(&tab_id);
                    }
                }
                FileSafetyResult::Observed {
                    request_id,
                    path,
                    dirty,
                    content_revision,
                    outcome,
                } => {
                    let Some(pending) = self.pending_requests.remove(&request_id) else {
                        continue;
                    };
                    if pending.tab_id
                        != self.path_to_tab.get(&path).copied().unwrap_or(pending.tab_id)
                        || pending.dirty != dirty
                        || pending.content_revision != content_revision
                    {
                        continue;
                    }
                    observations.push(FileSafetyObservation {
                        tab_id: pending.tab_id,
                        path,
                        dirty,
                        content_revision,
                        outcome,
                    });
                }
            }
        }
        observations
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
        self.tracked_paths.clear();
        self.path_to_tab.clear();
        self.pending_requests.clear();
    }

    fn replace_path_mapping(&mut self, tab_id: TabId, path: PathBuf) {
        if let Some(previous_path) = self.tracked_paths.get(&tab_id)
            && previous_path != &path
        {
            self.path_to_tab.remove(previous_path);
        }
        if let Some(previous_tab_id) = self.path_to_tab.insert(path.clone(), tab_id)
            && previous_tab_id != tab_id
        {
            self.tracked_paths.remove(&previous_tab_id);
        }
    }

    fn allocate_request_id(&mut self, tab_id: TabId, candidate: &FileSafetyCandidate) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.pending_requests.insert(
            request_id,
            PendingObservation {
                tab_id,
                content_revision: candidate.content_revision,
                dirty: candidate.dirty,
            },
        );
        request_id
    }
}

impl Default for FileSafetySession {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FileSafetySession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::workspace::types::TabIdAllocator;
    use std::path::Path;

    fn candidate(tab_id: TabId, path: &Path) -> FileSafetyCandidate {
        FileSafetyCandidate {
            tab_id,
            path: path.to_owned(),
            dirty: false,
            content_revision: 0,
            current_content: String::new(),
            baseline: None,
        }
    }

    #[test]
    fn checks_have_a_semantic_deadline() {
        let mut session = FileSafetySession::new();
        let now = Instant::now();
        assert!(session.should_check(now));
        session.schedule_next_check(now);
        assert!(!session.should_check(now));
        assert!(session.should_check(now + FILE_SAFETY_CHECK_INTERVAL));
    }

    #[test]
    fn closed_tabs_drop_pending_observations_before_results_are_applied() {
        let mut allocator = TabIdAllocator::new();
        let tab_id = allocator.allocate();
        let mut session = FileSafetySession::new();
        let path = PathBuf::from("note.txt");
        session.replace_path_mapping(tab_id, path.clone());
        let request = FileSafetyCandidate { content_revision: 1, ..candidate(tab_id, &path) };
        let request_id = session.allocate_request_id(tab_id, &request);
        assert!(session.pending_requests.contains_key(&request_id));
        session.forget_tab(tab_id);
        assert!(!session.pending_requests.contains_key(&request_id));
        assert!(session.tracked_paths.is_empty());
    }

    #[test]
    fn path_mapping_uses_stable_tab_ids_when_a_path_moves() {
        let mut allocator = TabIdAllocator::new();
        let first = allocator.allocate();
        let second = allocator.allocate();
        let mut session = FileSafetySession::new();
        session.replace_path_mapping(first, PathBuf::from("note.txt"));
        session.replace_path_mapping(second, PathBuf::from("note.txt"));
        assert!(!session.tracked_paths.contains_key(&first));
        assert_eq!(session.path_to_tab.get(std::path::Path::new("note.txt")), Some(&second));
    }
}
