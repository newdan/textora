//! 文本 reshape worker 的 generation、pending 和 debounce 会话。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::reshape_worker::{ReshapeRequest, ReshapeResult, ReshapeWorker};

const AHEAD_SUBMIT_DEBOUNCE: Duration = Duration::from_millis(16);
pub(crate) const RESHAPE_AHEAD_LINES: usize = 64;

pub(crate) struct ReshapeSession {
    shared_font_system: Option<Arc<Mutex<shaping::FontSystem>>>,
    worker: Option<ReshapeWorker>,
    generation: u64,
    pending_lines: HashSet<usize>,
    last_observed_anchor: Option<usize>,
    last_submitted_anchor: Option<usize>,
    last_submit: Option<Instant>,
    skip_next_submit: bool,
}

impl ReshapeSession {
    pub(crate) fn new() -> Self {
        Self {
            shared_font_system: None,
            worker: None,
            generation: 0,
            pending_lines: HashSet::new(),
            last_observed_anchor: None,
            last_submitted_anchor: None,
            last_submit: None,
            skip_next_submit: false,
        }
    }

    pub(crate) fn start_worker(
        &mut self,
        font_system: Arc<Mutex<shaping::FontSystem>>,
        font_size: f32,
        font_family: String,
    ) {
        self.shared_font_system = Some(Arc::clone(&font_system));
        if self.worker.is_none() {
            self.worker = Some(ReshapeWorker::spawn(font_system, font_size, font_family));
        }
    }

    pub(crate) fn set_shared_font_system(&mut self, font_system: Arc<Mutex<shaping::FontSystem>>) {
        self.shared_font_system = Some(font_system);
    }

    pub(crate) fn shared_font_system(&self) -> Option<Arc<Mutex<shaping::FontSystem>>> {
        self.shared_font_system.clone()
    }

    pub(crate) fn new_shaper(&self, font_size: f32, font_family: &str) -> Option<shaping::Shaper> {
        self.shared_font_system.as_ref().map(|font_system| {
            shaping::Shaper::from_shared_font_system(
                Arc::clone(font_system),
                font_size,
                font_family,
            )
        })
    }

    pub(crate) fn attach_worker(
        &mut self,
        worker: ReshapeWorker,
        shared_font_system: Option<Arc<Mutex<shaping::FontSystem>>>,
    ) {
        if self.worker.is_none() {
            self.worker = Some(worker);
            self.shared_font_system = shared_font_system;
        }
    }

    pub(crate) fn has_worker(&self) -> bool {
        self.worker.is_some()
    }

    pub(crate) fn invalidate(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.pending_lines.clear();
        self.last_observed_anchor = None;
        self.last_submitted_anchor = None;
        self.last_submit = None;
        if let Some(worker) = self.worker.as_ref() {
            worker.cancel_before(self.generation);
        }
        self.generation
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn accepts(&self, result: &ReshapeResult, active_document_index: usize) -> bool {
        result.generation == self.generation && result.dv_idx == active_document_index
    }

    pub(crate) fn should_submit_ahead(&mut self, anchor: usize, now: Instant) -> bool {
        if self
            .last_observed_anchor
            .is_some_and(|previous| previous.abs_diff(anchor) > RESHAPE_AHEAD_LINES)
        {
            self.last_observed_anchor = Some(anchor);
            self.last_submitted_anchor = None;
            self.last_submit = None;
            return false;
        }

        if self.last_submitted_anchor == Some(anchor) {
            return false;
        }

        self.last_submit.is_none_or(|last| now.duration_since(last) >= AHEAD_SUBMIT_DEBOUNCE)
    }

    pub(crate) fn mark_submitted(&mut self, anchor: usize) {
        self.last_observed_anchor = Some(anchor);
        self.last_submitted_anchor = Some(anchor);
        self.last_submit = Some(Instant::now());
    }

    pub(crate) fn mark_pending(&mut self, line: usize) -> bool {
        self.pending_lines.insert(line)
    }

    pub(crate) fn clear_pending(&mut self, line: usize) {
        self.pending_lines.remove(&line);
    }

    pub(crate) fn is_pending(&self, line: usize) -> bool {
        self.pending_lines.contains(&line)
    }

    pub(crate) fn skip_next_submit(&mut self) {
        self.skip_next_submit = true;
    }

    pub(crate) fn take_skip_next_submit(&mut self) -> bool {
        std::mem::take(&mut self.skip_next_submit)
    }

    pub(crate) fn submit(&self, mut request: ReshapeRequest) -> bool {
        let Some(worker) = self.worker.as_ref() else {
            return false;
        };
        request.generation = self.generation;
        worker.submit(request).is_ok()
    }

    pub(crate) fn drain_completed(&self, limit: usize) -> Vec<ReshapeResult> {
        self.worker.as_ref().map_or_else(Vec::new, |worker| worker.drain_completed(limit))
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
    }
}

impl Default for ReshapeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ReshapeSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ReshapeRequest {
        ReshapeRequest {
            generation: 0,
            doc_line: 0,
            byte_offset: 0,
            byte_length: 0,
            line_bytes: Arc::from([]),
            viewport_width: 100.0,
            font_size: 12.0,
            max_line_bytes: 0,
            dv_idx: 0,
        }
    }

    #[test]
    fn invalidation_advances_generation_and_clears_pending_lines() {
        let mut session = ReshapeSession::new();
        assert!(session.mark_pending(3));
        let generation = session.invalidate();
        assert_eq!(generation, 1);
        assert!(session.mark_pending(3));
        assert_eq!(session.generation(), generation);
    }

    #[test]
    fn stale_results_are_rejected_after_invalidation() {
        let mut session = ReshapeSession::new();
        let result = ReshapeResult {
            generation: session.generation(),
            doc_line: 0,
            entry: crate::snap_tree::DisplayLineEntry::placeholder(0, 0, 1, 1),
            dv_idx: 0,
        };

        assert!(session.accepts(&result, 0));
        session.invalidate();
        assert!(!session.accepts(&result, 0));
    }

    #[test]
    fn submit_without_worker_is_safe_and_does_not_claim_success() {
        let session = ReshapeSession::new();
        assert!(!session.submit(request()));
        assert!(session.drain_completed(4).is_empty());
    }

    #[test]
    fn anchor_submission_is_debounced_until_the_anchor_changes() {
        let mut session = ReshapeSession::new();
        let now = Instant::now();
        assert!(session.should_submit_ahead(5, now));
        session.mark_submitted(5);
        assert!(!session.should_submit_ahead(5, now + Duration::from_secs(1)));
        assert!(session.should_submit_ahead(6, now + Duration::from_secs(1)));
    }

    #[test]
    fn a_far_anchor_jump_is_skipped_once_before_resubmitting() {
        let mut session = ReshapeSession::new();
        let now = Instant::now();
        assert!(session.should_submit_ahead(5, now));
        session.mark_submitted(5);

        assert!(!session.should_submit_ahead(5 + RESHAPE_AHEAD_LINES + 1, now));
        assert!(session.should_submit_ahead(5 + RESHAPE_AHEAD_LINES + 1, now));
    }
}
