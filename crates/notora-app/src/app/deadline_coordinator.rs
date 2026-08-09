use std::time::Instant;

/// 各组件发布给顶层事件循环的下一次唤醒时间。
pub(super) struct DeadlineSnapshot {
    pub(super) autosave: Option<Instant>,
    pub(super) search: Option<Instant>,
    pub(super) persistence: Option<Instant>,
    pub(super) text_cursor_blink: Option<Instant>,
    pub(super) editor_cursor_blink: Option<Instant>,
}

pub(super) struct DeadlineCoordinator;

impl DeadlineCoordinator {
    pub(super) fn next_deadline(snapshot: DeadlineSnapshot) -> Option<Instant> {
        earliest_deadline([
            snapshot.autosave,
            snapshot.search,
            snapshot.persistence,
            snapshot.text_cursor_blink,
            snapshot.editor_cursor_blink,
        ])
    }
}

fn earliest_deadline<const DEADLINE_COUNT: usize>(
    deadlines: [Option<Instant>; DEADLINE_COUNT],
) -> Option<Instant> {
    deadlines.into_iter().flatten().min()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::earliest_deadline;

    #[test]
    fn earliest_deadline_ignores_inactive_sources() {
        let now = Instant::now();
        let first = now + Duration::from_millis(10);
        let second = now + Duration::from_millis(20);

        assert_eq!(earliest_deadline([None, Some(second), Some(first)]), Some(first));
        assert_eq!(earliest_deadline([None, None]), None);
    }
}
