use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use appkit_shell::ProductHost;

use super::{
    ActiveWorkspace, CATALOG_FILE_NAME, Catalog, Workspace, WorkspaceCommand,
    WorkspaceCommandResult, WorkspaceController, WorkspaceControllerError,
    WorkspaceFileMonitorEvent, default_migration_backup_retention, run_indexer, scan_workspace,
};
use crate::action::{CardQuery, DocumentLoadRequest};
use crate::index_worker::IndexWorkerCommand;
use crate::product::{
    NotoraProduct, NotoraProductEvent, WorkspaceBootstrapCompletion, WorkspaceCompletion,
    WorkspaceEventScope, WorkspaceEventSender,
};
use notora_core::DocumentIdentity;

fn active_completion<'a>(
    event: &'a NotoraProductEvent,
    active_workspace: &ActiveWorkspace,
) -> Option<&'a WorkspaceCompletion> {
    let NotoraProductEvent::Workspace(event) = event else {
        return None;
    };
    if event.scope.workspace_id != active_workspace.descriptor.workspace_id
        || event.scope.generation != active_workspace.generation
    {
        return None;
    }
    Some(&event.completion)
}

fn wait_for_bootstrap_completion(product: &mut NotoraProduct) -> WorkspaceBootstrapCompletion {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = ProductHost::drain_product_events(product);
        if let Some(completion) = product.take_events().into_iter().find_map(|event| {
            let NotoraProductEvent::WorkspaceBootstrap(completion) = event else {
                return None;
            };
            Some(completion)
        }) {
            return completion;
        }
        assert!(Instant::now() < deadline, "workspace bootstrap should complete promptly");
        thread::yield_now();
    }
}

#[test]
fn cancelled_selection_keeps_the_current_workspace_unchanged() {
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();

    assert_eq!(
        controller
            .execute(WorkspaceCommand::SelectionCancelled, &mut product)
            .expect("cancelling a selection should be harmless"),
        WorkspaceCommandResult::Unchanged
    );
    assert_eq!(controller.active_workspace(), None);
}

#[test]
fn background_workspace_preparation_installs_only_after_completion() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();

    let generation = controller
        .begin_open_existing(directory.path().to_path_buf(), &product)
        .expect("background workspace preparation should start");

    assert_eq!(controller.active_workspace(), None);
    let completion = wait_for_bootstrap_completion(&mut product);
    assert_eq!(completion.generation, generation);
    let result = controller
        .complete_open_existing(completion.generation, &mut product)
        .expect("prepared workspace should install")
        .expect("matching completion should produce a command result");
    let WorkspaceCommandResult::Opened(active_workspace) = result else {
        panic!("bootstrap completion should open the prepared workspace");
    };
    assert_eq!(active_workspace.generation, generation);
    assert_eq!(controller.active_workspace(), Some(active_workspace));
}

#[test]
fn creates_directory_initializes_workspace_and_advances_generation_on_close() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let root = directory.path().join("new-workspace");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();

    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(WorkspaceCommand::Create { root: root.clone() }, &mut product)
        .expect("new workspace should open")
    else {
        panic!("create command should open a workspace");
    };
    assert_eq!(active_workspace.generation, 1);
    assert!(root.join(".notora/workspace.toml").is_file());

    assert_eq!(
        controller.execute(WorkspaceCommand::Close, &mut product).expect("close should be safe"),
        WorkspaceCommandResult::Closed { generation: 2 }
    );
    assert_eq!(controller.active_workspace(), None);
}

#[test]
fn create_rejects_an_existing_target_without_opening_it() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let existing_root = directory.path().join("existing");
    fs::create_dir(&existing_root).expect("existing target should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();

    assert!(matches!(
        controller.execute(
            WorkspaceCommand::Create { root: existing_root.clone() },
            &mut product,
        ),
        Err(WorkspaceControllerError::CreateDirectory { path, source })
            if path == existing_root && source.kind() == std::io::ErrorKind::AlreadyExists
    ));
    assert_eq!(controller.active_workspace(), None);
    assert!(!existing_root.join(".notora").exists());
}

#[test]
fn failed_replacement_does_not_advance_generation_or_close_the_active_session() {
    let first_directory =
        tempfile::tempdir().expect("first workspace test directory should be created");
    let corrupt_directory =
        tempfile::tempdir().expect("corrupt workspace test directory should be created");
    fs::create_dir(corrupt_directory.path().join(".notora"))
        .expect("metadata directory should be created");
    fs::write(corrupt_directory.path().join(".notora/workspace.toml"), "not valid toml")
        .expect("corrupt manifest should be written");
    let second_directory =
        tempfile::tempdir().expect("second workspace test directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(first_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: first_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("first workspace should open")
    else {
        panic!("first workspace should become active");
    };

    assert!(
        controller
            .execute(
                WorkspaceCommand::OpenExisting { root: corrupt_directory.path().to_path_buf() },
                &mut product,
            )
            .is_err()
    );
    assert_eq!(controller.active_workspace(), Some(first_workspace.clone()));

    let WorkspaceCommandResult::Opened(second_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: second_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("second valid workspace should open")
    else {
        panic!("second workspace should become active");
    };
    assert_eq!(second_workspace.generation, first_workspace.generation + 1);
}

#[test]
fn indexer_catalog_startup_failure_does_not_replace_the_active_session() {
    let active_directory = tempfile::tempdir().expect("active workspace directory should exist");
    let target_directory = tempfile::tempdir().expect("target workspace directory should exist");
    let target_workspace = notora_core::Workspace::open_or_initialize(target_directory.path())
        .expect("target workspace metadata should initialize");
    let target_catalog_path = target_workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let target_catalog = rusqlite::Connection::open(&target_catalog_path)
        .expect("target catalog fixture should open");
    target_catalog
        .pragma_update(None, "user_version", 9_999_i64)
        .expect("future catalog version should be written");
    drop(target_catalog);
    drop(target_workspace);
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: active_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("active workspace should open")
    else {
        panic!("active workspace should become active");
    };

    assert!(
        controller
            .execute(
                WorkspaceCommand::OpenExisting { root: target_directory.path().to_path_buf() },
                &mut product,
            )
            .is_err()
    );
    assert_eq!(controller.active_workspace(), Some(active_workspace));
}

#[test]
fn corrupt_manifest_does_not_replace_the_active_workspace() {
    let active_directory = tempfile::tempdir().expect("active workspace directory should exist");
    let corrupt_directory = tempfile::tempdir().expect("corrupt workspace directory should exist");
    fs::create_dir(corrupt_directory.path().join(".notora"))
        .expect("metadata directory fixture should be created");
    fs::write(corrupt_directory.path().join(".notora/workspace.toml"), "not toml")
        .expect("corrupt manifest fixture should be written");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: active_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("active workspace should open")
    else {
        panic!("open command should activate the workspace");
    };

    assert!(matches!(
        controller.execute(
            WorkspaceCommand::OpenExisting { root: corrupt_directory.path().to_path_buf() },
            &mut product,
        ),
        Err(WorkspaceControllerError::Workspace(_))
    ));
    assert_eq!(controller.active_workspace(), Some(active_workspace));
}

#[test]
fn product_discards_late_scan_results_after_a_workspace_switch() {
    let first_directory = tempfile::tempdir().expect("first workspace directory should exist");
    let second_directory = tempfile::tempdir().expect("second workspace directory should exist");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(first_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: first_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("first workspace should open")
    else {
        panic!("first workspace should activate");
    };
    let sender = product.event_sender();
    let WorkspaceCommandResult::Opened(second_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: second_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("second workspace should open")
    else {
        panic!("second workspace should activate");
    };
    WorkspaceEventSender::new(
        sender.clone(),
        WorkspaceEventScope {
            workspace_id: first_workspace.descriptor.workspace_id,
            generation: first_workspace.generation,
        },
    )
    .send(WorkspaceCompletion::WorkspaceChanged {
        changed_paths: vec!["late.md".into()],
        note_relocations: vec![],
    })
    .expect("product receiver should stay available");
    WorkspaceEventSender::new(
        sender,
        WorkspaceEventScope {
            workspace_id: second_workspace.descriptor.workspace_id,
            generation: second_workspace.generation,
        },
    )
    .send(WorkspaceCompletion::WorkspaceChanged {
        changed_paths: vec!["current.md".into()],
        note_relocations: vec![],
    })
    .expect("product receiver should stay available");

    let _ = product.drain_product_events();
    let events = product.take_events();
    assert!(events.iter().any(|event| matches!(
        event,
        NotoraProductEvent::Workspace(event)
            if event.scope.workspace_id == second_workspace.descriptor.workspace_id
                && event.scope.generation == second_workspace.generation
                && matches!(event.completion, WorkspaceCompletion::WorkspaceChanged { .. })
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        NotoraProductEvent::Workspace(event)
            if event.scope.workspace_id == first_workspace.descriptor.workspace_id
                && event.scope.generation == first_workspace.generation
                && matches!(event.completion, WorkspaceCompletion::WorkspaceChanged { .. })
    )));
}

#[test]
fn indexer_shutdown_does_not_wait_for_a_blocked_monitor() {
    let directory = tempfile::tempdir().expect("workspace fixture should exist");
    let workspace = notora_core::Workspace::open_or_initialize(directory.path())
        .expect("workspace fixture should initialize");
    let descriptor = workspace.descriptor();
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let (file_sender, monitor_events) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();
    let product = NotoraProduct::new();
    let event_sender = WorkspaceEventSender::new(
        product.event_sender(),
        WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
    );
    let (stopped_sender, stopped_receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_indexer(
            workspace,
            catalog_path,
            None,
            default_migration_backup_retention(),
            monitor_events,
            command_receiver,
            event_sender,
        );
        let _ = stopped_sender.send(());
    });
    drop(command_sender);
    let stopped = stopped_receiver.recv_timeout(Duration::from_secs(2));
    drop(file_sender);
    worker.join().expect("indexer should stop after fixture release");
    assert!(
        stopped.is_ok(),
        "indexer shutdown must follow command ownership, not monitor completion"
    );
}

#[test]
fn watcher_disconnection_reports_failure_and_keeps_commands_available() {
    assert_monitor_failure_keeps_commands_available(None);
}

#[test]
fn watcher_startup_failure_reports_failure_and_keeps_commands_available() {
    assert_monitor_failure_keeps_commands_available(Some(WorkspaceFileMonitorEvent::Failed(
        notora_core::WorkspaceFileMonitorError::WorkerDisconnected,
    )));
}

fn assert_monitor_failure_keeps_commands_available(failure: Option<WorkspaceFileMonitorEvent>) {
    let directory = tempfile::tempdir().expect("workspace fixture should exist");
    let workspace =
        Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
    let descriptor = workspace.descriptor();
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let (file_sender, monitor_events) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();
    let mut product = NotoraProduct::new();
    product.set_active_workspace(descriptor.workspace_id, 1);
    let event_sender = WorkspaceEventSender::new(
        product.event_sender(),
        WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
    );
    let worker = thread::spawn(move || {
        run_indexer(
            workspace,
            catalog_path,
            None,
            default_migration_backup_retention(),
            monitor_events,
            command_receiver,
            event_sender,
        )
    });
    if let Some(failure) = failure {
        file_sender.send(failure).expect("backend failure should reach indexer");
    }
    drop(file_sender);
    wait_for_workspace_completion(&mut product, |completion| {
        matches!(completion,
            WorkspaceCompletion::WorkspaceIndexFailed { message } if message.contains("文件监视器"))
    });
    command_sender
        .send(IndexWorkerCommand::QueryNavigationTree)
        .expect("monitor failure must not terminate command handling");
    wait_for_workspace_completion(&mut product, |completion| {
        matches!(completion, WorkspaceCompletion::NavigationTreeLoaded { .. })
    });
    drop(command_sender);
    worker.join().expect("indexer should stop after command ownership ends");
}

#[test]
fn monitor_ready_rescans_files_created_before_registration() {
    let directory = tempfile::tempdir().expect("workspace fixture should exist");
    let workspace =
        Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
    let descriptor = workspace.descriptor();
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let completed_catalog_path = catalog_path.clone();
    let (event_sender, monitor_events) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();
    let mut product = NotoraProduct::new();
    product.set_active_workspace(descriptor.workspace_id, 1);
    let product_sender = WorkspaceEventSender::new(
        product.event_sender(),
        WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
    );
    let worker = thread::spawn(move || {
        run_indexer(
            workspace,
            catalog_path,
            None,
            default_migration_backup_retention(),
            monitor_events,
            command_receiver,
            product_sender,
        )
    });
    wait_for_workspace_completion(&mut product, |completion| {
        matches!(completion, WorkspaceCompletion::WorkspaceScanCompleted { .. })
    });
    let late_note_relative_path = PathBuf::from("late.md");
    fs::write(
        directory.path().join(&late_note_relative_path),
        "# Created while registration was blocked",
    )
    .expect("registration-gap fixture should be written");
    event_sender
        .send(WorkspaceFileMonitorEvent::Ready)
        .expect("registration completion should publish");
    wait_for_workspace_completion(&mut product, |completion| {
        matches!(completion, WorkspaceCompletion::NavigationTreeLoaded { .. })
    });
    drop(command_sender);
    drop(event_sender);
    worker.join().expect("indexer should stop after command ownership ends");
    let catalog = Catalog::open(&completed_catalog_path).expect("catalog should reopen");
    assert!(
        catalog
            .active_notes()
            .expect("indexed notes should query")
            .iter()
            .any(|note| note.relative_path == late_note_relative_path)
    );
}

fn wait_for_workspace_completion(
    product: &mut NotoraProduct,
    predicate: impl Fn(&WorkspaceCompletion) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        if product.take_events().iter().any(|event| {
            matches!(event,
                NotoraProductEvent::Workspace(event) if predicate(&event.completion))
        }) {
            return;
        }
        assert!(Instant::now() < deadline, "workspace completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn indexer_recovery_after_preflight_notifies_the_product_about_restored_metadata() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let backup_directory = tempfile::tempdir().expect("backup test directory should be created");
    let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
        .expect("workspace metadata should be initialized");
    let descriptor = workspace.descriptor();
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let catalog = Catalog::open(&catalog_path).expect("catalog should initialize");
    catalog.create_tag("Retained").expect("fixture metadata should persist");
    let workspace_backups = backup_directory.path().join(descriptor.workspace_id.to_string());
    notora_core::create_catalog_backup(
        &catalog,
        &workspace_backups,
        notora_core::BackupRetention::keep_latest(1)
            .expect("positive backup retention should be valid"),
    )
    .expect("fixture backup should persist");
    drop(catalog);
    fs::write(&catalog_path, "damaged after the startup preflight")
        .expect("fixture catalog should be damaged");
    let (monitor_event_sender, monitor_events) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();
    let mut product = NotoraProduct::new();
    product.set_active_workspace(descriptor.workspace_id, 1);
    drop(monitor_event_sender);
    drop(command_sender);

    run_indexer(
        workspace,
        catalog_path,
        Some(workspace_backups),
        default_migration_backup_retention(),
        monitor_events,
        command_receiver,
        WorkspaceEventSender::new(
            product.event_sender(),
            WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
        ),
    );

    let _ = product.drain_product_events();
    assert!(product.take_events().iter().any(|event| {
        matches!(
            event,
            NotoraProductEvent::Workspace(event)
                if event.scope.workspace_id == descriptor.workspace_id
                    && event.scope.generation == 1
                    && matches!(
                        &event.completion,
                        WorkspaceCompletion::CatalogRecoveryNotified { message }
                            if message.contains("元数据已从")
                    )
        )
    }));
}

#[test]
fn watcher_batch_schedules_a_second_presence_check_for_a_deleted_note() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let note_path = directory.path().join("removed.md");
    fs::write(&note_path, "# Removed").expect("fixture note should be written");
    let workspace = notora_core::Workspace::open_or_initialize(directory.path())
        .expect("workspace metadata should be initialized");
    let descriptor = workspace.descriptor();
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let (monitor_event_sender, monitor_events) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();
    let mut product = NotoraProduct::new();
    product.set_active_workspace(descriptor.workspace_id, 1);
    let event_sender = product.event_sender();
    let worker = thread::spawn({
        let catalog_path = catalog_path.clone();
        move || {
            run_indexer(
                workspace,
                catalog_path,
                None,
                default_migration_backup_retention(),
                monitor_events,
                command_receiver,
                WorkspaceEventSender::new(
                    event_sender,
                    WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
                ),
            )
        }
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut completed_scans = 0;
    while completed_scans < 1 {
        let _ = product.drain_product_events();
        completed_scans += product
                .take_events()
                .into_iter()
                .filter(|event| matches!(
                    event,
                    NotoraProductEvent::Workspace(event)
                        if matches!(event.completion, WorkspaceCompletion::WorkspaceScanCompleted { .. })
                ))
                .count();
        assert!(Instant::now() < deadline, "initial scan should complete promptly");
        thread::sleep(Duration::from_millis(10));
    }
    fs::remove_file(&note_path).expect("fixture note should be removed");
    monitor_event_sender
        .send(WorkspaceFileMonitorEvent::Changes(notora_core::WorkspaceFileBatch {
            relative_paths: vec!["removed.md".into()],
            changes: vec![notora_core::WorkspaceFileChange::Removed("removed.md".into())],
        }))
        .expect("watcher batch should reach the indexer");

    while completed_scans < 3 {
        let _ = product.drain_product_events();
        completed_scans += product
                .take_events()
                .into_iter()
                .filter(|event| matches!(
                    event,
                    NotoraProductEvent::Workspace(event)
                        if matches!(event.completion, WorkspaceCompletion::WorkspaceScanCompleted { .. })
                ))
                .count();
        assert!(
            Instant::now() < deadline,
            "deleted note should receive a delayed confirmation scan"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(monitor_event_sender);
    drop(command_sender);
    worker.join().expect("indexer should stop after its command channel closes");
    let catalog = Catalog::open(&catalog_path).expect("catalog should reopen after indexing");
    assert!(catalog.active_notes().expect("active notes should query").is_empty());
}

#[test]
fn watcher_batch_refreshes_navigation_after_an_external_directory_change() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let workspace = notora_core::Workspace::open_or_initialize(directory.path())
        .expect("workspace metadata should be initialized");
    let descriptor = workspace.descriptor();
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let (monitor_event_sender, monitor_events) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();
    let mut product = NotoraProduct::new();
    product.set_active_workspace(descriptor.workspace_id, 1);
    let event_sender = product.event_sender();
    let worker = thread::spawn(move || {
        run_indexer(
            workspace,
            catalog_path,
            None,
            default_migration_backup_retention(),
            monitor_events,
            command_receiver,
            WorkspaceEventSender::new(
                event_sender,
                WorkspaceEventScope { workspace_id: descriptor.workspace_id, generation: 1 },
            ),
        )
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        if product.take_events().iter().any(|event| {
            matches!(
                event,
                NotoraProductEvent::Workspace(event)
                    if matches!(
                        event.completion,
                        WorkspaceCompletion::WorkspaceScanCompleted { .. }
                    )
            )
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "initial scan should complete promptly");
        thread::sleep(Duration::from_millis(10));
    }

    let external_directory = PathBuf::from("external-empty");
    fs::create_dir(directory.path().join(&external_directory))
        .expect("external directory fixture should be created");
    monitor_event_sender
        .send(WorkspaceFileMonitorEvent::Changes(notora_core::WorkspaceFileBatch {
            relative_paths: vec![external_directory.clone()],
            changes: vec![notora_core::WorkspaceFileChange::Created(external_directory.clone())],
        }))
        .expect("watcher batch should reach the indexer");

    loop {
        let _ = product.drain_product_events();
        if product.take_events().iter().any(|event| {
            matches!(
                event,
                NotoraProductEvent::Workspace(event)
                    if matches!(
                        &event.completion,
                        WorkspaceCompletion::NavigationTreeLoaded { tree }
                            if tree.directories.contains(&external_directory)
                    )
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "external directory change should refresh the navigation tree"
        );
        thread::sleep(Duration::from_millis(10));
    }

    drop(monitor_event_sender);
    drop(command_sender);
    worker.join().expect("indexer should stop after its command channel closes");
}

#[test]
fn active_workspace_worker_executes_note_commands_and_returns_a_product_event() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };

    controller
        .execute_note_command(notora_core::note_command::NoteCommand::CreateConfigured(
            notora_core::note_command::ConfiguredCreateNoteRequest {
                kind: notora_core::DocumentKind::Markdown,
                target_directory: None,
                storage: notora_core::CreateNoteStorage::Unencrypted,
            },
        ))
        .expect("active workspace should accept the note command");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::NoteCommandCompleted { result }
                        if result.note.relative_path == std::path::Path::new("无标题.md")
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "note command completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn active_workspace_worker_returns_card_query_completion_with_its_generation() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };
    let query = CardQuery::from(notora_core::NavigationScope::WorkspaceRoot);
    controller.query_cards(query.clone()).expect("worker should accept card query");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::CardQueryCompleted { query: completed_query, page }
                        if completed_query == &query && page.cards.is_empty()
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "card query completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn document_load_completion_carries_editor_metadata_and_formal_tags() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    std::fs::write(directory.path().join("note.md"), "# 路线图\n\n正文")
        .expect("workspace note should be written");
    let workspace = Workspace::open_or_initialize(directory.path())
        .expect("workspace metadata should initialize");
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let catalog = Catalog::open(&catalog_path).expect("workspace catalog should open");
    scan_workspace(&workspace, &catalog).expect("workspace note should be indexed");
    let note_id = catalog
        .active_notes()
        .expect("active notes should load")
        .first()
        .expect("indexed note should exist")
        .note_id;
    let formal_tag = catalog.create_tag("产品/Notora").expect("formal tag should be created");
    let formal_tag_id = formal_tag.tag_id;
    catalog.attach_tag(note_id, formal_tag_id).expect("formal tag should attach");
    drop(catalog);
    drop(workspace);

    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };
    let request =
        DocumentLoadRequest { identity: DocumentIdentity::Note(note_id), selection_generation: 7 };
    controller.prepare_document(request, None).expect("worker should accept document loading");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::DocumentLoaded {
                        request: completed_request,
                        metadata,
                        tags,
                        ..
                    } if *completed_request == request
                        && metadata.note_id == note_id
                        && tags.iter().any(|tag| tag.tag_id == formal_tag_id)
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "document metadata should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn trashed_document_load_reads_the_controlled_trash_file() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    std::fs::write(directory.path().join("trashed.md"), "# 已删除\n\n回收站正文")
        .expect("workspace note should be written");
    let workspace = Workspace::open_or_initialize(directory.path())
        .expect("workspace metadata should initialize");
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let catalog = Catalog::open(&catalog_path).expect("workspace catalog should open");
    scan_workspace(&workspace, &catalog).expect("workspace note should be indexed");
    let note_id = catalog
        .active_notes()
        .expect("active notes should load")
        .first()
        .expect("indexed note should exist")
        .note_id;
    notora_core::move_to_trash(&workspace, &catalog, note_id)
        .expect("fixture note should move to trash");
    drop(catalog);
    drop(workspace);

    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };
    let request =
        DocumentLoadRequest { identity: DocumentIdentity::Note(note_id), selection_generation: 9 };
    controller
        .prepare_trashed_document(request, None)
        .expect("worker should accept trashed document loading");
    let canonical_trash_root = std::fs::canonicalize(directory.path().join(".notora/trash"))
        .expect("trash fixture root should canonicalize");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::DocumentLoaded {
                        request: completed_request,
                        document,
                        ..
                    } if *completed_request == request
                        && document.contents == "# 已删除\n\n回收站正文"
                        && document.path.starts_with(&canonical_trash_root)
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "trashed document should load promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn active_workspace_worker_executes_metadata_mutations_off_the_main_thread() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    std::fs::write(directory.path().join("note.md"), "body")
        .expect("workspace note should be written");
    let workspace = Workspace::open_or_initialize(directory.path())
        .expect("workspace metadata should initialize");
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let catalog = Catalog::open(&catalog_path).expect("workspace catalog should open");
    scan_workspace(&workspace, &catalog).expect("workspace note should be indexed");
    let note_id = catalog
        .active_notes()
        .expect("active notes should load")
        .first()
        .expect("indexed note should exist")
        .note_id;
    drop(catalog);
    drop(workspace);
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };

    controller
        .execute_metadata_mutation(crate::action::MetadataMutation::AttachTagByName {
            note_id,
            display_name: "产品/Notora".to_owned(),
        })
        .expect("active workspace should accept metadata mutations");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::MetadataMutationCompleted {
                        note_id: completed_note_id,
                        metadata,
                        tags,
                        mutation: crate::action::MetadataMutation::AttachTagByName {
                            note_id: mutation_note_id,
                            display_name,
                        },
                        ..
                    } if *completed_note_id == note_id
                        && metadata.note_id == note_id
                        && metadata.encryption == notora_core::NoteEncryption::Unencrypted
                        && *mutation_note_id == note_id
                        && display_name == "产品/Notora"
                        && tags.len() == 1
                        && tags[0].display_name == "产品/Notora"
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "metadata completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn catalog_backup_is_created_by_the_workspace_worker_and_reported_through_the_product_channel() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let backup_directory = tempfile::tempdir().expect("backup test directory should be created");
    let mut controller =
        WorkspaceController::with_catalog_backups_directory(backup_directory.path().to_path_buf());
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };
    let retention =
        notora_core::BackupRetention::keep_latest(1).expect("positive retention should be valid");
    let workspace_backups =
        backup_directory.path().join(active_workspace.descriptor.workspace_id.to_string());

    controller
        .create_catalog_backup(workspace_backups.clone(), retention)
        .expect("worker should accept the catalog backup request");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::CatalogBackupCompleted { backup_path }
                        if backup_path.is_file()
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "catalog backup completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn navigation_tree_uses_filesystem_directories_even_when_they_are_empty() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    fs::create_dir(workspace_directory.path().join("empty"))
        .expect("empty directory should be created");
    fs::create_dir_all(workspace_directory.path().join("docs/plans"))
        .expect("nested empty directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };

    controller.query_navigation_tree().expect("navigation query should reach the worker");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        let loaded_directories = events.iter().find_map(|event| {
            let WorkspaceCompletion::NavigationTreeLoaded { tree } =
                active_completion(event, &active_workspace)?
            else {
                return None;
            };
            Some(tree.directories.clone())
        });
        if let Some(directories) = loaded_directories {
            assert_eq!(
                directories,
                vec![PathBuf::from("docs"), PathBuf::from("docs/plans"), PathBuf::from("empty"),]
            );
            break;
        }
        assert!(Instant::now() < deadline, "navigation tree should load promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn active_workspace_worker_creates_directory_and_refreshes_navigation() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };

    controller
        .execute_directory_command(notora_core::WorkspaceDirectoryCommand::Create {
            parent_relative_path: PathBuf::new(),
            name: "notes".to_owned(),
        })
        .expect("directory command should reach the worker");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut completed = false;
    let mut refreshed = false;
    while !completed || !refreshed {
        let _ = product.drain_product_events();
        for event in product.take_events() {
            match active_completion(&event, &active_workspace) {
                Some(WorkspaceCompletion::DirectoryCommandCompleted { result }) => {
                    completed = result.relative_path == std::path::Path::new("notes");
                }
                Some(WorkspaceCompletion::NavigationTreeLoaded { tree }) => {
                    refreshed = tree.directories.contains(&PathBuf::from("notes"));
                }
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "directory completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(workspace_directory.path().join("notes").is_dir());
}

#[test]
fn opening_an_older_catalog_creates_a_backup_before_migration() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let backup_directory = tempfile::tempdir().expect("backup test directory should be created");
    let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
        .expect("workspace metadata should be initialized");
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    let test_catalog = rusqlite::Connection::open(&catalog_path)
        .expect("test catalog should reopen for schema setup");
    test_catalog
        .execute_batch(
            "CREATE TABLE notes (
                    note_id TEXT PRIMARY KEY NOT NULL,
                    relative_path TEXT NOT NULL UNIQUE,
                    kind INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    excerpt TEXT NOT NULL,
                    modified_ns INTEGER NOT NULL,
                    file_size INTEGER NOT NULL,
                    content_hash BLOB NOT NULL,
                    starred INTEGER NOT NULL DEFAULT 0 CHECK (starred IN (0, 1)),
                    lifecycle INTEGER NOT NULL DEFAULT 0 CHECK (lifecycle IN (0, 1))
                );
                CREATE INDEX notes_active_path_index ON notes(lifecycle, relative_path);
                CREATE INDEX notes_starred_modified_index
                    ON notes(lifecycle, starred, modified_ns DESC);
                CREATE TABLE tags (
                    tag_id TEXT PRIMARY KEY NOT NULL,
                    normalized_name TEXT NOT NULL UNIQUE,
                    display_name TEXT NOT NULL
                );
                CREATE TABLE note_tags (
                    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
                    tag_id TEXT NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
                    PRIMARY KEY (note_id, tag_id)
                );
                CREATE TABLE trash_entries (
                    note_id TEXT PRIMARY KEY NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
                    original_relative_path TEXT NOT NULL,
                    trash_relative_path TEXT NOT NULL UNIQUE,
                    deleted_at_ns INTEGER NOT NULL
                );
                PRAGMA user_version = 1;",
        )
        .expect("test catalog should match the previous schema version");

    let workspace_backup_directory =
        backup_directory.path().join(workspace.descriptor().workspace_id.to_string());
    let mut controller = WorkspaceController::with_catalog_backups_directory_and_retention(
        backup_directory.path().to_path_buf(),
        notora_core::BackupRetention::keep_latest(1)
            .expect("positive migration backup retention should be valid"),
    );
    let mut product = NotoraProduct::new();

    controller
        .execute(
            WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("older catalog should migrate after its backup is complete");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Catalog::migration_required(&catalog_path)
        .expect("workspace catalog migration state should be readable")
    {
        assert!(Instant::now() < deadline, "catalog migration should complete promptly");
        thread::sleep(Duration::from_millis(10));
    }

    let backup_path = notora_core::latest_valid_catalog_backup(&workspace_backup_directory)
        .expect("backup directory should be readable")
        .expect("migration should create a catalog backup");
    assert!(
        Catalog::migration_required(&backup_path)
            .expect("backup should remain readable before migration")
    );
    assert!(
        !Catalog::migration_required(&catalog_path)
            .expect("workspace catalog should be migrated after opening")
    );
}

#[test]
fn catalog_recovery_notifies_the_product_when_metadata_cannot_be_restored() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let backup_directory = tempfile::tempdir().expect("backup test directory should be created");
    let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
        .expect("workspace metadata should be initialized");
    let catalog_path = workspace.metadata_directory().join(CATALOG_FILE_NAME);
    std::fs::write(&catalog_path, "not a sqlite catalog").expect("test catalog should be damaged");
    let mut controller =
        WorkspaceController::with_catalog_backups_directory(backup_directory.path().to_path_buf());
    let mut product = NotoraProduct::new();

    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: workspace_directory.path().to_path_buf() },
            &mut product,
        )
        .expect("damaged catalog should rebuild without blocking workspace access")
    else {
        panic!("workspace should open after catalog recovery");
    };

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::CatalogRecoveryNotified { message }
                        if message.contains("元数据可能已丢失")
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "catalog recovery notice should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn active_workspace_worker_moves_only_the_requested_note_to_trash() {
    let directory = tempfile::tempdir().expect("workspace test directory should be created");
    let mut controller = WorkspaceController::default();
    let mut product = NotoraProduct::new();
    let WorkspaceCommandResult::Opened(active_workspace) = controller
        .execute(
            WorkspaceCommand::OpenExisting { root: directory.path().to_path_buf() },
            &mut product,
        )
        .expect("workspace should open")
    else {
        panic!("open command should activate the workspace");
    };
    controller
        .execute_note_command(notora_core::note_command::NoteCommand::CreateConfigured(
            notora_core::note_command::ConfiguredCreateNoteRequest {
                kind: notora_core::DocumentKind::Markdown,
                target_directory: None,
                storage: notora_core::CreateNoteStorage::Unencrypted,
            },
        ))
        .expect("worker should create a note");

    let deadline = Instant::now() + Duration::from_secs(2);
    let note_id = loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if let Some(note_id) =
            events.iter().find_map(|event| match active_completion(event, &active_workspace) {
                Some(WorkspaceCompletion::NoteCommandCompleted { result }) => {
                    Some(result.note.note_id)
                }
                _ => None,
            })
        {
            break note_id;
        }
        assert!(Instant::now() < deadline, "note creation completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    };
    controller
        .execute_trash_operation(crate::action::TrashOperation::MoveToTrash { note_id })
        .expect("worker should accept the exact note to trash");

    loop {
        let _ = product.drain_product_events();
        let events = product.take_events();
        if events.iter().any(|event| {
            active_completion(event, &active_workspace).is_some_and(|completion| {
                matches!(
                    completion,
                    WorkspaceCompletion::TrashOperationCompleted {
                        operation: crate::action::TrashOperation::MoveToTrash {
                            note_id: completed_note_id,
                        },
                    } if *completed_note_id == note_id
                )
            })
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "trash completion should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!directory.path().join("无标题.md").exists());
}

#[test]
fn regression_save_reindex_scans_only_the_saved_note() {
    let directory = tempfile::tempdir().expect("workspace test directory should exist");
    let workspace =
        Workspace::open_or_initialize(directory.path()).expect("workspace should initialize");
    let catalog = Catalog::open(&workspace.root().join(".notora").join(CATALOG_FILE_NAME))
        .expect("catalog should open");
    let saved_path = PathBuf::from("saved.md");
    let unrelated_path = PathBuf::from("unrelated.md");
    fs::write(workspace.root().join(&saved_path), "# Saved note").expect("saved note should exist");
    fs::write(workspace.root().join(&unrelated_path), "# Unrelated note")
        .expect("unrelated note should exist");
    let product = NotoraProduct::new();
    let events = WorkspaceEventSender::new(
        product.event_sender(),
        WorkspaceEventScope { workspace_id: workspace.descriptor().workspace_id, generation: 1 },
    );

    super::execute_workspace_command(
        &workspace,
        &catalog,
        IndexWorkerCommand::ReindexCatalog { relative_path: Some(saved_path.clone()) },
        &events,
    );

    let indexed = catalog.active_notes().expect("catalog should list indexed notes");
    let paths = indexed.into_iter().map(|note| note.relative_path).collect::<Vec<_>>();
    assert_eq!(paths, [saved_path]);

    super::execute_workspace_command(
        &workspace,
        &catalog,
        IndexWorkerCommand::ReindexCatalog { relative_path: Some(PathBuf::from("missing.md")) },
        &events,
    );
    assert!(
        catalog
            .active_notes()
            .expect("full fallback should refresh unrelated notes")
            .iter()
            .any(|note| note.relative_path == unrelated_path)
    );
}
