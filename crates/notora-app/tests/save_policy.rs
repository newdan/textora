use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use appkit_core::workspace::types::TabIdAllocator;
use notora_app::autosave::{AUTO_SAVE_IDLE_DELAY, AutoSaveClock, AutoSaveScheduler, AutoSaveState};
use notora_core::{DocumentKind, DocumentOrigin, ExternalFileId, NoteId, WorkspaceId};

#[derive(Clone)]
struct ManualClock(Arc<Mutex<Instant>>);

impl ManualClock {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().expect("test clock lock should not be poisoned");
        *now += duration;
    }
}

impl AutoSaveClock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().expect("test clock lock should not be poisoned")
    }
}

fn note_origin(relative_path: &str) -> DocumentOrigin {
    DocumentOrigin::Note {
        workspace_id: WorkspaceId::generate(),
        note_id: NoteId::generate(),
        relative_path: relative_path.into(),
    }
}

#[test]
fn all_note_kinds_save_only_after_800ms_of_idle_time() {
    let clock = ManualClock::new();
    let mut scheduler = AutoSaveScheduler::with_clock(clock.clone());
    let mut tabs = TabIdAllocator::new();

    for (index, path) in ["note.txt", "note.md", "note.mmap.md"].into_iter().enumerate() {
        let tab_id = tabs.allocate();
        scheduler.on_content_changed(&note_origin(path), tab_id, index as u64 + 1);
    }
    clock.advance(AUTO_SAVE_IDLE_DELAY - Duration::from_millis(1));
    assert!(scheduler.take_due_saves().is_empty());

    clock.advance(Duration::from_millis(1));
    let saves = scheduler.take_due_saves();
    assert_eq!(saves.len(), 3);
}

#[test]
fn external_and_untitled_documents_remain_unsaved_across_idle_periods() {
    let clock = ManualClock::new();
    let mut scheduler = AutoSaveScheduler::with_clock(clock.clone());
    let mut tabs = TabIdAllocator::new();
    let external_tab = tabs.allocate();
    let untitled_tab = tabs.allocate();
    let external = DocumentOrigin::ExternalFile {
        external_file_id: ExternalFileId::generate(),
        canonical_path: "/tmp/outside.md".into(),
    };
    let untitled = DocumentOrigin::UntitledExternal {
        external_file_id: ExternalFileId::generate(),
        kind: DocumentKind::Markdown,
    };

    scheduler.on_content_changed(&external, external_tab, 1);
    scheduler.on_content_changed(&untitled, untitled_tab, 1);
    clock.advance(AUTO_SAVE_IDLE_DELAY * 3);

    assert!(scheduler.take_due_saves().is_empty());
    assert_eq!(scheduler.state(external_tab), None);
    assert_eq!(scheduler.state(untitled_tab), None);
}

#[test]
fn editing_while_a_save_is_in_flight_preserves_the_newer_dirty_revision() {
    let clock = ManualClock::new();
    let mut scheduler = AutoSaveScheduler::with_clock(clock.clone());
    let tab_id = TabIdAllocator::new().allocate();
    let origin = note_origin("note.md");

    scheduler.request_immediate_save(&origin, tab_id, 4);
    let previous_save =
        scheduler.take_due_saves().pop().expect("immediate save should become in-flight");
    scheduler.on_content_changed(&origin, tab_id, 5);
    scheduler.on_save_completed(previous_save);

    assert!(matches!(
        scheduler.state(tab_id),
        Some(AutoSaveState::Scheduled { content_revision: 5, .. })
    ));
}
