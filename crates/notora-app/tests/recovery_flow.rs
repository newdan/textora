use std::cell::Cell;
use std::time::{Duration, Instant};

use appkit_core::{snapshot::SNAPSHOT_EXT, workspace::types::TabIdAllocator};
use notora_app::autosave::{AutoSaveClock, AutoSaveScheduler, AutoSaveState};
use notora_app::dirty_snapshot::{
    DirtySnapshotPlan, RecoverableDirtySnapshot, list_recoverable_snapshots, write_dirty_snapshot,
};
use notora_app::{WorkspaceCommand, WorkspaceController};
use notora_core::{
    BackupRetention, Catalog, CatalogOpenOutcome, DocumentOrigin, NoteId, WorkspaceId,
};

#[derive(Debug)]
struct ManualClock {
    now: Cell<Instant>,
}

impl ManualClock {
    fn new() -> Self {
        Self { now: Cell::new(Instant::now()) }
    }
}

impl AutoSaveClock for ManualClock {
    fn now(&self) -> Instant {
        self.now.get()
    }
}

#[test]
fn unavailable_workspace_does_not_activate_a_partial_session() {
    let directory = tempfile::tempdir().expect("recovery fixture directory should exist");
    let missing_root = directory.path().join("removed-workspace");
    let mut controller = WorkspaceController::default();
    let mut product = notora_app::product::NotoraProduct::new();

    assert!(
        controller
            .execute(WorkspaceCommand::OpenExisting { root: missing_root }, &mut product)
            .is_err()
    );
    assert_eq!(controller.active_workspace(), None);
}

#[test]
fn damaged_catalog_recovers_from_backup_without_losing_note_metadata() {
    let directory = tempfile::tempdir().expect("recovery fixture directory should exist");
    let catalog_path = directory.path().join("catalog.sqlite3");
    let catalog = Catalog::open(&catalog_path).expect("fixture catalog should open");
    let tag = catalog.create_tag("Release").expect("tag creation should succeed");
    let backups = directory.path().join("backups");
    notora_core::create_catalog_backup(
        &catalog,
        &backups,
        BackupRetention::keep_latest(1).expect("positive retention should be valid"),
    )
    .expect("catalog backup should complete");
    drop(catalog);
    std::fs::write(&catalog_path, "corrupt catalog").expect("fixture should damage the catalog");

    let outcome = Catalog::open_or_recover(&catalog_path, &backups)
        .expect("catalog should recover from its valid backup");
    let CatalogOpenOutcome::RecoveredFromBackup { catalog, .. } = outcome else {
        panic!("valid backup should be selected for corruption recovery");
    };
    assert_eq!(
        catalog.tags_with_active_note_counts().expect("tags should be readable"),
        vec![notora_core::TagWithActiveNoteCount {
            tag_id: tag.tag_id,
            display_name: "Release".to_owned(),
            active_note_count: 0,
        }]
    );
}

#[test]
fn failed_workspace_save_remains_retryable_without_losing_the_revision() {
    let clock = ManualClock::new();
    let mut scheduler = AutoSaveScheduler::with_clock_and_idle_delay(clock, Duration::ZERO);
    let tab_id = TabIdAllocator::new().allocate();
    let origin = DocumentOrigin::Note {
        workspace_id: WorkspaceId::generate(),
        note_id: NoteId::generate(),
        relative_path: "notes/permission-denied.md".into(),
    };

    scheduler.on_content_changed(&origin, tab_id, 42);
    let request =
        scheduler.take_due_saves().pop().expect("the edited workspace note should request a save");
    scheduler.on_save_failed(request);

    assert_eq!(scheduler.state(tab_id), Some(AutoSaveState::Failed { content_revision: 42 }));
    assert!(scheduler.retry_failed_save(tab_id));
    assert_eq!(
        scheduler
            .take_due_saves()
            .pop()
            .expect("the failed save should be retryable")
            .content_revision,
        42
    );
}

#[test]
fn abnormal_exit_snapshot_is_listed_for_explicit_recovery_on_next_start() {
    let directory = tempfile::tempdir().expect("recovery fixture directory should exist");
    let tab_id = TabIdAllocator::new().allocate();
    let plan = DirtySnapshotPlan {
        tab_id,
        filename: format!("abnormal-exit.{SNAPSHOT_EXT}"),
        baseline: None,
        current_lines: vec!["unsaved local change".to_owned()],
    };

    let snapshot_path =
        write_dirty_snapshot(directory.path(), &plan).expect("shutdown snapshot should persist");
    assert!(snapshot_path.is_file());

    let entries =
        list_recoverable_snapshots(directory.path()).expect("next launch should list snapshots");
    assert!(matches!(
        entries.as_slice(),
        [RecoverableDirtySnapshot::Ready { content_lines, .. }]
            if content_lines == &vec!["unsaved local change".to_owned()]
    ));
}
