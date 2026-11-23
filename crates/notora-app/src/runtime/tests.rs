use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use super::document_runtime::{
    DocumentCommand, PendingNoteMove, PendingTitleUpdate, PendingTrashMove,
    initial_title_from_document,
};
use super::frame_runtime::{FontSystemPreparation, StartupMilestone};
use super::{
    NotoraRuntime, SettingsPersistenceState, StartupTrace, resolve_pointer_cursor,
    workspace_relative_directory,
};
use crate::action::{
    DocumentLoadRequest, MetadataMutation, NotoraAction, WorkspaceTransitionRequest,
};
use crate::autosave::{AutoSaveRequest, AutoSaveState};
use crate::editor_adapter::LoadedDocument;
use crate::state::{CardPageState, normalize_notora_title};
use crate::{
    CompactContent, ExternalFileSession, FocusTarget, NotoraPaths, OverlayState, WorkspaceCommand,
    WorkspaceRootState,
};
use appkit_shell::editor_runtime::{
    DocumentTextReplacement, EditorFocus, EditorInputContext, EditorNotification,
};
use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, WorkspaceId};

#[test]
fn product_pointer_cursor_takes_priority_over_editor_feedback() {
    use winit::window::CursorIcon;

    assert_eq!(
        resolve_pointer_cursor(Some(CursorIcon::EwResize), Some(CursorIcon::Text)),
        CursorIcon::EwResize
    );
    assert_eq!(resolve_pointer_cursor(None, Some(CursorIcon::Grab)), CursorIcon::Grab);
    assert_eq!(resolve_pointer_cursor(None, None), CursorIcon::Default);
}

pub(super) fn app() -> NotoraRuntime {
    let directory = tempfile::tempdir().expect("test should create a temporary directory");
    let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
        .expect("test should create isolated product paths");
    NotoraRuntime::with_paths(paths).expect("notora app should construct without a window")
}

#[test]
fn title_bar_navigation_toggle_preserves_width_and_respects_modal_input() {
    let mut app = app();
    let width = app.state().layout.navigation_width_logical;
    app.toggle_navigation_from_title_bar();
    assert_eq!(
        app.state().layout.navigation_pane_visibility,
        crate::NavigationPaneVisibility::Collapsed
    );
    assert_eq!(app.state().layout.navigation_width_logical, width);
    app.toggle_navigation_from_title_bar();
    assert_eq!(
        app.state().layout.navigation_pane_visibility,
        crate::NavigationPaneVisibility::Expanded
    );
    app.dispatch_action(NotoraAction::OpenSettings);
    app.toggle_navigation_from_title_bar();
    assert_eq!(
        app.state().layout.navigation_pane_visibility,
        crate::NavigationPaneVisibility::Expanded
    );
}

#[test]
fn title_bar_navigation_toggle_opens_and_closes_compact_navigation() {
    let mut app = app();
    app.window_runtime.set_size(700, 600);
    app.toggle_navigation_from_title_bar();
    assert_eq!(app.state().layout.compact_navigation, crate::CompactNavigation::Visible);
    app.toggle_navigation_from_title_bar();
    assert_eq!(app.state().layout.compact_navigation, crate::CompactNavigation::Hidden);
    assert_eq!(app.state().layout.focus_target, FocusTarget::CardList);
}

fn encryption_runtime_test_guard() -> MutexGuard<'static, ()> {
    static ENCRYPTION_RUNTIME_TEST_LOCK: Mutex<()> = Mutex::new(());

    ENCRYPTION_RUNTIME_TEST_LOCK
        .lock()
        .expect("an encryption runtime test must not poison the shared test lock")
}

fn finish_pending_session_restore(app: &mut NotoraRuntime) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.session_restore_runtime.is_active() {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "session restore should complete promptly");
        thread::yield_now();
    }
}

pub(super) fn install_registered_note(
    app: &mut NotoraRuntime,
    path: &str,
    contents: &str,
) -> (DocumentIdentity, appkit_core::workspace::types::TabId) {
    let identity = DocumentIdentity::Note(notora_core::NoteId::generate());
    let prepared = crate::editor_adapter::prepare_loaded_document(
        &app.document_runtime.editor_runtime,
        LoadedDocument {
            path: std::path::PathBuf::from(path),
            contents: contents.to_owned(),
            disk_revision: None,
        },
    )
    .expect("registered note fixture should prepare");
    let _ = app.document_runtime.editor_runtime.install_prepared_tab(
        prepared,
        None,
        appkit_shell::editor_runtime::OpenDisposition::Persistent,
    );
    let tab_id = app
        .document_runtime
        .editor_runtime
        .active_tab_id()
        .expect("registered note fixture should become active");
    let _ = app.document_runtime.document_registry.register(identity, tab_id);
    app.action_runtime.state.library.selected_card = Some(identity);
    (identity, tab_id)
}

fn regular_files_below(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut pending_directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending_directories.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending_directories.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn encrypted_creation_installs_empty_editor_with_unlocked_session() {
    use ui::core::widget::SensitiveText;

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::BeginEncryptedNoteCreation);
    for action in [
        NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "runtime-test-password".to_owned(),
        )),
        NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "runtime-test-password".to_owned(),
        )),
        NotoraAction::EncryptedNoteDialogSubmitRequested,
    ] {
        app.dispatch_action(action);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    let (identity, tab_id) = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_runtime.tab_for(identity)
        {
            break (identity, tab_id);
        }
        assert!(Instant::now() < deadline, "encrypted creation should finish promptly");
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(matches!(identity, DocumentIdentity::Note(_)));
    assert!(app.document_runtime.unlocked_note_session(tab_id).is_some());
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created editor should expose a snapshot")
            .text,
        ""
    );
    let serialized = std::fs::read(workspace.path().join("无标题.md"))
        .expect("created encrypted file should be readable");
    textora_encryption::inspect_encrypted_markdown(&serialized)
        .expect("created file should be a strict encrypted envelope");
}

#[test]
fn encrypted_note_requires_password_after_runtime_restart() {
    use ui::core::widget::SensitiveText;

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut creating_app = app();
    creating_app
        .execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace.path().to_path_buf(),
        })
        .expect("workspace should open for creation");
    creating_app.dispatch_action(NotoraAction::BeginEncryptedNoteCreation);
    for action in [
        NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "restart-test-password".to_owned(),
        )),
        NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "restart-test-password".to_owned(),
        )),
        NotoraAction::EncryptedNoteDialogSubmitRequested,
    ] {
        creating_app.dispatch_action(action);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let identity = loop {
        creating_app.drain_product_events();
        if let Some(identity) = creating_app.action_runtime.state().library.selected_card
            && creating_app.document_runtime.tab_for(identity).is_some()
        {
            break identity;
        }
        assert!(Instant::now() < deadline, "encrypted creation should finish promptly");
        std::thread::sleep(Duration::from_millis(10));
    };
    creating_app
        .execute_workspace_command(WorkspaceCommand::Close)
        .expect("first runtime should close its workspace");
    drop(creating_app);

    let mut reopening_app = app();
    reopening_app
        .execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace.path().to_path_buf(),
        })
        .expect("workspace should reopen");
    reopening_app.dispatch_action(NotoraAction::CardSelected(identity));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        reopening_app.drain_product_events();
        if matches!(
            reopening_app.action_runtime.state().encrypted_note_unlock,
            crate::state::EncryptedNoteUnlockState::Editing { .. }
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "encrypted note should request an unlock password");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(reopening_app.document_runtime.tab_for(identity).is_none());

    reopening_app.dispatch_action(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "incorrect-password".to_owned(),
    )));
    reopening_app.dispatch_action(NotoraAction::EncryptedNoteDialogSubmitRequested);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        reopening_app.drain_product_events();
        if matches!(
            &reopening_app.action_runtime.state().encrypted_note_unlock,
            crate::state::EncryptedNoteUnlockState::Editing {
                error_message: Some(message),
                ..
            } if message == "密码错误"
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "wrong password should return a stable failure");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(reopening_app.document_runtime.tab_for(identity).is_none());

    reopening_app.dispatch_action(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "restart-test-password".to_owned(),
    )));
    reopening_app.dispatch_action(NotoraAction::EncryptedNoteDialogSubmitRequested);
    let deadline = Instant::now() + Duration::from_secs(3);
    let tab_id = loop {
        reopening_app.drain_product_events();
        if let Some(tab_id) = reopening_app.document_runtime.tab_for(identity) {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "correct password should unlock the note");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(reopening_app.document_runtime.unlocked_note_session(tab_id).is_some());
    assert_eq!(
        reopening_app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("unlocked editor should expose plaintext")
            .text,
        ""
    );
}

#[test]
fn encrypted_note_reopens_from_the_process_cache_without_another_password() {
    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (encrypted_identity, encrypted_tab_id) =
        create_encrypted_note_for_test(&mut app, workspace.path(), "process-cache-password");

    let _ = app.document_runtime.editor_mut().close_for_product(encrypted_tab_id);
    app.document_runtime.remove_tab(encrypted_tab_id);
    assert_eq!(app.document_runtime.tab_for(encrypted_identity), None);
    assert!(app.document_runtime.unlocked_note_session(encrypted_tab_id).is_none());

    app.dispatch_action(NotoraAction::CardSelected(encrypted_identity));
    let deadline = Instant::now() + Duration::from_secs(3);
    let reopened_tab_id = loop {
        app.drain_product_events();
        if let Some(tab_id) = app.document_runtime.tab_for(encrypted_identity) {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "cached encrypted note should reopen promptly");
        assert!(matches!(
            app.action_runtime.state().encrypted_note_unlock,
            crate::state::EncryptedNoteUnlockState::Inactive
        ));
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_ne!(reopened_tab_id, encrypted_tab_id);
    assert!(matches!(
        app.action_runtime.state().encrypted_note_unlock,
        crate::state::EncryptedNoteUnlockState::Inactive
    ));
    assert!(app.document_runtime.unlocked_note_session(reopened_tab_id).is_some());
}

#[test]
fn switching_workspaces_clears_cached_encrypted_note_sessions() {
    let _encryption_test_guard = encryption_runtime_test_guard();
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let target_workspace = tempfile::tempdir().expect("target workspace should exist");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");
    let path = old_workspace.path().join("encrypted.md");
    let (identity, _) =
        install_registered_note(&mut app, path.to_str().expect("fixture path should be UTF-8"), "");
    let DocumentIdentity::Note(note_id) = identity else {
        panic!("created encrypted note should have a workspace identity");
    };
    let password = textora_encryption::EncryptionPassword::new("workspace-cache-password".into())
        .expect("fixture password should be valid");
    let (serialized, session) = textora_encryption::create_encrypted_markdown(&password, b"")
        .expect("fixture should create a valid note session")
        .into_parts();
    std::fs::write(&path, serialized).expect("fixture should write the encrypted note");
    let selection = app.document_selection();
    app.document_runtime.install_created_encrypted_note(
        DocumentLoadRequest { identity, selection_generation: selection.generation },
        LoadedDocument { path, contents: String::new(), disk_revision: None },
        std::sync::Arc::new(session),
        selection,
    );
    assert!(app.document_runtime.cached_unlocked_workspace_note_session(note_id).is_some());

    app.dispatch_action(NotoraAction::WorkspaceTransitionConfirmed(
        WorkspaceTransitionRequest::OpenExisting { root: target_workspace.path().to_path_buf() },
    ));

    assert!(app.document_runtime.tab_for(identity).is_none());
    assert!(app.document_runtime.cached_unlocked_workspace_note_session(note_id).is_none());
}

fn active_editor_input_context() -> EditorInputContext {
    EditorInputContext { focus: EditorFocus::Active, modal_blocked: false }
}

fn create_encrypted_note_for_test(
    app: &mut NotoraRuntime,
    workspace_root: &std::path::Path,
    password: &str,
) -> (DocumentIdentity, appkit_core::workspace::types::TabId) {
    use ui::core::widget::SensitiveText;

    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_root.to_path_buf(),
    })
    .expect("workspace should open for encrypted-note test");
    app.dispatch_action(NotoraAction::BeginEncryptedNoteCreation);
    for action in [
        NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(password.to_owned())),
        NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(password.to_owned())),
        NotoraAction::EncryptedNoteDialogSubmitRequested,
    ] {
        app.dispatch_action(action);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_runtime.tab_for(identity)
        {
            return (identity, tab_id);
        }
        assert!(Instant::now() < deadline, "encrypted creation should finish promptly");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn newly_created_encrypted_note_persists_immediate_title_and_body_edits() {
    const PASSWORD: &str = "new-encrypted-note-password";
    const TITLE: &str = "私密路线图";
    const FIRST_FRAGMENT: &str = "第一段";
    const SECOND_FRAGMENT: &str = "\n第二段";

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (identity, tab_id) = create_encrypted_note_for_test(&mut app, workspace.path(), PASSWORD);

    assert!(
        app.action_runtime.state().library.active_editor_metadata.is_some(),
        "a newly created encrypted note must be ready for title commits before editing starts"
    );
    app.dispatch_action(NotoraAction::TitleCommitRequested(TITLE.to_owned()));

    for fragment in [FIRST_FRAGMENT, SECOND_FRAGMENT] {
        let outcome = app
            .document_runtime
            .editor_runtime
            .commit_text(active_editor_input_context(), fragment.to_owned());
        app.apply_editor_outcome(outcome);
    }

    let expected_path = app
        .workspace_controller
        .active_workspace()
        .expect("created note should retain its active workspace")
        .descriptor
        .root
        .join(format!("{TITLE}.md"));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.drain_product_events();
        let path_synchronized = app
            .document_runtime
            .tab_for(identity)
            .and_then(|tab_id| app.document_runtime.editor_runtime.document_summary(tab_id))
            .is_some_and(|summary| summary.path.as_deref() == Some(&expected_path));
        if path_synchronized && expected_path.is_file() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "title and body should persist promptly; summary={:?}, error={:?}, files={:?}",
            app.document_runtime
                .tab_for(identity)
                .and_then(|tab_id| app.document_runtime.editor_runtime.document_summary(tab_id)),
            app.action_runtime.state().library.last_command_error,
            regular_files_below(workspace.path()),
        );
        thread::sleep(Duration::from_millis(10));
    }

    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save(tab_id)
        .expect("immediate body edits should produce a save snapshot");
    let transform = app
        .document_runtime
        .save_payload_transform(tab_id)
        .expect("created encrypted note should retain its save transform")
        .expect("encrypted note save must not use plaintext identity transform");
    let completion =
        appkit_shell::editor_runtime::execute_prepared_save_with_transform(prepared, transform);
    assert!(completion.result.is_ok());
    let outcome = app.document_runtime.editor_runtime.apply_save_completion(completion);
    app.apply_editor_outcome(outcome);

    let password = textora_encryption::EncryptionPassword::new(PASSWORD.to_owned())
        .expect("test password should satisfy policy");
    let encrypted = std::fs::read(expected_path).expect("renamed encrypted note should exist");
    assert_eq!(
        textora_encryption::unlock_encrypted_markdown(&encrypted, &password)
            .expect("saved encrypted note should authenticate")
            .plaintext(),
        format!("{FIRST_FRAGMENT}{SECOND_FRAGMENT}")
    );
}

#[test]
fn shutdown_makes_the_latest_encrypted_revision_immediately_saveable() {
    const PASSWORD: &str = "shutdown-save-password";
    const LATEST_FRAGMENT: &str = "退出前刚输入的内容";

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (_identity, tab_id) = create_encrypted_note_for_test(&mut app, workspace.path(), PASSWORD);
    let outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), LATEST_FRAGMENT.to_owned());
    app.apply_editor_outcome(outcome);
    assert!(matches!(
        app.document_runtime.autosave_state(tab_id),
        Some(AutoSaveState::Scheduled { .. })
    ));

    app.request_immediate_shutdown_workspace_note_saves();

    let requests = app.document_runtime.take_due_autosaves();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tab_id, tab_id);
    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save(tab_id)
        .expect("shutdown should prepare the latest encrypted revision");
    assert_eq!(prepared.content_revision, requests[0].content_revision);
    let transform = app
        .document_runtime
        .save_payload_transform(tab_id)
        .expect("encrypted shutdown save should retain its session")
        .expect("encrypted shutdown save must transform plaintext");
    let completion =
        appkit_shell::editor_runtime::execute_prepared_save_with_transform(prepared, transform);
    assert!(completion.result.is_ok());

    let password = textora_encryption::EncryptionPassword::new(PASSWORD.to_owned())
        .expect("test password should satisfy policy");
    let encrypted = std::fs::read(workspace.path().join("无标题.md"))
        .expect("encrypted note should remain readable after the shutdown save");
    assert_eq!(
        textora_encryption::unlock_encrypted_markdown(&encrypted, &password)
            .expect("shutdown save should authenticate")
            .plaintext(),
        LATEST_FRAGMENT
    );
}

#[test]
fn encrypted_save_transforms_plaintext_and_requires_session() {
    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (_identity, tab_id) =
        create_encrypted_note_for_test(&mut app, workspace.path(), "save-test-password");
    let original_envelope = std::fs::read(workspace.path().join("无标题.md"))
        .expect("original encrypted envelope should be readable");
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "sensitive-save-marker".to_owned());
    app.apply_editor_outcome(edit_outcome);
    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save(tab_id)
        .expect("dirty encrypted note should prepare a plaintext snapshot");
    let transform = app
        .document_runtime
        .save_payload_transform(tab_id)
        .expect("unlocked note should select an encryption transform")
        .expect("encrypted tab should never use identity transform");
    let completion =
        appkit_shell::editor_runtime::execute_prepared_save_with_transform(prepared, transform);
    assert!(completion.result.is_ok());
    let _ = app.document_runtime.editor_runtime.apply_save_completion(completion);

    let saved_envelope = std::fs::read(workspace.path().join("无标题.md"))
        .expect("saved encrypted envelope should be readable");
    assert_ne!(saved_envelope, original_envelope, "each save must use a fresh nonce");
    assert!(
        !saved_envelope
            .windows("sensitive-save-marker".len())
            .any(|window| { window == "sensitive-save-marker".as_bytes() })
    );
    let password = textora_encryption::EncryptionPassword::new("save-test-password".to_owned())
        .expect("test password should satisfy policy");
    assert_eq!(
        textora_encryption::unlock_encrypted_markdown(&saved_envelope, &password)
            .expect("saved envelope should authenticate")
            .plaintext(),
        "sensitive-save-marker"
    );

    app.document_runtime.remove_unlocked_note_session_for_test(tab_id);
    assert!(matches!(
        app.document_runtime.save_payload_transform(tab_id),
        Err(message) if message == "加密笔记缺少解锁会话，保存已取消"
    ));
}

#[test]
fn encrypted_note_plaintext_and_password_never_reach_workspace_catalog_or_snapshots() {
    const PLAINTEXT_MARKER: &str = "encrypted-leakage-marker-7e654a";
    const PASSWORD_MARKER: &str = "leakage-test-password";

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (_identity, tab_id) =
        create_encrypted_note_for_test(&mut app, workspace.path(), PASSWORD_MARKER);
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), PLAINTEXT_MARKER.to_owned());
    app.apply_editor_outcome(edit_outcome);

    app.write_dirty_snapshots_in_background();
    assert!(regular_files_below(&app.paths.snapshots_directory).is_empty());

    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save(tab_id)
        .expect("dirty encrypted note should prepare for save");
    let transform = app
        .document_runtime
        .save_payload_transform(tab_id)
        .expect("encrypted save transform should resolve")
        .expect("encrypted note should require a transform");
    let completion =
        appkit_shell::editor_runtime::execute_prepared_save_with_transform(prepared, transform);
    assert!(completion.result.is_ok());
    let _ = app.document_runtime.editor_runtime.apply_save_completion(completion);
    app.request_catalog_reindex_after_note_save(tab_id);
    for _ in 0..50 {
        app.drain_product_events();
        std::thread::sleep(Duration::from_millis(10));
    }

    for root in [workspace.path(), app.paths.config_directory.as_path()] {
        for path in regular_files_below(root) {
            let bytes = std::fs::read(&path).expect("acceptance artifact should be readable");
            assert!(
                !bytes
                    .windows(PLAINTEXT_MARKER.len())
                    .any(|window| { window == PLAINTEXT_MARKER.as_bytes() }),
                "plaintext leaked into {}",
                path.display()
            );
            assert!(
                !bytes
                    .windows(PASSWORD_MARKER.len())
                    .any(|window| { window == PASSWORD_MARKER.as_bytes() }),
                "password leaked into {}",
                path.display()
            );
        }
    }
}

#[test]
fn encrypted_conflict_copy_uses_a_new_document_identity_and_never_writes_plaintext() {
    use ui::core::widget::SensitiveText;

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (identity, _tab_id) =
        create_encrypted_note_for_test(&mut app, workspace.path(), "original-password");
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "conflict-copy-marker".to_owned());
    app.apply_editor_outcome(edit_outcome);
    let original_envelope = std::fs::read(workspace.path().join("无标题.md"))
        .expect("original envelope should be readable");
    let target_path = workspace.path().join("加密冲突副本.md");
    app.dispatch_action(NotoraAction::SaveConflictDetected { identity, content_revision: 1 });
    app.dispatch_action(NotoraAction::EncryptedConflictCopyRequired {
        identity,
        target_path: target_path.clone(),
    });
    for action in [
        NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
            "conflict-copy-password".to_owned(),
        )),
        NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
            "conflict-copy-password".to_owned(),
        )),
        NotoraAction::EncryptedNoteDialogSubmitRequested,
    ] {
        app.dispatch_action(action);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    while !target_path.is_file() {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "encrypted conflict copy should finish promptly");
        std::thread::sleep(Duration::from_millis(10));
    }
    app.drain_product_events();

    let copied_envelope =
        std::fs::read(&target_path).expect("encrypted conflict copy should be readable");
    assert!(
        !copied_envelope
            .windows("conflict-copy-marker".len())
            .any(|window| { window == "conflict-copy-marker".as_bytes() })
    );
    let original_header = textora_encryption::inspect_encrypted_markdown(&original_envelope)
        .expect("original should remain a valid envelope");
    let copied_header = textora_encryption::inspect_encrypted_markdown(&copied_envelope)
        .expect("copy should be a valid envelope");
    assert_ne!(original_header.document_id, copied_header.document_id);
    let password = textora_encryption::EncryptionPassword::new("conflict-copy-password".to_owned())
        .expect("copy password should satisfy policy");
    assert_eq!(
        textora_encryption::unlock_encrypted_markdown(&copied_envelope, &password)
            .expect("copy should unlock with its supplied password")
            .plaintext(),
        "conflict-copy-marker"
    );
}

#[test]
fn encrypted_conflict_reload_relocks_when_the_external_document_identity_changes() {
    use ui::core::widget::SensitiveText;

    let _encryption_test_guard = encryption_runtime_test_guard();
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let mut app = app();
    let (identity, original_tab_id) =
        create_encrypted_note_for_test(&mut app, workspace.path(), "original-password");
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "local-unsaved-text".to_owned());
    app.apply_editor_outcome(edit_outcome);
    let replacement_password =
        textora_encryption::EncryptionPassword::new("replacement-password".to_owned())
            .expect("replacement password should satisfy policy");
    let replacement =
        textora_encryption::create_encrypted_markdown(&replacement_password, b"replacement")
            .expect("replacement envelope should be created")
            .into_parts()
            .0;
    std::fs::write(workspace.path().join("无标题.md"), replacement)
        .expect("external replacement should be written");
    let content_revision = app
        .document_runtime
        .editor_runtime
        .document_summary(original_tab_id)
        .expect("encrypted tab should have a summary")
        .content_revision;
    app.dispatch_action(NotoraAction::SaveConflictDetected { identity, content_revision });
    app.dispatch_action(NotoraAction::SaveConflictResolutionRequested(
        crate::action::ConflictResolution::ReloadFromDisk,
    ));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.drain_product_events();
        if matches!(
            app.action_runtime.state().encrypted_note_unlock,
            crate::state::EncryptedNoteUnlockState::Editing { .. }
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "replacement should require a fresh unlock");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.document_runtime.tab_for(identity).is_none());
    assert!(app.document_runtime.unlocked_note_session(original_tab_id).is_none());

    app.dispatch_action(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "replacement-password".to_owned(),
    )));
    app.dispatch_action(NotoraAction::EncryptedNoteDialogSubmitRequested);
    let reopened_tab_id = loop {
        app.drain_product_events();
        if let Some(tab_id) = app.document_runtime.tab_for(identity) {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "replacement password should unlock the new file");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_ne!(reopened_tab_id, original_tab_id);
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(reopened_tab_id)
            .expect("reopened replacement should expose plaintext")
            .text,
        "replacement"
    );
}

fn install_registered_external(
    app: &mut NotoraRuntime,
    path: &std::path::Path,
) -> (DocumentIdentity, appkit_core::workspace::types::TabId) {
    let identity = DocumentIdentity::ExternalFile(notora_core::ExternalFileId::generate());
    let prepared = crate::editor_adapter::prepare_loaded_document(
        &app.document_runtime.editor_runtime,
        LoadedDocument {
            path: path.to_path_buf(),
            contents: "外部正文".to_owned(),
            disk_revision: None,
        },
    )
    .expect("external fixture should prepare");
    let _ = app.document_runtime.editor_runtime.install_prepared_tab(
        prepared,
        None,
        appkit_shell::editor_runtime::OpenDisposition::Persistent,
    );
    let tab_id = app
        .document_runtime
        .editor_runtime
        .active_tab_id()
        .expect("external fixture should become active");
    let _ = app.document_runtime.document_registry.register(identity, tab_id);
    (identity, tab_id)
}

fn install_registered_untitled_external(
    app: &mut NotoraRuntime,
) -> (DocumentIdentity, appkit_core::workspace::types::TabId) {
    let identity = app.action_runtime.create_untitled_external(DocumentKind::Markdown);
    app.dispatch_action(NotoraAction::ExternalFileOpened(identity));
    let tab_id =
        app.document_tab_for(identity).expect("untitled external fixture should install a tab");
    (identity, tab_id)
}

#[test]
fn workspace_transition_aborts_when_a_dirty_note_cannot_be_saved() {
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let target_workspace = tempfile::tempdir().expect("target workspace should exist");
    let note_path = std::fs::canonicalize(old_workspace.path())
        .expect("old workspace path should canonicalize")
        .join("draft.md");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");
    let (_, note_tab) = install_registered_note(
        &mut app,
        note_path.to_str().expect("fixture path should be UTF-8"),
        "原始正文",
    );
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "未保存修改".to_owned());
    app.apply_editor_outcome(edit_outcome);

    app.dispatch_action(NotoraAction::WorkspaceTransitionConfirmed(
        WorkspaceTransitionRequest::OpenExisting { root: target_workspace.path().to_path_buf() },
    ));

    assert_eq!(
        app.workspace_controller
            .active_workspace()
            .expect("old workspace should remain active")
            .descriptor
            .root,
        std::fs::canonicalize(old_workspace.path())
            .expect("old workspace path should canonicalize")
    );
    assert!(app.document_runtime.editor_runtime.document_summary(note_tab).is_some());
    assert!(app.state().library.last_command_error.as_deref().is_some_and(|message| {
        message.contains("工作区未切换") && message.contains("保存失败")
    }));
    assert!(
        app.document_runtime
            .editor_runtime
            .document_summary(note_tab)
            .is_some_and(|summary| summary.dirty)
    );
}

#[test]
fn workspace_transition_start_failure_preserves_the_old_workspace_and_note_tabs() {
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let target_workspace = tempfile::tempdir().expect("target workspace should exist");
    std::fs::create_dir(target_workspace.path().join(".notora"))
        .expect("target metadata directory should exist");
    std::fs::write(
        target_workspace.path().join(".notora/workspace.toml"),
        "not valid workspace metadata",
    )
    .expect("corrupt workspace fixture should be written");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");
    let (note_identity, _) = install_registered_note(
        &mut app,
        old_workspace.path().join("kept.md").to_str().expect("fixture path should be UTF-8"),
        "保留正文",
    );

    app.dispatch_action(NotoraAction::WorkspaceTransitionConfirmed(
        WorkspaceTransitionRequest::OpenExisting { root: target_workspace.path().to_path_buf() },
    ));

    assert_eq!(
        app.workspace_controller
            .active_workspace()
            .expect("old workspace should remain active")
            .descriptor
            .root,
        std::fs::canonicalize(old_workspace.path())
            .expect("old workspace path should canonicalize")
    );
    assert!(app.document_tab_for(note_identity).is_some());
}

#[test]
fn successful_workspace_transition_closes_notes_and_keeps_external_tabs() {
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let target_workspace = tempfile::tempdir().expect("target workspace should exist");
    let external_directory = tempfile::tempdir().expect("external directory should exist");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");
    let (note_identity, _) = install_registered_note(
        &mut app,
        old_workspace.path().join("old.md").to_str().expect("fixture path should be UTF-8"),
        "旧工作区正文",
    );
    let (external_identity, external_tab) =
        install_registered_external(&mut app, &external_directory.path().join("outside.md"));
    let (untitled_identity, untitled_tab) = install_registered_untitled_external(&mut app);

    app.dispatch_action(NotoraAction::WorkspaceTransitionConfirmed(
        WorkspaceTransitionRequest::OpenExisting { root: target_workspace.path().to_path_buf() },
    ));

    assert_eq!(
        app.workspace_controller
            .active_workspace()
            .expect("target workspace should become active")
            .descriptor
            .root,
        std::fs::canonicalize(target_workspace.path())
            .expect("target workspace path should canonicalize")
    );
    assert!(app.document_tab_for(note_identity).is_none());
    assert_eq!(app.document_tab_for(external_identity), Some(external_tab));
    assert_eq!(app.document_tab_for(untitled_identity), Some(untitled_tab));
    assert_eq!(app.editor_runtime_tab_count(), 2);
    assert_eq!(app.state().library.navigation_scope, NavigationScope::WorkspaceRoot);
}

#[test]
fn workspace_transition_waits_for_a_successful_dirty_note_save_before_switching() {
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let target_workspace = tempfile::tempdir().expect("target workspace should exist");
    let note_path = std::fs::canonicalize(old_workspace.path())
        .expect("old workspace path should canonicalize")
        .join("saved-before-switch.md");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");
    let (note_identity, note_tab) = install_registered_note(
        &mut app,
        note_path.to_str().expect("fixture path should be UTF-8"),
        "原始正文",
    );
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "切换前保存的正文".to_owned());
    app.apply_editor_outcome(edit_outcome);
    let request =
        WorkspaceTransitionRequest::OpenExisting { root: target_workspace.path().to_path_buf() };
    let candidates = app.document_runtime.workspace_note_save_candidates();
    assert_eq!(candidates.len(), 1);
    app.action_runtime.state.workspace_transition =
        crate::state::WorkspaceTransitionState::AwaitingDirtySaves { request: request.clone() };
    assert!(app.workspace_transition_runtime.begin(request, &candidates));
    let origin = app
        .document_origin_for_tab(note_tab)
        .expect("dirty note should retain its old workspace origin");
    app.document_runtime.request_immediate_workspace_note_save(&origin, candidates[0]);
    let save_requests = app.document_runtime.take_due_autosaves();
    assert_eq!(save_requests.len(), 1);
    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save(save_requests[0].tab_id)
        .expect("dirty note should produce a save snapshot");
    let completion = appkit_shell::editor_runtime::execute_prepared_save(prepared);
    assert!(completion.result.is_ok());
    let save_outcome = app.document_runtime.editor_runtime.apply_save_completion(completion);

    app.apply_editor_outcome(save_outcome);

    assert_eq!(
        std::fs::read_to_string(&note_path).expect("dirty note should be persisted"),
        "原始正文切换前保存的正文"
    );
    assert_eq!(
        app.workspace_controller
            .active_workspace()
            .expect("target workspace should become active")
            .descriptor
            .root,
        std::fs::canonicalize(target_workspace.path())
            .expect("target workspace path should canonicalize")
    );
    assert!(app.document_tab_for(note_identity).is_none());
}

#[test]
fn new_workspace_form_creates_only_the_requested_leaf_directory() {
    let parent = tempfile::tempdir().expect("workspace parent should exist");
    let target = parent.path().join("研究笔记");
    let mut app = app();

    app.dispatch_action(NotoraAction::OpenWorkspaceCreationRequested);
    app.dispatch_action(NotoraAction::WorkspaceCreationNameChanged("研究笔记".to_owned()));
    app.dispatch_action(NotoraAction::WorkspaceCreationLocationSelected(
        parent.path().to_path_buf(),
    ));
    app.dispatch_action(NotoraAction::WorkspaceCreationCommitRequested);

    assert!(target.is_dir());
    assert!(target.join(".notora/workspace.toml").is_file());
    assert_eq!(
        app.workspace_controller
            .active_workspace()
            .expect("created workspace should become active")
            .descriptor
            .root,
        std::fs::canonicalize(&target).expect("created workspace should canonicalize")
    );
}

#[test]
fn existing_create_target_reports_conflict_and_preserves_the_active_workspace() {
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let parent = tempfile::tempdir().expect("workspace parent should exist");
    let existing_target = parent.path().join("已存在");
    std::fs::create_dir(&existing_target).expect("conflict target should exist");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");

    app.dispatch_action(NotoraAction::OpenWorkspaceCreationRequested);
    app.dispatch_action(NotoraAction::WorkspaceCreationNameChanged("已存在".to_owned()));
    app.dispatch_action(NotoraAction::WorkspaceCreationLocationSelected(
        parent.path().to_path_buf(),
    ));
    app.dispatch_action(NotoraAction::WorkspaceCreationCommitRequested);

    assert_eq!(
        app.workspace_controller
            .active_workspace()
            .expect("old workspace should remain active")
            .descriptor
            .root,
        std::fs::canonicalize(old_workspace.path()).expect("old workspace should canonicalize")
    );
    assert!(
        app.state()
            .library
            .last_command_error
            .as_deref()
            .is_some_and(|message| message.contains("已存在"))
    );
    assert!(!existing_target.join(".notora").exists());
}

#[test]
fn cancelling_open_workspace_selection_preserves_the_current_workspace() {
    let old_workspace = tempfile::tempdir().expect("old workspace should exist");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: old_workspace.path().to_path_buf(),
    })
    .expect("old workspace should open");
    let before =
        app.workspace_controller.active_workspace().expect("old workspace should be active");
    app.workspace_directory_chooser = Box::new(|| None);

    app.dispatch_action(NotoraAction::WorkspaceRootSelectionRequested);

    assert_eq!(app.workspace_controller.active_workspace(), Some(before));
    assert_eq!(app.state().workspace_transition, crate::state::WorkspaceTransitionState::Idle);
}

fn note_origin(relative_path: &str) -> notora_core::DocumentOrigin {
    notora_core::DocumentOrigin::Note {
        workspace_id: notora_core::WorkspaceId::generate(),
        note_id: notora_core::NoteId::generate(),
        relative_path: relative_path.into(),
    }
}

fn drain_until_document_text(
    app: &mut NotoraRuntime,
    tab_id: appkit_core::workspace::types::TabId,
    expected_text: &str,
    deadline: Instant,
) {
    loop {
        app.drain_product_events();
        let text = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("document text should remain available")
            .text;
        if text == expected_text {
            return;
        }
        assert!(Instant::now() < deadline, "document text should reach {expected_text:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn notora_title_normalization_does_not_depend_on_document_source() {
    assert_eq!(normalize_notora_title("  项目路线图  "), "项目路线图");
    assert_eq!(normalize_notora_title("   "), "无标题");
}

#[test]
fn autosave_failure_retains_the_diagnostic_for_the_editor_header() {
    let mut app = app();
    let tab_id = appkit_core::workspace::types::TabIdAllocator::new().allocate();
    let request = AutoSaveRequest { tab_id, content_revision: 7 };
    let origin = notora_core::DocumentOrigin::Note {
        workspace_id: notora_core::WorkspaceId::generate(),
        note_id: notora_core::NoteId::generate(),
        relative_path: "diagram.mmap.md".into(),
    };
    app.document_runtime.autosave.request_immediate_save(&origin, tab_id, request.content_revision);
    assert_eq!(app.document_runtime.autosave.take_due_saves(), vec![request]);

    app.record_autosave_failure(request, "file is read-only".to_owned());

    assert_eq!(
        app.document_runtime.autosave.state(tab_id),
        Some(AutoSaveState::Failed { content_revision: request.content_revision })
    );
    assert_eq!(
        app.document_runtime.save_failure_messages.get(&tab_id).map(String::as_str),
        Some("file is read-only")
    );
}

#[test]
fn autosave_request_for_a_clean_document_finishes_without_a_failure() {
    let directory = tempfile::tempdir().expect("save fixture directory should exist");
    let path = directory.path().join("clean.md");
    let mut app = app();
    let (_, tab_id) = install_registered_note(
        &mut app,
        path.to_str().expect("fixture path should be valid UTF-8"),
        "原始正文",
    );
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "已保存修改".to_owned());
    app.apply_editor_outcome(edit_outcome);
    let edited_summary = app
        .document_runtime
        .editor_runtime
        .document_summary(tab_id)
        .expect("edited note should remain available");
    app.document_runtime.autosave.request_immediate_save(
        &note_origin("clean.md"),
        tab_id,
        edited_summary.content_revision,
    );
    let request = app
        .document_runtime
        .autosave
        .take_due_saves()
        .pop()
        .expect("edited revision request should become due");
    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save(tab_id)
        .expect("edited note should prepare a save");
    let save_outcome = app
        .document_runtime
        .editor_runtime
        .apply_save_completion(appkit_shell::editor_runtime::execute_prepared_save(prepared));
    app.apply_editor_outcome(save_outcome);
    let clean_summary = app
        .document_runtime
        .editor_runtime
        .document_summary(tab_id)
        .expect("registered note should remain available");
    assert!(!clean_summary.dirty);

    app.submit_autosave(request);

    assert_eq!(app.document_runtime.autosave.state(tab_id), None);
    assert_eq!(app.document_runtime.save_failure_messages.get(&tab_id), None);
}

#[test]
fn stale_autosave_request_is_replaced_by_the_current_dirty_revision() {
    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "stale.md", "原始正文");
    let first_edit = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "第一次修改".to_owned());
    app.apply_editor_outcome(first_edit);
    let first_revision = app
        .document_runtime
        .editor_runtime
        .document_summary(tab_id)
        .expect("edited note should remain available")
        .content_revision;
    app.document_runtime.autosave.request_immediate_save(
        &note_origin("stale.md"),
        tab_id,
        first_revision,
    );
    let stale_request = app
        .document_runtime
        .autosave
        .take_due_saves()
        .pop()
        .expect("first revision request should become due");
    let second_edit = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "第二次修改".to_owned());
    app.apply_editor_outcome(second_edit);
    let current_revision = app
        .document_runtime
        .editor_runtime
        .document_summary(tab_id)
        .expect("twice-edited note should remain available")
        .content_revision;
    assert_ne!(current_revision, stale_request.content_revision);

    app.submit_autosave(stale_request);

    assert!(matches!(
        app.document_runtime.autosave.state(tab_id),
        Some(AutoSaveState::Scheduled { content_revision, .. })
            if content_revision == current_revision
    ));
    assert_eq!(app.document_runtime.save_failure_messages.get(&tab_id), None);
}

#[test]
fn initial_document_title_supports_markdown_h1_and_compact_mmap_root_syntax() {
    assert_eq!(
        initial_title_from_document(DocumentKind::Markdown, "# Markdown Title\n"),
        Some("Markdown Title".to_owned())
    );
    assert_eq!(
        initial_title_from_document(DocumentKind::Mindmap, "#Mindmap Root\n##Child\n"),
        Some("Mindmap Root".to_owned())
    );
}

fn card_page_contains_note(card_page: &CardPageState, note_id: notora_core::NoteId) -> bool {
    let cards = match card_page {
        CardPageState::LoadingNextPage { cards, .. }
        | CardPageState::Refreshing { cards, .. }
        | CardPageState::Ready { cards, .. }
        | CardPageState::Failed { cards, .. } => cards,
        CardPageState::Idle
        | CardPageState::LoadingInitial { .. }
        | CardPageState::Empty { .. } => return false,
    };
    cards.iter().any(|card| card.note_id == note_id)
}

#[test]
fn duplicate_metadata_mutations_share_one_pending_worker_request() {
    let note_id = notora_core::NoteId::generate();
    let mutation =
        MetadataMutation::AttachTagByName { note_id, display_name: "产品/Notora".to_owned() };
    let mut app = app();

    assert!(app.document_runtime.register_metadata_mutation(mutation.clone(), note_id, 7,));
    assert!(!app.document_runtime.register_metadata_mutation(mutation.clone(), note_id, 7,));
    assert_eq!(app.document_runtime.pending_metadata_mutations, vec![mutation.clone()]);

    assert_eq!(app.document_runtime.complete_metadata_mutation(&mutation, note_id), Some(7));
    assert!(app.document_runtime.pending_metadata_mutations.is_empty());
}

#[test]
fn startup_trace_reports_the_first_frame_once() {
    let mut trace = StartupTrace::started_now();

    assert!(trace.take_first_frame_elapsed().is_some());
    assert!(trace.take_first_frame_elapsed().is_none());
}

#[test]
fn startup_trace_reports_each_usability_milestone_once() {
    let mut trace = StartupTrace::started_now();

    for milestone in [
        StartupMilestone::SessionRestoreStarted,
        StartupMilestone::WorkspaceSessionReady,
        StartupMilestone::SessionRestoreFinished,
        StartupMilestone::RestoredDocumentRendered,
    ] {
        assert!(trace.take_milestone_elapsed(milestone).is_some());
        assert!(trace.take_milestone_elapsed(milestone).is_none());
    }
}

#[test]
fn background_font_preparation_returns_to_the_deferred_state_after_join() {
    let directory = tempfile::tempdir().expect("test should create a temporary directory");
    let paths = NotoraPaths::from_config_directory(directory.path().join("notora"))
        .expect("test should create isolated product paths");
    let mut app =
        NotoraRuntime::with_paths(paths).expect("notora app should construct without a window");

    app.start_font_system_preparation();
    assert!(matches!(
        &app.frame_runtime.font_system_preparation,
        FontSystemPreparation::InProgress(_)
    ));

    let font_system = app.take_prepared_font_system();

    std::hint::black_box(font_system);
    assert!(matches!(app.frame_runtime.font_system_preparation, FontSystemPreparation::Deferred));
}

#[test]
fn move_dialog_destination_is_reduced_to_a_safe_domain_input() {
    let workspace = tempfile::tempdir().expect("workspace fixture should exist");
    let archive_directory = workspace.path().join("archive");
    let outside_directory = tempfile::tempdir().expect("outside fixture should exist");
    std::fs::create_dir_all(&archive_directory).expect("archive directory should exist");

    assert_eq!(
        workspace_relative_directory(workspace.path(), &archive_directory),
        Ok("archive".into())
    );
    assert!(workspace_relative_directory(workspace.path(), outside_directory.path()).is_err());
}

#[test]
fn markdown_bold_shortcut_formats_selection_and_undo_restores_it() {
    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "bold.md", "中文正文");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    let mut tab = app
        .document_runtime
        .editor_runtime
        .tab_session_mut(tab_id)
        .expect("Markdown shortcut fixture should retain its tab");
    tab.document_mut().cursor_move_to_offset("中文".len());
    tab.document_mut().cursor_mut().selection_anchor = Some(0);
    let modifiers = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };

    app.handle_key_input(ui::KeyCode::Char('b'), modifiers, None);

    let tab = app
        .document_runtime
        .editor_runtime
        .tab_session(tab_id)
        .expect("formatted Markdown should retain its tab");
    assert_eq!(tab.document().full_text(), "**中文**正文");
    assert!(tab.document().dirty);

    app.handle_key_input(ui::KeyCode::Char('z'), modifiers, None);

    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("undo should preserve the Markdown document")
            .text,
        "中文正文"
    );
}

#[test]
fn markdown_bold_shortcut_supports_control_without_a_selection() {
    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "bold.md", "正文");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.document_runtime
        .editor_runtime
        .tab_session_mut(tab_id)
        .expect("Markdown shortcut fixture should retain its tab")
        .document_mut()
        .cursor_move_to_offset(0);

    app.handle_key_input(
        ui::KeyCode::Char('B'),
        ui::core::Modifiers { ctrl: true, ..ui::core::Modifiers::NONE },
        None,
    );

    let tab = app
        .document_runtime
        .editor_runtime
        .tab_session(tab_id)
        .expect("Markdown shortcut fixture should retain its tab");
    assert_eq!(tab.document().full_text(), "****正文");
    assert_eq!(tab.document().cursor_offset().to_usize(), "**".len());
}

#[test]
fn markdown_bold_shortcut_respects_focus_modal_preedit_and_source_view() {
    #[derive(Debug)]
    enum Blocker {
        SearchFocus,
        Modal,
        Preedit,
        SourceView,
        WindowUnfocused,
        SelectionDetached,
    }

    for blocker in [
        Blocker::SearchFocus,
        Blocker::Modal,
        Blocker::Preedit,
        Blocker::SourceView,
        Blocker::WindowUnfocused,
        Blocker::SelectionDetached,
    ] {
        let mut app = app();
        let (_, tab_id) = install_registered_note(&mut app, "bold.md", "正文");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        match blocker {
            Blocker::SearchFocus => {
                app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::NavigationSearch))
            }
            Blocker::Modal => app.dispatch_action(NotoraAction::OpenSettings),
            Blocker::Preedit => {
                assert!(app.update_editor_preedit("拼".to_owned(), Some((0, 1))));
            }
            Blocker::SourceView => {
                app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
            }
            Blocker::WindowUnfocused => app.set_window_focused(false),
            Blocker::SelectionDetached => {
                app.action_runtime.state.library.selected_card = None;
            }
        }

        app.handle_key_input(
            ui::KeyCode::Char('b'),
            ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE },
            None,
        );

        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("blocked shortcut should retain the document")
                .text,
            "正文",
            "bold shortcut must be blocked by {blocker:?}"
        );
    }
}

#[test]
fn document_ime_is_gated_by_product_focus_and_modal_state() {
    let mut app = app();
    assert!(!app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    assert!(!app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

    let _ = install_registered_note(&mut app, "ime.md", "");
    assert!(app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

    app.dispatch_action(NotoraAction::OpenSettings);
    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::Settings);
    assert!(!app.update_editor_preedit("音".to_owned(), Some((0, 1))));
}

#[test]
fn navigation_change_detaches_the_visible_editor_from_the_previous_runtime_tab() {
    let mut app = app();
    let _ = install_registered_note(&mut app, "previous.md", "旧正文");

    assert!(app.active_editor_matches_selection());

    app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Trash));

    assert!(!app.active_editor_matches_selection());
}

#[test]
fn switching_away_from_a_preview_removes_its_document_mapping() {
    let mut app = app();
    let (persistent_identity, persistent_tab_id) =
        install_registered_note(&mut app, "persistent.md", "常驻正文");
    let preview_identity = DocumentIdentity::Note(notora_core::NoteId::generate());
    app.action_runtime.state.library.selected_card = Some(preview_identity);
    app.action_runtime.state.library.selected_document_generation += 1;
    let preview_request = DocumentLoadRequest {
        identity: preview_identity,
        selection_generation: app.action_runtime.state().library.selected_document_generation,
    };

    app.install_loaded_preview(
        preview_request,
        LoadedDocument {
            path: "preview.md".into(),
            contents: "预览正文".to_owned(),
            disk_revision: None,
        },
    );
    let preview_tab_id =
        app.document_runtime.tab_for(preview_identity).expect("preview note should be registered");
    assert_ne!(preview_tab_id, persistent_tab_id);

    app.action_runtime.state.library.selected_card = Some(persistent_identity);
    app.action_runtime.state.library.selected_document_generation += 1;
    let actions = app.prepare_document(DocumentLoadRequest {
        identity: persistent_identity,
        selection_generation: app.action_runtime.state().library.selected_document_generation,
    });

    assert!(actions.is_empty());
    assert_eq!(app.document_runtime.editor().active_tab_id(), Some(persistent_tab_id));
    assert_eq!(app.document_runtime.tab_for(preview_identity), None);
}

#[test]
fn clicking_the_editor_transfers_keyboard_focus_from_the_card_list() {
    let mut app = app();
    app.render().expect("shell layout should be available for pointer routing");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
    let editor_rect = app.shell_layout().editor_rect;

    assert!(!app.route_pointer_event(&ui::Event::MouseDown {
        px: editor_rect.x + editor_rect.w * 0.5,
        py: editor_rect.y + editor_rect.h * 0.5,
        button: ui::core::widget::MouseButton::Left,
    }));
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
}

#[test]
fn focused_markdown_document_schedules_cursor_blink() {
    let mut app = app();
    let _ = install_registered_note(&mut app, "caret.md", "正文内容");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

    assert!(app.next_deadline().is_some());
    assert!(app.document_runtime.editor_runtime.active_cursor_paint_enabled());

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTitle));
    assert!(!app.document_runtime.editor_runtime.active_cursor_paint_enabled());

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTag));
    assert!(!app.document_runtime.editor_runtime.active_cursor_paint_enabled());

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
    assert_eq!(app.next_deadline(), None);
}

#[test]
fn product_text_focus_schedules_its_blink_before_another_input_event() {
    let mut app = app();
    assert_eq!(app.frame_runtime.shell.next_text_cursor_blink_at(), None);

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::NavigationSearch));

    assert!(app.frame_runtime.shell.next_text_cursor_blink_at().is_some());
    assert_eq!(app.next_deadline(), app.frame_runtime.shell.next_text_cursor_blink_at());
}

#[test]
fn first_search_click_synchronizes_widget_focus_before_the_next_input_event() {
    let mut app = app();
    app.render().expect("headless shell frame should render");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    let search_rect = app.frame_runtime.shell.search_box_rect();

    assert!(app.route_product_event(&ui::Event::MouseDown {
        px: search_rect.x + search_rect.w * 0.5,
        py: search_rect.y + search_rect.h * 0.5,
        button: ui::core::MouseButton::Left,
    }));

    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationSearch);
    assert!(app.frame_runtime.shell.search_box_is_focused());
}

#[test]
fn search_text_drag_requests_redraw_in_the_same_pointer_event_cycle() {
    let mut app = app();
    app.dispatch_action(NotoraAction::SearchTextChanged("路线图".to_owned()));
    app.render().expect("headless search box should render");
    let search_rect = app.frame_runtime.shell.search_box_rect();

    assert!(app.route_pointer_event(&ui::Event::MouseDown {
        px: search_rect.x + 8.0,
        py: search_rect.y + search_rect.h * 0.5,
        button: ui::core::widget::MouseButton::Left,
    }));
    app.render().expect("focused search box should render");
    let _ = app.take_redraw_request();
    assert!(!app.take_redraw_request());

    assert!(app.route_pointer_event(&ui::Event::MouseMove {
        px: search_rect.right() - 8.0,
        py: search_rect.y + search_rect.h * 0.5,
    }));
    assert!(app.window_runtime.redraw_is_requested(), "drag selection should schedule a redraw");
    assert!(
        app.document_runtime.editor_runtime.take_redraw_request(),
        "drag selection should wake the renderer without waiting for about_to_wait"
    );
}

#[test]
fn settings_modal_consumes_unhandled_keyboard_input() {
    let mut app = app();
    app.dispatch_action(NotoraAction::OpenSettings);

    assert!(app.route_product_event(&ui::Event::KeyDown(
        ui::KeyCode::Char('x'),
        ui::core::Modifiers::NONE,
    )));
}

#[test]
fn settings_modal_blocks_splitter_keyboard_adjustments() {
    let mut app = app();
    app.render().expect("shell layout should initialize splitter inputs");
    let navigation_width_before_modal = app.action_runtime.state().layout.navigation_width_logical;

    app.dispatch_action(NotoraAction::OpenSettings);

    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::Settings);
    assert!(
        app.route_product_event(&ui::Event::KeyDown(ui::KeyCode::Left, ui::core::Modifiers::NONE,))
    );
    assert_eq!(
        app.action_runtime.state().layout.navigation_width_logical,
        navigation_width_before_modal,
        "modal keyboard input must not resize an underlying splitter"
    );
}

#[test]
fn modal_state_blocks_workspace_shortcuts_and_escape_closes_the_modal() {
    let mut app = app();
    app.dispatch_action(NotoraAction::OpenSettings);
    let modal_focus = app.action_runtime.state().layout.focus_target;

    let command_modifiers = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };
    for key_code in [
        ui::KeyCode::Char('n'),
        ui::KeyCode::Char('o'),
        ui::KeyCode::Char('f'),
        ui::KeyCode::Char('s'),
    ] {
        app.handle_key_input(key_code, command_modifiers, None);
    }

    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::Settings);
    assert_eq!(app.action_runtime.state().layout.focus_target, modal_focus);

    app.handle_key_input(ui::KeyCode::Escape, ui::core::Modifiers::NONE, None);
    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationTree);
}

#[test]
fn notora_theme_mode_resolves_against_its_own_product_settings() {
    let mut app = app();
    app.persistence_runtime.product_settings.appearance.theme_mode = ui::ThemeMode::System;
    app.rebuild_theme_for_system_appearance(winit::window::Theme::Light);
    assert!(!app.frame_runtime.theme.is_dark);
    app.rebuild_theme_for_system_appearance(winit::window::Theme::Dark);
    assert!(app.frame_runtime.theme.is_dark);

    app.persistence_runtime.product_settings.appearance.theme_mode = ui::ThemeMode::Light;
    app.rebuild_theme_for_system_appearance(winit::window::Theme::Dark);
    assert!(!app.frame_runtime.theme.is_dark);
}

#[test]
fn notora_editor_setting_updates_its_product_and_runtime_snapshots() {
    let mut app = app();

    app.dispatch_action(NotoraAction::ProductSettingsUpdateRequested(
        crate::settings_overlay::ProductSettingsUpdate::FontSize(20.0),
    ));

    assert_eq!(app.persistence_runtime.product_settings.editor.font_size, 20.0);
    assert_eq!(app.document_runtime.editor_runtime.settings_snapshot().font_size, 20.0);
}

#[test]
fn watcher_disconnection_marks_reconciliation_pending_and_surfaces_a_diagnostic() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.drain_product_events();
    let active_workspace =
        app.workspace_controller.active_workspace().expect("opened workspace should stay active");
    crate::product::WorkspaceEventSender::new(
        app.product.event_sender(),
        crate::product::WorkspaceEventScope {
            workspace_id: active_workspace.descriptor.workspace_id,
            generation: active_workspace.generation,
        },
    )
    .send(crate::product::WorkspaceCompletion::WorkspaceIndexFailed {
        message: "工作区文件监视器已断开，自动同步已停止".to_owned(),
    })
    .expect("product receiver should stay available");

    app.drain_product_events();

    assert!(app.document_runtime.catalog_reconciliation_pending);
    assert!(
        app.action_runtime
            .state()
            .library
            .last_command_error
            .as_deref()
            .is_some_and(|message| message.contains("文件监视器已断开"))
    );
}

#[test]
fn session_workspace_identity_mismatch_closes_the_unexpected_workspace() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    notora_core::Workspace::open_or_initialize(workspace_directory.path())
        .expect("workspace metadata should initialize");
    let mut app = app();
    app.persistence_runtime.pending_session = Some(crate::session::ProductSession {
        workspace_root: Some(workspace_directory.path().to_path_buf()),
        workspace_id: Some(WorkspaceId::generate()),
        ..crate::session::ProductSession::default()
    });

    app.restore_pending_session();

    assert_eq!(app.workspace_controller.active_workspace(), None);
    assert!(app.session_restore_runtime.is_active());
    finish_pending_session_restore(&mut app);
    assert_eq!(app.workspace_controller.active_workspace(), None);
    assert!(
        app.action_runtime
            .state
            .library
            .last_command_error
            .as_deref()
            .is_some_and(|message| message.contains("不再匹配"))
    );
}

#[test]
fn session_restore_registers_all_external_paths_but_activates_only_the_saved_last_document() {
    let directory = tempfile::tempdir().expect("external fixture directory should be created");
    let saved_last_path = directory.path().join("last.md");
    let other_path = directory.path().join("other.md");
    std::fs::write(&saved_last_path, "# Last").expect("last document should be written");
    std::fs::write(&other_path, "# Other").expect("other document should be written");
    let mut app = app();
    app.persistence_runtime.pending_session = Some(crate::session::ProductSession {
        external_paths: vec![saved_last_path.clone(), other_path],
        last_document: Some(crate::session::SavedDocument::ExternalPath {
            path: saved_last_path.clone(),
        }),
        ..crate::session::ProductSession::default()
    });

    app.restore_pending_session();

    let deadline = Instant::now() + Duration::from_secs(2);
    while app.action_runtime.state.external_files.sessions().len() < 2 {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "external sessions should restore promptly");
        thread::sleep(Duration::from_millis(10));
    }
    let selected_identity = app
        .action_runtime
        .state
        .library
        .selected_card
        .expect("saved last external document should be selected");
    let selected_path = match selected_identity {
        DocumentIdentity::ExternalFile(external_file_id) => app
            .action_runtime
            .state
            .external_files
            .session(external_file_id)
            .and_then(|session| match session {
                ExternalFileSession::Existing { canonical_path, .. } => {
                    Some(canonical_path.as_path())
                }
                ExternalFileSession::Untitled { .. } | ExternalFileSession::Missing { .. } => None,
            })
            .expect("selected external session should have a path"),
        DocumentIdentity::Note(_) => panic!("session should restore an external document"),
    };
    let canonical_saved_last_path =
        std::fs::canonicalize(saved_last_path).expect("saved last document should canonicalize");
    assert_eq!(selected_path, canonical_saved_last_path.as_path());
    assert_eq!(app.editor_runtime_tab_count(), 1);
}

#[test]
fn headless_session_capture_uses_the_latest_window_size_with_safe_position_defaults() {
    let mut app = app();
    app.set_window_size(1_024, 768);

    let geometry = app.capture_window_geometry();

    assert_eq!(geometry.x_px, crate::session::WindowGeometry::default().x_px);
    assert_eq!(geometry.y_px, crate::session::WindowGeometry::default().y_px);
    assert_eq!(geometry.width_px, 1_024.0);
    assert_eq!(geometry.height_px, 768.0);
}

#[test]
fn session_capture_includes_navigation_root_expansion_states() {
    let mut app = app();
    app.dispatch_action(NotoraAction::WorkspaceRootExpansionToggled);
    app.dispatch_action(NotoraAction::TagRootExpansionToggled);

    let session = app.capture_product_session();

    assert!(!session.workspace_root_expanded);
    assert!(session.tag_root_expanded);
}

#[test]
fn settings_persistence_failure_is_visible_and_retry_clears_it() {
    let mut app = app();
    let occupied_parent = app.paths.config_directory.join("occupied-settings-parent");
    std::fs::write(&occupied_parent, "not a directory")
        .expect("settings failure fixture should be written");
    app.paths.settings_file = occupied_parent.join("settings.toml");

    app.apply_product_settings_update(crate::settings_overlay::ProductSettingsUpdate::WordWrap(
        false,
    ));

    assert_eq!(app.action_runtime.state.library.last_command_error, None);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        app.drain_product_events();
        if matches!(
            app.persistence_runtime.settings_state,
            SettingsPersistenceState::SaveFailed { .. }
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "settings persistence failure should arrive");
        thread::sleep(Duration::from_millis(10));
    }

    assert!(matches!(
        app.persistence_runtime.settings_state.to_view(),
        crate::settings_overlay::NotoraSettingsPersistenceView::SaveFailed { .. }
    ));
    let retry_path = app.paths.config_directory.join("retry-settings.toml");
    app.paths.settings_file = retry_path.clone();
    app.dispatch_action(NotoraAction::RetryProductSettingsPersistence);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        app.drain_product_events();
        if app.persistence_runtime.settings_state == SettingsPersistenceState::Saved {
            break;
        }
        assert!(Instant::now() < deadline, "settings persistence retry should complete");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!crate::settings::load_product_settings(&retry_path).settings.editor.word_wrap);
}

#[test]
fn search_ime_commit_is_consumed_before_it_can_reach_the_editor_runtime() {
    let mut app = app();
    let _ = install_registered_note(&mut app, "search.md", "搜索正文");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    assert!(app.update_editor_preedit("document".to_owned(), Some((0, 8))));
    app.render().expect("headless shell frame should render");
    let search_rect = app.frame_runtime.shell.search_box_rect();

    assert!(app.route_product_event(&ui::Event::MouseDown {
        px: search_rect.x + search_rect.w * 0.5,
        py: search_rect.y + search_rect.h * 0.5,
        button: ui::core::widget::MouseButton::Left,
    }));
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationSearch);
    assert!(app.route_product_event(&ui::Event::ImeCommit("路线图".to_owned())));

    assert_eq!(
        app.action_runtime.state().library.navigation_scope,
        NavigationScope::Search { query: "路线图".to_owned() }
    );
    assert!(app.document_runtime.editor_runtime.preedit().0.is_empty());
}

#[test]
fn active_workspace_note_creation_reaches_the_worker_and_updates_product_state() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        app.drain_product_events();
        if matches!(
            app.action_runtime.state().library.selected_card,
            Some(notora_core::DocumentIdentity::Note(_))
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "note completion should update product state");
        thread::sleep(Duration::from_millis(10));
    }

    assert!(workspace_directory.path().join("无标题.md").is_file());
    assert_eq!(app.action_runtime.state().library.last_command_error, None);
}

#[test]
fn typed_menu_creation_uses_the_selected_directory_and_opens_the_note() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let notes_directory = workspace_directory.path().join("notes");
    std::fs::create_dir_all(&notes_directory).expect("notes directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Directory {
        relative_path: "notes".into(),
    }));
    app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Text));

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        app.drain_product_events();
        if matches!(
            app.action_runtime.state().library.selected_card,
            Some(DocumentIdentity::Note(_))
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "configured note should be created");
        thread::sleep(Duration::from_millis(10));
    }

    assert!(notes_directory.join("无标题.txt").is_file());
    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
}

#[test]
fn creation_failure_closes_the_menu_and_reports_the_error() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Directory {
        relative_path: "missing".into(),
    }));
    app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    while app.action_runtime.state().library.last_command_error.is_none() {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "creation failure should return to the app");
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
    assert!(!workspace_directory.path().join("missing").exists());
}

#[test]
fn new_markdown_title_commit_renames_the_note_without_modifying_the_body() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card {
            break identity;
        }
        assert!(Instant::now() < deadline, "note completion should select a note");
        thread::sleep(Duration::from_millis(10));
    };
    let tab_id = loop {
        app.drain_product_events();
        if let Some(tab_id) = app.document_tab_for(identity) {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created note should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };

    app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));

    while !workspace_directory.path().join("项目路线图.md").is_file()
        || app
            .state()
            .library
            .active_editor_metadata
            .as_ref()
            .is_none_or(|metadata| metadata.metadata.title_revision < 1)
    {
        app.drain_runtime_save_completions();
        app.drain_product_events();
        assert!(Instant::now() < deadline, "first title should rename its entity file",);
        thread::sleep(Duration::from_millis(10));
    }

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created note text should remain available");
    assert_eq!(snapshot.text, "");
    assert_eq!(snapshot.content_revision, 0);

    app.dispatch_action(NotoraAction::TitleCommitRequested("独立的 Notora 标题".to_owned()));
    let second_deadline = Instant::now() + Duration::from_secs(2);
    while !workspace_directory.path().join("独立的 Notora 标题.md").is_file() {
        app.drain_runtime_save_completions();
        app.drain_product_events();
        assert!(
            Instant::now() < second_deadline,
            "independent title should persist; dirty={:?}, error={:?}, revision={:?}",
            app.document_runtime
                .editor_runtime
                .document_summary(tab_id)
                .map(|summary| summary.dirty),
            app.action_runtime.state().library.last_command_error,
            app.action_runtime
                .state()
                .library
                .active_editor_metadata
                .as_ref()
                .map(|metadata| metadata.metadata.title_revision),
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!workspace_directory.path().join("项目路线图.md").exists());
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("independent body should remain available")
            .text,
        ""
    );
}

#[test]
fn leaving_title_focus_commits_the_current_draft() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created note should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.render().expect("created note should render its title editor");

    assert!(app.route_product_event(&ui::Event::ImeCommit("项目路线图".to_owned())));
    assert_eq!(app.frame_runtime.shell.editor_title_text(), "项目路线图");

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

    while !workspace_directory.path().join("项目路线图.md").is_file() {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "title commit should rename the markdown file");
        thread::sleep(Duration::from_millis(10));
    }
    app.render().expect("committed title should render");
    assert_eq!(app.frame_runtime.shell.editor_title_text(), "项目路线图");
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created markdown source should remain available")
            .text,
        ""
    );
}

#[test]
fn first_saved_markdown_h1_initializes_the_title_and_then_becomes_independent() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    let (note_id, tab_id) = loop {
        app.drain_product_events();
        if let Some(DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(DocumentIdentity::Note(note_id))
            && app.action_runtime.state().library.active_editor_metadata.is_some()
        {
            break (note_id, tab_id);
        }
        assert!(Instant::now() < deadline, "created note should finish loading");
        thread::sleep(Duration::from_millis(10));
    };
    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created source should exist");
    let outcome = app
        .document_runtime
        .editor_runtime
        .replace_document_text(DocumentTextReplacement {
            tab_id,
            content_revision: snapshot.content_revision,
            range: 0..snapshot.text.len(),
            replacement: "# 正文优先\n\n正文".to_owned(),
        })
        .expect("body fixture edit should apply");
    app.apply_editor_outcome(outcome);
    let content_revision = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("edited source should exist")
        .content_revision;
    app.handle_editor_notification(&EditorNotification::SaveCompleted { tab_id, content_revision });

    while app.document_runtime.pending_metadata_mutations.iter().any(|mutation| {
        matches!(mutation, MetadataMutation::CompleteTitleInitializationFromDocument { .. })
    }) {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "document initialization should persist");
        thread::sleep(Duration::from_millis(10));
    }
    let catalog =
        notora_core::Catalog::open(&workspace_directory.path().join(".notora/catalog.sqlite3"))
            .expect("catalog should reopen for verification");
    assert_eq!(
        catalog
            .active_note(note_id)
            .expect("note lookup should succeed")
            .expect("note should remain active")
            .title,
        "正文优先"
    );

    app.dispatch_action(NotoraAction::TitleCommitRequested("后改的 Notora 标题".to_owned()));
    let second_deadline = Instant::now() + Duration::from_secs(2);
    while app
        .document_runtime
        .pending_metadata_mutations
        .iter()
        .any(|mutation| matches!(mutation, MetadataMutation::SetTitle { .. }))
    {
        app.drain_product_events();
        assert!(Instant::now() < second_deadline, "independent title should persist");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("independent body should remain available")
            .text,
        "# 正文优先\n\n正文"
    );
}

#[test]
fn mindmap_title_commit_renames_the_existing_empty_root() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created mmap should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap source should exist")
            .text,
        "#"
    );

    app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));

    while app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created mmap source should remain available")
        .text
        == "#"
    {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "mmap title initialization should complete");
        thread::sleep(Duration::from_millis(10));
    }

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("renamed mmap source should remain available");
    assert_eq!(snapshot.text, "# 项目路线图");
    let tree = textora_mindmap::mmf::parser::parse(&snapshot.text)
        .expect("renamed mmap must retain one valid root");
    assert_eq!(tree.root.title, "项目路线图");
}

#[test]
fn tab_in_empty_markdown_body_immediately_moves_the_caret() {
    const TAB_SPACES: &str = "    ";
    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "tab-caret.md", "");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    let mut shaper = shaping::Shaper::new().expect("Tab caret test requires body fonts");
    let _ = app
        .document_runtime
        .editor_runtime
        .tab_session_mut(tab_id)
        .expect("empty body must retain its session")
        .render_plugin(
            ui::Rect::new(0.0, 0.0, 800.0, 600.0),
            &ui::theme::test_theme(),
            &mut shaper,
            1.0,
        );
    let before = app
        .document_runtime
        .editor_runtime
        .tab_session(tab_id)
        .and_then(|session| session.query_cursor_screen_rect(0))
        .expect("empty Markdown body must expose its caret");

    app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("indented body must remain available");
    assert_eq!(snapshot.text, TAB_SPACES);
    let session = app
        .document_runtime
        .editor_runtime
        .tab_session(tab_id)
        .expect("indented body must retain its session");
    assert_eq!(session.document().cursor_offset().to_usize(), TAB_SPACES.len());
    let after = session
        .query_cursor_screen_rect(TAB_SPACES.len())
        .expect("Tab must expose the updated caret before another paint");
    assert!(after.0 > before.0, "Tab must immediately advance the visual caret");
    assert_eq!(after.1, before.1, "Tab must keep the caret on its current row");
}

#[test]
fn tab_after_typing_into_an_empty_mindmap_root_creates_a_child() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created mmap should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.render().expect("created mmap should render its empty root");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    let Some(mut tab) = app.document_runtime.editor_runtime.tab_session_mut(tab_id) else {
        panic!("created mmap should keep its runtime session");
    };
    tab.document_mut().cursor_mut().selection_anchor = Some(0);
    tab.document_mut().cursor_move_to_offset(1);

    app.commit_editor_text("kk".to_owned());
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("typed mmap root should remain available")
            .text,
        "#kk"
    );

    let mapped_tab = appkit_shell::window_input::winit_key_to_keycode(
        &winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab),
        Some("\t"),
    )
    .expect("native Tab should map to an editor key");
    let tab_event = ui::Event::KeyDown(mapped_tab, ui::core::Modifiers::NONE);
    if !app.route_product_event(&tab_event) {
        app.handle_editor_key_input(mapped_tab, ui::core::Modifiers::NONE);
    }

    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("indented mmap root should remain available")
            .text,
        "#kk\n##\n"
    );
}

#[test]
fn tab_from_an_untitled_new_mindmap_root_creates_a_child_without_spaces() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created mmap should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.render().expect("created mmap should render its empty root editor");
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .tab_session(tab_id)
            .expect("created mmap should have a runtime session")
            .plugin_name(),
        ui::plugin::PLUGIN_MINDMAP
    );
    assert!(app.frame_runtime.shell.editor_title_text().is_empty());

    let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
    assert!(app.route_product_event(&tab_event));
    drain_until_document_text(&mut app, tab_id, "# 无标题", deadline);
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .tab_session(tab_id)
            .expect("created mmap should retain its runtime session")
            .document()
            .cursor_offset()
            .to_usize(),
        "# 无标题".len()
    );
    app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created mmap source should remain available");
    assert_eq!(snapshot.text, "# 无标题\n##\n");
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
}

#[test]
fn tab_from_a_new_mindmap_title_hands_input_to_the_canvas_and_creates_a_child() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created mmap should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.render().expect("created mmap should render its title editor");
    assert!(app.route_product_event(&ui::Event::ImeCommit("项目路线图".to_owned())));
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::EditorTitle);
    assert_eq!(app.frame_runtime.shell.editor_title_text(), "项目路线图");

    let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
    assert!(app.route_product_event(&tab_event));
    drain_until_document_text(&mut app, tab_id, "# 项目路线图", deadline);
    app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created mmap source should remain available");
    assert_eq!(snapshot.text, "# 项目路线图\n##\n");
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);

    app.commit_editor_text("子节点".to_owned());
    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created mmap child should remain available");
    assert_eq!(snapshot.text, "# 项目路线图\n##子节点\n");

    let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
    if !app.route_product_event(&tab_event) {
        app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
    }
    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created mmap grandchild should remain available");
    assert_eq!(snapshot.text, "# 项目路线图\n##子节点\n###\n");
}

#[test]
fn tab_from_a_new_markdown_title_focuses_the_empty_body_without_seeding_a_heading() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created markdown should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.render().expect("created markdown should render its title editor");
    assert!(app.route_product_event(&ui::Event::ImeCommit("项目记录".to_owned())));

    let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
    assert!(app.route_product_event(&tab_event));
    while !workspace_directory.path().join("项目记录.md").is_file() {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "title commit should rename the markdown file");
        thread::sleep(Duration::from_millis(10));
    }

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created markdown source should remain available");
    assert_eq!(snapshot.text, "");
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
}

#[test]
fn source_view_backspace_and_delete_edit_graphemes_through_product_routing() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created mmap should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));
    drain_until_document_text(&mut app, tab_id, "# 项目路线图", deadline);
    app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .tab_session(tab_id)
            .expect("source view should keep the runtime session")
            .plugin_name(),
        ui::plugin::PLUGIN_EDITOR
    );

    let backspace_event = ui::Event::KeyDown(ui::KeyCode::Backspace, ui::core::Modifiers::NONE);
    if !app.route_product_event(&backspace_event) {
        app.handle_editor_key_input(ui::KeyCode::Backspace, ui::core::Modifiers::NONE);
    }
    app.document_runtime
        .editor_runtime
        .tab_session_mut(tab_id)
        .expect("source view should keep the mutable runtime session")
        .document_mut()
        .cursor_move_to_offset("# ".len());
    let delete_event = ui::Event::KeyDown(ui::KeyCode::Delete, ui::core::Modifiers::NONE);
    if !app.route_product_event(&delete_event) {
        app.handle_editor_key_input(ui::KeyCode::Delete, ui::core::Modifiers::NONE);
    }

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("source text should remain available");
    assert_eq!(snapshot.text, "# 目路线");
}

#[test]
fn markdown_and_mindmap_toolbar_actions_toggle_real_source_views() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    let mut previous_identity = None;
    for (kind, visual_plugin) in [
        (DocumentKind::Markdown, ui::plugin::PLUGIN_MARKDOWN_EDITOR),
        (DocumentKind::Mindmap, ui::plugin::PLUGIN_MINDMAP),
    ] {
        app.dispatch_action(NotoraAction::CreateRequested(kind));
        let deadline = Instant::now() + Duration::from_secs(2);
        let (identity, tab_id) = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && Some(identity) != previous_identity
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break (identity, tab_id);
            }
            assert!(Instant::now() < deadline, "created note should install its visual view");
            thread::sleep(Duration::from_millis(10));
        };
        previous_identity = Some(identity);
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("created note should have a runtime session")
                .plugin_name(),
            visual_plugin
        );

        app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("source view should keep the runtime session")
                .plugin_name(),
            ui::plugin::PLUGIN_EDITOR
        );

        app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("visual view should be restorable")
                .plugin_name(),
            visual_plugin
        );
    }
}

#[test]
fn inspector_selection_writes_one_revision_and_schedules_autosave() {
    use ui::inspector_panel::{InspectorPanelIntent, ItemId};
    use ui::plugin::SemanticEditCommand;

    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created mmap should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };

    app.document_runtime.set_autosave_idle_delay(Duration::ZERO);
    app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
    app.render().expect("open inspector should render");
    let panel = app
        .document_runtime
        .editor()
        .active_inspector_panel()
        .expect("inspector should be mounted");
    let original = app
        .document_runtime
        .editor()
        .document_text_snapshot(tab_id)
        .expect("original source should exist");
    assert_eq!(app.document_runtime.autosave_state(tab_id), None);

    let select_tide = inspector_selection_action(&app, &panel, "tide");
    let stale_select_dawn = inspector_selection_action(&app, &panel, "dawn");
    app.dispatch_action(select_tide);
    let themed = app
        .document_runtime
        .editor()
        .document_text_snapshot(tab_id)
        .expect("themed source should exist");
    assert!(themed.text.contains("theme = \"tide\""));
    assert_eq!(themed.content_revision, original.content_revision + 1);
    let scheduled = app.document_runtime.autosave_state(tab_id);
    assert!(matches!(
        scheduled,
        Some(AutoSaveState::Scheduled { content_revision, .. })
            if content_revision == themed.content_revision
    ));

    app.dispatch_action(stale_select_dawn);
    app.render().expect("themed inspector should render");
    let panel = app
        .document_runtime
        .editor()
        .active_inspector_panel()
        .expect("inspector should remain mounted");
    assert_eq!(panel.descriptor.presets.selected_item, Some(ItemId("tide".into())));
    let select_current = inspector_selection_action(&app, &panel, "tide");
    app.dispatch_action(select_current);
    assert_eq!(app.document_runtime.editor().document_text_snapshot(tab_id), Some(themed.clone()));
    assert_eq!(app.document_runtime.autosave_state(tab_id), scheduled);
    assert_eq!(
        app.document_runtime.take_due_autosaves(),
        vec![AutoSaveRequest { tab_id, content_revision: themed.content_revision }]
    );
    assert!(app.document_runtime.take_due_autosaves().is_empty());

    app.dispatch_action(NotoraAction::SemanticEditRequested(SemanticEditCommand::Undo));
    let restored = app
        .document_runtime
        .editor()
        .document_text_snapshot(tab_id)
        .expect("restored source should exist");
    assert_eq!(restored.text, original.text);
    assert_eq!(restored.content_revision, original.content_revision + 2);

    app.render().expect("restored inspector should render");
    let restored_panel = app
        .document_runtime
        .editor()
        .active_inspector_panel()
        .expect("restored inspector should remain mounted");
    let close = inspector_action(&app, &restored_panel, InspectorPanelIntent::Close);
    app.dispatch_action(close);
    assert!(app.document_runtime.editor().active_inspector_panel().is_none());
}

fn inspector_selection_action(
    app: &NotoraRuntime,
    panel: &appkit_shell::inspector_panel::MountedInspectorPanel,
    item: &str,
) -> NotoraAction {
    use ui::inspector_panel::{InspectorItemTarget, InspectorPanelIntent, ItemId};

    inspector_action(
        app,
        panel,
        InspectorPanelIntent::SelectItem(InspectorItemTarget {
            group_id: panel.descriptor.presets.id.clone(),
            item_id: ItemId(item.into()),
        }),
    )
}

fn inspector_action(
    app: &NotoraRuntime,
    panel: &appkit_shell::inspector_panel::MountedInspectorPanel,
    intent: ui::inspector_panel::InspectorPanelIntent,
) -> NotoraAction {
    app.frame_runtime
        .shell
        .translate_widget_action(&ui::WidgetAction::InspectorPanel(
            ui::inspector_panel::InspectorPanelAction {
                panel_id: panel.descriptor.id.clone(),
                intent,
            },
        ))
        .expect("the rendered inspector should stamp its widget action")
}

#[test]
fn inspector_reports_metadata_without_writing_rejected_actions() {
    use ui::inspector_panel::ItemId;
    use ui::theme::DEFAULT_MINDMAP_COLOR_SCHEME_ID;

    let default_item = ItemId(DEFAULT_MINDMAP_COLOR_SCHEME_ID.into());
    for (source, selection, selection_disabled, notice_fragment, rejected_item) in [
        ("# Diagram\n", Some(default_item.clone()), false, None, DEFAULT_MINDMAP_COLOR_SCHEME_ID),
        (
            "```toml mindmap\ntheme = \"some-old-theme\"\n```\n# Diagram\n",
            None,
            false,
            Some("some-old-theme"),
            "unknown-action",
        ),
        ("```toml mindmap\ntheme = [\n```\n# Diagram\n", None, true, Some("元数据"), "tide"),
    ] {
        let mut app = app();
        let (_, tab_id) = install_registered_note(&mut app, "diagram.mmap.md", source);
        let original = app
            .document_runtime
            .editor()
            .document_text_snapshot(tab_id)
            .expect("source fixture should exist");
        app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
        app.render().expect("inspector should render");
        let panel = app
            .document_runtime
            .editor()
            .active_inspector_panel()
            .expect("mindmap supports the inspector");
        assert_eq!(panel.descriptor.presets.selected_item, selection);
        assert_eq!(panel.descriptor.presets.fallback_summary_item, Some(default_item.clone()));
        assert_eq!(panel.descriptor.selection_disabled, selection_disabled);
        assert_eq!(panel.descriptor.notice.is_some(), notice_fragment.is_some());
        if let Some(notice_fragment) = notice_fragment {
            assert!(
                panel
                    .descriptor
                    .notice
                    .as_deref()
                    .is_some_and(|notice| notice.contains(notice_fragment))
            );
        }

        let rejected_selection = inspector_selection_action(&app, &panel, rejected_item);
        app.dispatch_action(rejected_selection);
        assert_eq!(app.document_runtime.editor().document_text_snapshot(tab_id), Some(original));
        assert!(
            !app.document_runtime
                .editor()
                .document_summary(tab_id)
                .expect("document should be open")
                .dirty
        );
        assert_eq!(app.document_runtime.autosave_state(tab_id), None);
    }
}

#[test]
fn inspector_can_replace_an_unknown_theme_with_the_default_and_undo_it() {
    use ui::plugin::SemanticEditCommand;
    use ui::theme::DEFAULT_MINDMAP_COLOR_SCHEME_ID;

    const ORIGINAL: &str = "```toml mindmap\ntheme = \"legacy-scheme\" # keep\n```\n# Diagram\n";
    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "diagram.mmap.md", ORIGINAL);
    app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
    app.render().expect("inspector should render unknown theme fallback");
    let panel =
        app.document_runtime.editor().active_inspector_panel().expect("inspector should mount");
    assert!(panel.descriptor.presets.selected_item.is_none());
    let select_default = inspector_selection_action(&app, &panel, DEFAULT_MINDMAP_COLOR_SCHEME_ID);
    app.dispatch_action(select_default);

    let themed = app
        .document_runtime
        .editor()
        .document_text_snapshot(tab_id)
        .expect("themed source should exist");
    assert_eq!(themed.text, ORIGINAL.replace("legacy-scheme", DEFAULT_MINDMAP_COLOR_SCHEME_ID));

    app.dispatch_action(NotoraAction::SemanticEditRequested(SemanticEditCommand::Undo));
    let restored = app
        .document_runtime
        .editor()
        .document_text_snapshot(tab_id)
        .expect("restored source should exist");
    assert_eq!(restored.text, ORIGINAL);
}

#[test]
fn inspector_keyboard_focus_and_ime_follow_the_active_tab() {
    let mut app = app();
    let (first_identity, first_tab_id) =
        install_registered_note(&mut app, "first.mmap.md", "# First\n");
    app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
    app.render().expect("first inspector should render");
    let first_panel = app
        .document_runtime
        .editor()
        .active_inspector_panel()
        .expect("first inspector should mount");
    let replay = inspector_selection_action(&app, &first_panel, "tide");
    let first_source = app.document_runtime.editor().document_text_snapshot(first_tab_id);

    assert!(
        app.route_product_event(&ui::Event::KeyDown(ui::KeyCode::Right, ui::Modifiers::default(),))
    );
    let focused_item = app
        .document_runtime
        .editor()
        .active_inspector_panel()
        .expect("first inspector should remain open")
        .presentation
        .focused_item;
    assert!(focused_item.is_some());
    for event in [
        ui::Event::ImeEnable,
        ui::Event::ImePreedit { text: "输入法".into(), cursor: None },
        ui::Event::ImeCommit("输入法".into()),
        ui::Event::ImeDisable,
        ui::Event::KeyDown(ui::KeyCode::Char('k'), ui::Modifiers::default()),
    ] {
        assert!(app.route_product_event(&event), "inspector should consume {event:?}");
    }
    assert_eq!(app.document_runtime.editor().document_text_snapshot(first_tab_id), first_source);

    let (_, second_tab_id) = install_registered_note(&mut app, "second.mmap.md", "# Second\n");
    assert!(app.document_runtime.editor().active_inspector_panel().is_none());
    app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
    app.render().expect("second inspector should render");
    let second_source = app.document_runtime.editor().document_text_snapshot(second_tab_id);
    app.dispatch_action(replay.clone());
    assert_eq!(app.document_runtime.editor().document_text_snapshot(second_tab_id), second_source);
    assert_eq!(
        app.document_runtime
            .editor()
            .active_inspector_panel()
            .expect("second inspector should mount")
            .presentation
            .focused_item,
        None
    );

    app.action_runtime.state.library.selected_card = Some(first_identity);
    let outcome = app.document_runtime.editor_mut().activate(first_tab_id);
    app.apply_editor_outcome(outcome);
    app.render().expect("first inspector should remount");
    app.dispatch_action(replay);
    assert_eq!(app.document_runtime.editor().document_text_snapshot(first_tab_id), first_source);
    assert_eq!(
        app.document_runtime
            .editor()
            .active_inspector_panel()
            .expect("first inspector should remount")
            .presentation
            .focused_item,
        focused_item
    );

    let escape = ui::Event::KeyDown(ui::KeyCode::Escape, ui::Modifiers::NONE);
    app.dispatch_action(NotoraAction::OpenSettings);
    assert!(app.route_product_event(&escape));
    assert_eq!(app.state().layout.overlay, OverlayState::None);
    assert!(app.document_runtime.editor().active_inspector_panel().is_some());
    assert!(app.route_product_event(&escape));
    assert!(app.document_runtime.editor().active_inspector_panel().is_none());
}

#[test]
fn inspector_rejects_replayed_actions_after_switching_to_plain_document() {
    let mut app = app();
    let (_, mindmap_tab_id) = install_registered_note(&mut app, "diagram.mmap.md", "# Diagram\n");
    app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
    app.render().expect("mindmap inspector should render");
    let panel = app
        .document_runtime
        .editor()
        .active_inspector_panel()
        .expect("mindmap inspector should mount");
    let replay = inspector_selection_action(&app, &panel, "tide");
    let mindmap_original = app.document_runtime.editor().document_text_snapshot(mindmap_tab_id);

    let (_, markdown_tab_id) = install_registered_note(&mut app, "plain.md", "plain document");
    let markdown_original = app.document_runtime.editor().document_text_snapshot(markdown_tab_id);
    app.dispatch_action(NotoraAction::ToggleInspectorPanelRequested);
    app.dispatch_action(replay);
    app.render().expect("plain document should render");
    assert!(!app.document_runtime.editor().active_inspector_available());
    assert!(app.document_runtime.editor().active_inspector_panel().is_none());
    assert_eq!(
        app.frame_runtime.shell.translate_widget_action(&ui::WidgetAction::InspectorPanel(
            ui::inspector_panel::InspectorPanelAction {
                panel_id: panel.descriptor.id,
                intent: ui::inspector_panel::InspectorPanelIntent::Close,
            }
        )),
        None
    );
    assert_eq!(
        app.document_runtime.editor().document_text_snapshot(mindmap_tab_id),
        mindmap_original
    );
    assert_eq!(
        app.document_runtime.editor().document_text_snapshot(markdown_tab_id),
        markdown_original
    );
    assert_eq!(app.document_runtime.autosave_state(mindmap_tab_id), None);
    assert_eq!(app.document_runtime.autosave_state(markdown_tab_id), None);
}

#[test]
fn title_text_change_does_not_update_the_active_note_before_explicit_commit() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card {
            break identity;
        }
        assert!(Instant::now() < deadline, "note completion should select a note");
        thread::sleep(Duration::from_millis(10));
    };
    let tab_id = loop {
        app.drain_product_events();
        if let Some(tab_id) = app.document_tab_for(identity) {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "created note should have an editor tab");
        thread::sleep(Duration::from_millis(10));
    };

    app.dispatch_action(NotoraAction::TitleTextChanged("项目路线图".to_owned()));

    let snapshot = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("created note text should remain available");
    assert_eq!(snapshot.text, "");
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::EditorTitle);
}

#[test]
fn completed_tag_mutation_refreshes_active_editor_chips() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");

    app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    let note_id = loop {
        app.drain_product_events();
        if let Some(DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && app.action_runtime.state().library.active_editor_metadata.is_some()
        {
            break note_id;
        }
        assert!(Instant::now() < deadline, "created note metadata should load");
        thread::sleep(Duration::from_millis(10));
    };

    app.dispatch_action(NotoraAction::MetadataMutationRequested(
        MetadataMutation::AttachTagByName { note_id, display_name: "产品/Notora".to_owned() },
    ));

    loop {
        app.drain_product_events();
        let labels = app
            .state()
            .library
            .active_editor_metadata
            .as_ref()
            .map(|snapshot| {
                snapshot.tags.iter().map(|tag| tag.display_name.as_str()).collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if labels == ["产品/Notora"] {
            break;
        }
        assert!(Instant::now() < deadline, "completed tag mutation should refresh chips");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn new_note_button_is_disabled_until_a_workspace_root_is_selected() {
    let mut app = app();
    app.render().expect("headless shell frame should render");
    let layout = app.shell_layout();
    let click_x = layout.card_list_rect.right() - 76.0 * layout.dpi;
    let click_y = 22.0 * layout.dpi;

    app.route_product_event(&ui::Event::MouseDown {
        px: click_x,
        py: click_y,
        button: ui::core::widget::MouseButton::Left,
    });
    app.route_product_event(&ui::Event::MouseUp {
        px: click_x,
        py: click_y,
        button: ui::core::widget::MouseButton::Left,
    });

    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
    assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationTree);
}

#[test]
fn workspace_root_selection_is_completed_before_note_creation_becomes_available() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let selected_workspace_root = workspace_directory.path().to_path_buf();
    let mut app = app();
    app.workspace_directory_chooser = Box::new(move || Some(selected_workspace_root.clone()));

    app.dispatch_action(NotoraAction::WorkspaceRootSelectionRequested);

    assert_eq!(app.action_runtime.state().workspace_root, WorkspaceRootState::Active);
    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);

    app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
    assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::NewDocumentMenu);
}

#[test]
fn completed_note_creation_is_persisted_and_restored_after_restart() {
    let configuration_directory =
        tempfile::tempdir().expect("configuration fixture should be created");
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let paths = NotoraPaths::from_config_directory(configuration_directory.path().join("notora"))
        .expect("isolated product paths should be created");
    let mut first_app =
        NotoraRuntime::with_paths(paths.clone()).expect("first app should construct");
    let selected_workspace_root = workspace_directory.path().to_path_buf();
    first_app.workspace_directory_chooser = Box::new(move || Some(selected_workspace_root.clone()));

    assert!(first_app.select_workspace_root());
    first_app.persistence_runtime.session_persist_at = Some(Instant::now());
    first_app.process_due_session_persistence();

    let workspace_session_deadline = Instant::now() + Duration::from_secs(2);
    while crate::session::load_product_session(&paths.session_file).session.workspace_root.is_none()
    {
        assert!(
            Instant::now() < workspace_session_deadline,
            "selected workspace should be persisted before note completion"
        );
        thread::sleep(Duration::from_millis(10));
    }

    first_app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
    let creation_deadline = Instant::now() + Duration::from_secs(2);
    let note_id = loop {
        first_app.drain_product_events();
        if let Some(DocumentIdentity::Note(note_id)) =
            first_app.action_runtime.state().library.selected_card
            && first_app.document_tab_for(DocumentIdentity::Note(note_id)).is_some()
        {
            break note_id;
        }
        assert!(Instant::now() < creation_deadline, "created note should load promptly");
        thread::sleep(Duration::from_millis(10));
    };

    assert!(
        first_app.persistence_runtime.session_persist_at.is_some(),
        "note completion should schedule persistence for the new last document"
    );
    assert_eq!(
        first_app.capture_product_session().last_document,
        Some(crate::session::SavedDocument::Note { note_id })
    );
    first_app.persistence_runtime.session_persist_at = Some(Instant::now());
    first_app.process_due_session_persistence();
    first_app.persistence_runtime.worker.shutdown();
    first_app.drain_product_events();
    assert_eq!(first_app.action_runtime.state().library.last_command_error, None);
    drop(first_app);
    let note_session_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let loaded_session = crate::session::load_product_session(&paths.session_file);
        if loaded_session.session.last_document
            == Some(crate::session::SavedDocument::Note { note_id })
        {
            break;
        }
        assert!(
            Instant::now() < note_session_deadline,
            "created note should become the persisted last document: {loaded_session:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let mut restarted_app =
        NotoraRuntime::with_paths(paths).expect("restarted app should construct");
    restarted_app.restore_pending_session();
    let restore_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        restarted_app.drain_product_events();
        let identity = DocumentIdentity::Note(note_id);
        if card_page_contains_note(&restarted_app.action_runtime.state().library.card_page, note_id)
            && restarted_app.document_tab_for(identity).is_some()
        {
            break;
        }
        assert!(
            Instant::now() < restore_deadline,
            "restarted app should restore the new note in the workspace"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(restarted_app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
    assert_eq!(restarted_app.action_runtime.state().layout.compact_content, CompactContent::Editor);
    assert!(matches!(
        restarted_app.render_frame().expect("restored note frame should compose"),
        appkit_shell::editor_runtime::EditorSurfacePaint::Document { .. }
    ));
}

#[test]
fn dirty_note_is_not_moved_to_trash_when_its_required_save_fails() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let note_id = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)).is_some()
        {
            break note_id;
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    let identity = notora_core::DocumentIdentity::Note(note_id);
    let tab_id = app.document_tab_for(identity).expect("note should have an open tab");
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.commit_editor_text("unsaved edit".to_owned());
    assert!(
        app.document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .expect("tab should remain available")
            .dirty
    );

    app.dispatch_action(NotoraAction::TrashOperationRequested(
        crate::action::TrashOperation::MoveToTrash { note_id },
    ));

    assert!(workspace_directory.path().join("无标题.md").is_file());
    assert_eq!(app.document_tab_for(identity), Some(tab_id));
    assert_eq!(app.action_runtime.state().library.selected_card, Some(identity));
    assert!(app.document_runtime.pending_trash_moves.is_empty());
    assert!(
        app.action_runtime
            .state()
            .library
            .last_command_error
            .as_deref()
            .is_some_and(|message| message.contains("未移入回收站"))
    );
}

#[test]
fn dirty_note_move_waits_for_save_before_worker_command() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let archive_directory = workspace_directory.path().join("archive");
    std::fs::create_dir(&archive_directory).expect("move target directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let (note_id, tab_id, original_path) = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
            && let Some(path) = app
                .document_runtime
                .editor_runtime
                .document_summary(tab_id)
                .and_then(|summary| summary.path)
        {
            break (note_id, tab_id, path);
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.commit_editor_text("unsaved edit".to_owned());
    assert!(
        app.document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .expect("note tab should remain available")
            .dirty
    );

    app.dispatch_action(NotoraAction::MoveRequested {
        note_id,
        target_directory: std::path::PathBuf::from("archive"),
    });

    let moved_path = archive_directory
        .join(original_path.file_name().expect("created note should have a file name"));
    let move_check_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < move_check_deadline && original_path.is_file() && !moved_path.is_file() {
        app.drain_product_events();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(original_path.is_file());
    assert!(!moved_path.is_file());
    assert!(
        app.action_runtime
            .state()
            .library
            .last_command_error
            .as_deref()
            .is_some_and(|message| message.contains("未移动"))
    );
}

#[test]
fn pending_trash_move_rejects_a_document_changed_after_its_save_started() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let (note_id, tab_id, saved_revision) = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
            && let Some(summary) = app.document_runtime.editor_runtime.document_summary(tab_id)
        {
            break (note_id, tab_id, summary.content_revision);
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    let pending_move = PendingTrashMove { note_id, content_revision: saved_revision };
    assert!(app.pending_trash_move_has_current_saved_document(tab_id, pending_move));

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.commit_editor_text("newer edit".to_owned());

    assert!(!app.pending_trash_move_has_current_saved_document(tab_id, pending_move));
}

#[test]
fn pending_note_move_rejects_a_document_changed_after_its_save_started() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let (note_id, tab_id, saved_revision) = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
            && let Some(summary) = app.document_runtime.editor_runtime.document_summary(tab_id)
        {
            break (note_id, tab_id, summary.content_revision);
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    let pending_move = PendingNoteMove {
        request: notora_core::note_command::MoveNoteRequest {
            note_id,
            target_directory: std::path::PathBuf::from("archive"),
        },
        content_revision: saved_revision,
    };
    assert!(app.pending_note_move_has_current_saved_document(tab_id, &pending_move));

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.commit_editor_text("newer edit".to_owned());

    assert!(!app.pending_note_move_has_current_saved_document(tab_id, &pending_move));
}

#[test]
fn pending_title_update_rejects_a_document_changed_after_its_save_started() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let (note_id, tab_id, saved_revision) = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
            && let Some(summary) = app.document_runtime.editor_runtime.document_summary(tab_id)
        {
            break (note_id, tab_id, summary.content_revision);
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    let pending_update = PendingTitleUpdate {
        request: notora_core::UpdateNoteTitleRequest {
            note_id,
            expected_title_revision: 0,
            title: "新标题".to_owned(),
        },
        content_revision: saved_revision,
    };
    assert!(app.pending_title_update_has_current_saved_document(tab_id, &pending_update));

    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.commit_editor_text("newer edit".to_owned());

    assert!(!app.pending_title_update_has_current_saved_document(tab_id, &pending_update));
}

#[test]
fn redundant_autosave_dispatches_pending_title_update_for_saved_revision() {
    let mut app = app();
    let (identity, tab_id) = install_registered_note(&mut app, "note.md", "已保存正文");
    let DocumentIdentity::Note(note_id) = identity else {
        panic!("registered note should have a workspace identity");
    };
    let summary = app
        .document_runtime
        .editor_runtime
        .document_summary(tab_id)
        .expect("registered note should have a document summary");
    assert!(!summary.dirty);
    let content_revision = summary.content_revision;
    let title_request = notora_core::UpdateNoteTitleRequest {
        note_id,
        expected_title_revision: 0,
        title: "新标题".to_owned(),
    };
    app.document_runtime
        .pending_title_updates
        .insert(tab_id, PendingTitleUpdate { request: title_request.clone(), content_revision });

    let outcome =
        app.document_runtime.submit_autosave(AutoSaveRequest { tab_id, content_revision }, None);

    assert!(matches!(
        outcome.commands.as_slice(),
        [DocumentCommand::ExecuteNote(notora_core::NoteCommand::UpdateTitle(request))]
            if request == &title_request
    ));
    assert!(!app.document_runtime.pending_title_updates.contains_key(&tab_id));
}

#[test]
fn clean_note_moves_to_trash_and_closes_its_runtime_after_the_worker_confirms() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let note_id = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)).is_some()
        {
            break note_id;
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    let identity = notora_core::DocumentIdentity::Note(note_id);
    app.dispatch_action(NotoraAction::TrashOperationRequested(
        crate::action::TrashOperation::MoveToTrash { note_id },
    ));

    loop {
        app.drain_product_events();
        if app.document_tab_for(identity).is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "trash completion should close the matching tab");
        thread::sleep(Duration::from_millis(10));
    }

    assert!(!workspace_directory.path().join("无标题.md").exists());
    assert_eq!(app.action_runtime.state().library.selected_card, None);
    assert_eq!(app.editor_runtime_tab_count(), 0);
    assert_eq!(app.action_runtime.state().library.last_command_error, None);
}

#[test]
fn selecting_a_trashed_note_loads_a_read_only_runtime_document() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let note_id = loop {
        app.drain_product_events();
        if let Some(DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && app.document_tab_for(DocumentIdentity::Note(note_id)).is_some()
        {
            break note_id;
        }
        assert!(Instant::now() < deadline, "created note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };
    let identity = DocumentIdentity::Note(note_id);
    app.dispatch_action(NotoraAction::TrashOperationRequested(
        crate::action::TrashOperation::MoveToTrash { note_id },
    ));
    loop {
        app.drain_product_events();
        if app.document_tab_for(identity).is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "trash move should close the editable tab");
        thread::sleep(Duration::from_millis(10));
    }

    app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Trash));
    loop {
        app.drain_product_events();
        if matches!(
            &app.action_runtime.state().library.card_page,
            CardPageState::Ready { cards, .. }
                if cards.iter().any(|card| card.note_id == note_id)
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "trash card should load promptly");
        thread::sleep(Duration::from_millis(10));
    }
    app.dispatch_action(NotoraAction::CardSelected(identity));
    let tab_id = loop {
        app.drain_product_events();
        if let Some(tab_id) = app.document_tab_for(identity) {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "trash document should install promptly");
        thread::sleep(Duration::from_millis(10));
    };
    let before = app
        .document_runtime
        .editor_runtime
        .document_text_snapshot(tab_id)
        .expect("trash document text should be available");

    assert!(
        !app.document_runtime
            .editor_runtime
            .workspace_snapshot()
            .tabs
            .iter()
            .find(|tab| tab.tab_id == tab_id)
            .expect("trash runtime tab should exist")
            .allows_editing
    );
    app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    app.commit_editor_text("禁止写入".to_owned());
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("trash document text should remain available")
            .text,
        before.text
    );

    app.dispatch_action(NotoraAction::TrashOperationRequested(
        crate::action::TrashOperation::Restore { note_id },
    ));
    loop {
        app.drain_product_events();
        if app.document_tab_for(identity).is_none()
            && app.action_runtime.state().library.selected_card.is_none()
            && workspace_directory.path().join("无标题.md").is_file()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "restore should close the read-only tab and clear its selection"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn selected_note_is_loaded_by_the_worker_and_installed_as_a_preview_tab() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let selected_identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && app.document_tab_for(identity).is_some()
        {
            break identity;
        }
        assert!(Instant::now() < deadline, "selected note should install a preview tab");
        thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(app.editor_runtime_tab_count(), 1);
    assert!(app.document_tab_for(selected_identity).is_some());
    assert_eq!(app.action_runtime.state().library.last_command_error, None);
}

#[test]
fn external_note_relocation_rebinds_the_open_runtime_and_refreshes_title_revision() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));
    let deadline = Instant::now() + Duration::from_secs(3);
    let (note_id, tab_id) = loop {
        app.drain_product_events();
        if let Some(notora_core::DocumentIdentity::Note(note_id)) =
            app.action_runtime.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
            && app.action_runtime.state().library.active_editor_metadata.is_some()
        {
            break (note_id, tab_id);
        }
        assert!(Instant::now() < deadline, "created note should install a preview");
        thread::sleep(Duration::from_millis(10));
    };
    let original_path = workspace_directory.path().join("无标题.md");
    let finder_path = workspace_directory.path().join("Finder 改名.md");
    std::fs::rename(&original_path, &finder_path).expect("Finder rename should succeed");
    let active_metadata = app
        .state()
        .library
        .active_editor_metadata
        .as_ref()
        .expect("active metadata should be loaded");
    let mut relocated_metadata = active_metadata.metadata.clone();
    relocated_metadata.title_revision = 1;
    app.synchronize_external_note_relocations(vec![crate::product::WorkspaceNoteRelocation {
        note_id,
        from: "无标题.md".into(),
        to: "Finder 改名.md".into(),
        metadata: relocated_metadata,
        tags: active_metadata.tags.clone(),
    }]);

    assert_eq!(app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)), Some(tab_id));
    assert!(!original_path.exists());
    assert!(finder_path.is_file());
    assert_eq!(
        app.action_runtime
            .state()
            .library
            .active_editor_metadata
            .as_ref()
            .map(|snapshot| snapshot.metadata.title_revision),
        Some(1)
    );
}

#[test]
fn promoting_a_preview_preserves_its_tab_when_the_next_preview_opens() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let first_identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && app.document_tab_for(identity).is_some()
        {
            break identity;
        }
        assert!(Instant::now() < deadline, "first preview should install");
        thread::sleep(Duration::from_millis(10));
    };
    app.request_preview_promotion();
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Text));

    loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && identity != first_identity
            && app.document_tab_for(identity).is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "second preview should install");
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.editor_runtime_tab_count(), 2);
    assert!(app.document_tab_for(first_identity).is_some());
}

#[test]
fn first_content_change_promotes_the_active_preview_before_the_next_preview_opens() {
    let workspace_directory =
        tempfile::tempdir().expect("workspace test directory should be created");
    let mut app = app();
    app.execute_workspace_command(WorkspaceCommand::OpenExisting {
        root: workspace_directory.path().to_path_buf(),
    })
    .expect("workspace should open");
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

    let deadline = Instant::now() + Duration::from_secs(2);
    let first_identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && app.document_tab_for(identity).is_some()
        {
            break identity;
        }
        assert!(Instant::now() < deadline, "first preview should install");
        thread::sleep(Duration::from_millis(10));
    };
    let first_tab_id =
        app.document_tab_for(first_identity).expect("first preview should have a registered tab");
    app.handle_editor_notification(
        &appkit_shell::editor_runtime::EditorNotification::ContentChanged {
            tab_id: first_tab_id,
            content_revision: 1,
        },
    );
    assert!(matches!(
        app.document_runtime.autosave.state(first_tab_id),
        Some(AutoSaveState::Scheduled { content_revision: 1, .. })
    ));
    app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Text));

    loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && identity != first_identity
            && app.document_tab_for(identity).is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "second preview should install");
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.editor_runtime_tab_count(), 2);
    assert!(app.document_tab_for(first_identity).is_some());
}

#[test]
fn open_external_path_switches_to_files_and_reuses_the_existing_tab() {
    let directory = tempfile::tempdir().expect("external file fixture directory should exist");
    let path = directory.path().join("outside.md");
    std::fs::write(&path, "# Outside").expect("external file fixture should be written");
    let mut app = app();

    app.receive_system_open_paths(vec![path.clone()]);
    app.receive_system_open_paths(vec![path]);
    assert_eq!(
        app.action_runtime.state().library.selected_card,
        None,
        "external path validation and reads must not complete on the caller thread"
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && app.document_tab_for(identity).is_some()
        {
            break identity;
        }
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        app.action_runtime.state().library.navigation_scope,
        notora_core::NavigationScope::ExternalFiles
    );
    assert!(matches!(identity, notora_core::DocumentIdentity::ExternalFile(_)));
    assert_eq!(app.editor_runtime_tab_count(), 1);
    assert!(app.document_tab_for(identity).is_some());
    assert_eq!(app.action_runtime.state().external_files.sessions().len(), 1);
    assert_eq!(app.action_runtime.state().library.last_command_error, None);
}

fn open_plain_text_fixture(
    directory: &std::path::Path,
    contents: &str,
) -> (NotoraRuntime, appkit_core::workspace::types::TabId, std::path::PathBuf) {
    let path = directory.join("outside.txt");
    std::fs::write(&path, contents).expect("external text fixture should be written");
    let paths = NotoraPaths::from_config_directory(directory.join("notora"))
        .expect("test should create isolated product paths");
    let mut app = NotoraRuntime::with_paths(paths).expect("test runtime should initialize");
    app.receive_system_open_paths(vec![path.clone()]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let tab_id = loop {
        app.drain_product_events();
        if let Some(identity) = app.state().library.selected_card
            && let Some(tab_id) = app.document_tab_for(identity)
        {
            break tab_id;
        }
        assert!(Instant::now() < deadline, "external text fixture should open promptly");
        thread::sleep(Duration::from_millis(10));
    };
    (app, tab_id, path)
}

#[test]
fn first_editor_drag_from_card_list_depends_on_window_focus() {
    let directory = tempfile::tempdir().expect("external fixture directory should exist");
    let (mut app, tab_id, _) = open_plain_text_fixture(directory.path(), "abcdefghij\n");
    app.set_window_size(1600, 1000);
    app.render().expect("headless editor frame should record its bounds");
    let bounds = app.shell_layout().editor_body_rect;
    let editor = app.editor_runtime_mut();
    let settings = editor.settings_snapshot();
    let dpi = editor.scale_factor() as f32;
    let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
    let gutter = metrics.content_left_margin.max(settings.gutter_width(2) * dpi);
    let row_left = bounds.x + gutter;
    let glyph_width = 24.0;
    let mut tab = editor.tab_session_mut(tab_id).expect("external text fixture should have a tab");
    let original_cursor = tab.document().cursor_offset().to_usize();
    tab.display_mut().advance_cache = vec![ui::render_geom::AdvanceCacheEntry {
        doc_line: 0,
        vl_byte_start: 0,
        vl_grapheme_start: 0,
        clusters: (1..=10)
            .map(|end| (end, row_left + end as f32 * glyph_width, (end - 1) as u32))
            .collect(),
    }];

    let start_x = row_left + 4.0 * glyph_width;
    let end_x = row_left + 8.0 * glyph_width;
    let row_y = bounds.y + metrics.line_height * 0.5;
    let button = ui::core::MouseButton::Left;
    for (window_focused, expected_position) in
        [(false, (None, original_cursor)), (true, (Some(4), 8))]
    {
        app.set_window_focused(window_focused);
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
        assert!(!app.route_pointer_event(&ui::Event::MouseDown { px: start_x, py: row_y, button }));
        assert_eq!(app.state().layout.focus_target, FocusTarget::Editor);
        assert_eq!(app.editor_pointer_is_captured(), window_focused);
        app.route_pointer_event(&ui::Event::MouseMove { px: end_x, py: row_y });
        app.route_pointer_event(&ui::Event::MouseUp { px: end_x, py: row_y, button });

        let editor = app.document_runtime.editor();
        let tab = editor.tab_session(tab_id).expect("external text fixture should stay open");
        assert_eq!(
            (tab.document().cursor().selection_anchor, tab.document().cursor_offset().to_usize()),
            expected_position
        );
        assert!(!app.editor_pointer_is_captured());
    }
}

#[test]
fn shutting_down_with_dirty_external_text_preserves_source_and_writes_recovery() {
    let directory = tempfile::tempdir().expect("external fixture directory should exist");
    let original = "abcdefghij\n";
    let (mut app, tab_id, path) = open_plain_text_fixture(directory.path(), original);
    let revision = app
        .editor_runtime_mut()
        .document_text_snapshot(tab_id)
        .expect("external text fixture should provide its source")
        .content_revision;
    app.editor_runtime_mut()
        .replace_document_text(DocumentTextReplacement {
            tab_id,
            content_revision: revision,
            range: 0..1,
            replacement: "changed-".to_owned(),
        })
        .expect("external document should accept an unsaved edit");

    app.shutdown();

    assert_eq!(std::fs::read_to_string(&path).expect("external source should exist"), original);
    let snapshots =
        app.recoverable_dirty_snapshots().expect("recovery directory should be readable");
    assert!(
        matches!(
            snapshots.as_slice(),
            [crate::dirty_snapshot::RecoverableDirtySnapshot::Ready { content_lines, .. }]
                if content_lines.join("\n") == "changed-bcdefghij\n"
        ),
        "shutdown should preserve the edited text in exactly one recoverable snapshot: {snapshots:?}"
    );
}

#[test]
fn closing_a_clean_external_file_removes_its_card_record_and_runtime_tab() {
    let directory = tempfile::tempdir().expect("external file fixture directory should exist");
    let path = directory.path().join("close-clean.md");
    std::fs::write(&path, "# Clean").expect("external file fixture should be written");
    let mut app = app();
    app.receive_system_open_paths(vec![path]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let external_file_id = loop {
        app.drain_product_events();
        if let Some(DocumentIdentity::ExternalFile(external_file_id)) =
            app.action_runtime.state().library.selected_card
            && app.document_tab_for(DocumentIdentity::ExternalFile(external_file_id)).is_some()
        {
            break external_file_id;
        }
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    };

    app.dispatch_action(NotoraAction::ExternalFileCloseRequested(external_file_id));

    assert!(app.action_runtime.state().external_files.session(external_file_id).is_none());
    assert!(app.document_tab_for(DocumentIdentity::ExternalFile(external_file_id)).is_none());
    assert_eq!(app.action_runtime.state().library.selected_card, None);
}

#[test]
fn closing_a_dirty_untitled_file_can_discard_its_record_and_runtime_tab() {
    let mut app = app();
    app.external_file_close_prompt = Box::new(|_, _| super::ExternalFileCloseChoice::Discard);
    let (identity, _tab_id) = install_registered_untitled_external(&mut app);
    let DocumentIdentity::ExternalFile(external_file_id) = identity else {
        panic!("untitled external fixture must have an external identity");
    };
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "未保存修改".to_owned());
    app.apply_editor_outcome(edit_outcome);

    app.dispatch_action(NotoraAction::ExternalFileCloseRequested(external_file_id));

    assert!(app.action_runtime.state().external_files.session(external_file_id).is_none());
    assert_eq!(app.document_tab_for(identity), None);
    assert_eq!(app.action_runtime.state().library.last_command_error, None);
}

#[test]
fn successful_untitled_save_requested_by_close_closes_after_path_promotion() {
    let directory = tempfile::tempdir().expect("external save fixture directory should exist");
    let saved_path = directory.path().join("saved-before-close.md");
    let mut app = app();
    let (identity, tab_id) = install_registered_untitled_external(&mut app);
    let DocumentIdentity::ExternalFile(external_file_id) = identity else {
        panic!("untitled external fixture must have an external identity");
    };
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "关闭前保存的修改".to_owned());
    app.apply_editor_outcome(edit_outcome);
    let prepared = app
        .document_runtime
        .editor_runtime
        .prepare_save_as(tab_id, &saved_path)
        .expect("dirty untitled file should prepare the chosen save path");
    app.document_runtime.pending_external_save_as.insert(
        tab_id,
        super::document_runtime::PendingExternalSaveAs {
            external_file_id,
            content_revision: prepared.content_revision,
        },
    );
    app.document_runtime.request_external_file_close_after_save(external_file_id);
    app.document_runtime
        .editor_runtime
        .submit_save(prepared, || {})
        .expect("the test polls the save worker without a window event loop");

    let deadline = Instant::now() + Duration::from_secs(2);
    while app.action_runtime.state().external_files.session(external_file_id).is_some() {
        app.drain_runtime_save_completions();
        assert_eq!(app.document_tab_for(identity), Some(tab_id));
        app.drain_product_events();
        assert!(Instant::now() < deadline, "save-before-close should complete promptly");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.document_tab_for(identity), None);
    assert_eq!(
        std::fs::read_to_string(saved_path).expect("saved file should remain on disk"),
        "关闭前保存的修改"
    );
    assert_eq!(app.action_runtime.state().library.last_command_error, None);
}

#[test]
fn clearing_external_files_closes_clean_records_and_keeps_dirty_records() {
    let directory = tempfile::tempdir().expect("external file fixture directory should exist");
    let path = directory.path().join("clear-clean.md");
    std::fs::write(&path, "# Clean").expect("external file fixture should be written");
    let mut app = app();
    app.receive_system_open_paths(vec![path]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let clean_external_file_id = loop {
        app.drain_product_events();
        if let Some(DocumentIdentity::ExternalFile(external_file_id)) =
            app.action_runtime.state().library.selected_card
            && app.document_tab_for(DocumentIdentity::ExternalFile(external_file_id)).is_some()
        {
            break external_file_id;
        }
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    };
    let (dirty_identity, dirty_tab_id) = install_registered_untitled_external(&mut app);
    let DocumentIdentity::ExternalFile(dirty_external_file_id) = dirty_identity else {
        panic!("untitled external fixture must have an external identity");
    };
    let edit_outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "未保存修改".to_owned());
    app.apply_editor_outcome(edit_outcome);

    app.dispatch_action(NotoraAction::ExternalFilesClearRequested);

    assert!(app.action_runtime.state().external_files.session(clean_external_file_id).is_none());
    assert!(app.action_runtime.state().external_files.session(dirty_external_file_id).is_some());
    assert_eq!(app.document_tab_for(dirty_identity), Some(dirty_tab_id));
    assert_eq!(
        app.action_runtime.state().library.last_command_error.as_deref(),
        Some("有 1 个文件未清空：请先保存未保存内容或取消固定")
    );
}

#[test]
fn product_completions_keep_fifo_order_and_ignore_previous_selection_errors() {
    use crate::product::{DocumentCompletion, NotoraProductEvent, PersistenceCompletion};

    let mut app = app();
    let (identity, _) = install_registered_untitled_external(&mut app);
    let generation = app.state().library.selected_document_generation;
    assert_eq!(app.state().library.selected_card, Some(identity));
    assert!(generation > 0);
    let document_error = |selection_generation, message: &str| {
        NotoraProductEvent::Document(DocumentCompletion::ExternalDocumentLoadFailed {
            request: DocumentLoadRequest { identity, selection_generation },
            message: message.to_owned(),
        })
    };
    let persistence_error = |message: &str| {
        NotoraProductEvent::Persistence(PersistenceCompletion::SessionPersistenceFailed {
            message: message.to_owned(),
        })
    };
    let sender = app.product.event_sender();
    for event in [
        persistence_error("earlier persistence failure"),
        document_error(generation, "current selection failure"),
        persistence_error("latest applicable failure"),
        document_error(generation - 1, "previous selection failure"),
    ] {
        sender.send(event).expect("the product inbox is open");
    }

    app.drain_product_events();

    assert_eq!(
        app.state().library.last_command_error.as_deref(),
        Some("latest applicable failure")
    );
}

#[test]
fn conflict_retry_captures_the_replacement_revision_off_the_caller_thread() {
    let directory = tempfile::tempdir().expect("external file fixture directory should exist");
    let path = directory.path().join("outside.md");
    std::fs::write(&path, "# Outside").expect("external file fixture should be written");
    let mut app = app();
    app.receive_system_open_paths(vec![path]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        app.drain_product_events();
        if let Some(identity) = app.action_runtime.state().library.selected_card
            && app.document_tab_for(identity).is_some()
        {
            break identity;
        }
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    };
    let tab_id = app.document_tab_for(identity).expect("external file should have a tab");
    let initial_summary = app
        .document_runtime
        .editor_runtime
        .document_summary(tab_id)
        .expect("external tab should have a summary");
    let external_path = initial_summary.path.clone().expect("external tab should retain its path");
    let initial_disk_revision = initial_summary.disk_revision;
    std::fs::write(&external_path, "# Changed elsewhere")
        .expect("external fixture should change on disk");
    app.dispatch_action(NotoraAction::SaveConflictDetected {
        identity,
        content_revision: initial_summary.content_revision,
    });
    app.dispatch_action(NotoraAction::SaveConflictResolutionRequested(
        crate::action::ConflictResolution::RetrySave,
    ));

    assert_eq!(
        app.document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .and_then(|summary| summary.disk_revision),
        initial_disk_revision,
        "revision capture must not run on the conflict action caller"
    );
    loop {
        app.drain_product_events();
        if app
            .document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .is_some_and(|summary| summary.disk_revision != initial_disk_revision)
        {
            break;
        }
        assert!(Instant::now() < deadline, "replacement revision should arrive promptly");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn late_document_load_from_an_earlier_same_identity_selection_is_discarded() {
    let mut app = app();
    let selected_identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
    let intervening_identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
    app.dispatch_action(NotoraAction::CardSelected(selected_identity));
    let outdated_generation = app.action_runtime.state().library.selected_document_generation;
    app.dispatch_action(NotoraAction::CardSelected(intervening_identity));
    app.dispatch_action(NotoraAction::CardSelected(selected_identity));
    let current_generation = app.action_runtime.state().library.selected_document_generation;

    app.install_loaded_preview(
        crate::action::DocumentLoadRequest {
            identity: selected_identity,
            selection_generation: outdated_generation,
        },
        LoadedDocument {
            path: std::path::PathBuf::from("older.md"),
            contents: "# Older".to_owned(),
            disk_revision: None,
        },
    );
    app.install_loaded_preview(
        crate::action::DocumentLoadRequest {
            identity: selected_identity,
            selection_generation: current_generation,
        },
        LoadedDocument {
            path: std::path::PathBuf::from("current.md"),
            contents: "# Current".to_owned(),
            disk_revision: None,
        },
    );

    assert_eq!(app.editor_runtime_tab_count(), 1);
    assert!(app.document_tab_for(selected_identity).is_some());
    assert_eq!(app.document_tab_for(intervening_identity), None);
}

#[test]
fn editor_runtime_routes_text_markdown_and_mindmap_documents_to_product_plugins() {
    let app = app();

    assert_eq!(
        app.document_runtime
            .editor_runtime
            .create_plugin_for_path(std::path::Path::new("draft.txt"))
            .name(),
        ui::plugin::PLUGIN_EDITOR
    );
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .create_plugin_for_path(std::path::Path::new("draft.md"))
            .name(),
        ui::plugin::PLUGIN_MARKDOWN_EDITOR
    );
    assert_eq!(
        app.document_runtime
            .editor_runtime
            .create_plugin_for_path(std::path::Path::new("draft.mmap.md"))
            .name(),
        ui::plugin::PLUGIN_MINDMAP
    );
}

#[test]
fn switching_back_to_mindmap_after_reload_refreshes_reused_generation() {
    use ui::inspector_panel::ItemId;
    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MINDMAP};

    const PATH: &str = "diagram.mmap.md";
    const ORIGINAL_SOURCE: &str = "```toml mindmap\ntheme = \"warm-night\"\n```\n# Original\n";
    const RELOADED_SOURCE: &str = "```toml mindmap\ntheme = \"dawn\"\n```\n# Reloaded\n";
    let mut app = app();
    let (identity, tab_id) = install_registered_note(&mut app, PATH, ORIGINAL_SOURCE);
    let editor = app.document_runtime.editor_mut();
    let tab = editor.tab_session(tab_id).expect("mindmap tab should be installed");
    assert_eq!(tab.plugin_name(), PLUGIN_MINDMAP);
    let generation = tab.document().generation();
    let content_revision = tab.document().content_revision();
    let _ = editor.toggle_active_inspector_panel();
    let initial_panel = editor.active_inspector_panel().expect("initial inspector should mount");
    assert_eq!(initial_panel.descriptor.presets.selected_item, Some(ItemId("warm-night".into())));
    editor.switch_active_plugin();
    assert_eq!(
        editor.tab_session(tab_id).expect("source tab should exist").plugin_name(),
        PLUGIN_EDITOR
    );

    app.document_runtime.complete_conflict_reload(
        identity,
        tab_id,
        content_revision,
        LoadedDocument { path: PATH.into(), contents: RELOADED_SOURCE.into(), disk_revision: None },
    );

    let editor = app.document_runtime.editor_mut();
    let tab = editor.tab_session(tab_id).expect("reloaded tab should exist");
    assert!(tab.document().full_text().contains("# Reloaded"));
    assert_eq!(tab.document().generation(), generation);

    editor.switch_active_plugin();

    let tab = editor.tab_session(tab_id).expect("restored mindmap tab should exist");
    assert_eq!(tab.plugin_name(), PLUGIN_MINDMAP);
    let restored_panel = editor.active_inspector_panel().expect("inspector should remount");
    assert_eq!(restored_panel.descriptor.presets.selected_item, Some(ItemId("dawn".into())));
}

#[test]
fn markdown_source_view_roundtrip_preserves_selection_and_undo_boundaries() {
    use ui::plugin::{PluginMode, SemanticEditCommand, SemanticEditPlan};

    const ORIGINAL: &str = "first second";
    const AFTER_RICH_EDIT: &str = "first second!";
    const AFTER_SOURCE_EDIT: &str = "first changed!";
    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "roundtrip.md", ORIGINAL);

    let set_selection = |app: &mut NotoraRuntime, start, end| {
        let mut tab = app
            .document_runtime
            .editor_runtime
            .tab_session_mut(tab_id)
            .expect("installed Markdown tab");
        tab.document_mut().set_cursor_offset_synced(end);
        tab.document_mut().cursor_mut().selection_anchor = (start < end).then_some(start);
        tab.document().generation()
    };
    set_selection(&mut app, ORIGINAL.len(), ORIGINAL.len());
    let outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "!".to_owned());
    app.apply_editor_outcome(outcome);
    let rich_generation = set_selection(&mut app, "first ".len(), ORIGINAL.len());

    app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
    let tab = app.document_runtime.editor_runtime.tab_session(tab_id).expect("source tab");
    assert_eq!(tab.plugin().mode(), PluginMode::Source);
    assert_eq!(tab.document().full_text(), AFTER_RICH_EDIT);
    assert_eq!(tab.document().generation(), rich_generation);
    assert_eq!(tab.document().selection_range(), Some(("first ".len(), ORIGINAL.len())));

    let outcome = app
        .document_runtime
        .editor_runtime
        .commit_text(active_editor_input_context(), "changed".to_owned());
    app.apply_editor_outcome(outcome);
    let source_generation = set_selection(&mut app, 0, "first".len());
    assert_ne!(source_generation, rich_generation);

    app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
    let tab =
        app.document_runtime.editor_runtime.tab_session(tab_id).expect("restored Markdown tab");
    assert_eq!(tab.plugin().mode(), PluginMode::RichText);
    assert_eq!(tab.document().full_text(), AFTER_SOURCE_EDIT);
    assert_eq!(tab.document().generation(), source_generation);
    assert_eq!(tab.document().selection_range(), Some((0, "first".len())));
    let editable = tab.plugin().editable().expect("Markdown supports semantic edits");
    assert!(matches!(
        editable.plan_semantic_edit(SemanticEditCommand::ToggleBold, source_generation, "first".len(), Some(0.."first".len())),
        SemanticEditPlan::Apply(transaction) if transaction.source_generation == source_generation
    ));

    for expected in [AFTER_RICH_EDIT, ORIGINAL] {
        app.dispatch_action(NotoraAction::SemanticEditRequested(SemanticEditCommand::Undo));
        let tab = app
            .document_runtime
            .editor_runtime
            .tab_session(tab_id)
            .expect("Markdown tab after undo");
        assert_eq!(tab.document().full_text(), expected);
    }
}

#[test]
fn semantic_edit_refreshes_stale_plugin_source_before_planning() {
    use appkit_shell::editor_runtime::SemanticEditResult;
    use ui::plugin::SemanticEditCommand;

    let mut app = app();
    let (_, tab_id) = install_registered_note(&mut app, "note.md", "old");
    let editor = app.document_runtime.editor_mut();
    let mut tab = editor.tab_session_mut(tab_id).expect("Markdown tab should be installed");
    // Change the document without synchronizing the active plugin.
    tab.document_mut().replace_range(0.."old".len(), "new");

    let (result, _) = editor.execute_semantic_edit(SemanticEditCommand::SetHeadingLevel(2));

    assert_eq!(result, SemanticEditResult::Applied);
    let tab = editor.tab_session(tab_id).expect("edited Markdown tab should exist");
    assert_eq!(tab.document().full_text(), "## new");
}

#[test]
fn open_document_sender_routes_system_paths_through_product_inbox() {
    let directory = tempfile::tempdir().expect("external fixture directory should exist");
    let path = directory.path().join("architecture.md");
    std::fs::write(&path, "# Architecture").expect("external fixture should be written");
    let mut app = app();

    app.open_document_sender()
        .send(vec![path.clone()])
        .expect("product inbox should accept system open paths");

    let deadline = Instant::now() + Duration::from_secs(2);
    while app.editor_runtime_tab_count() == 0 {
        app.drain_product_events();
        assert!(Instant::now() < deadline, "external preview should install promptly");
        thread::sleep(Duration::from_millis(10));
    }

    let canonical_path =
        std::fs::canonicalize(&path).expect("external fixture path should canonicalize");
    assert_eq!(app.action_runtime.state().library.navigation_scope, NavigationScope::ExternalFiles);
    assert!(matches!(
        app.action_runtime.state().library.selected_card,
        Some(DocumentIdentity::ExternalFile(external_file_id))
            if matches!(
                app.action_runtime.state().external_files.session(external_file_id),
                Some(ExternalFileSession::Existing { canonical_path: known_path, .. })
                    if known_path.as_path() == canonical_path.as_path()
            )
    ));
}

#[test]
fn system_open_before_session_restore_completion_keeps_the_external_document_focused() {
    let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
    let workspace = notora_core::Workspace::open_or_initialize(workspace_directory.path())
        .expect("workspace metadata should initialize");
    let external_directory = tempfile::tempdir().expect("external fixture should be created");
    let external_path = external_directory.path().join("architecture.md");
    std::fs::write(&external_path, "# Architecture").expect("external fixture should be written");
    let mut app = app();
    app.persistence_runtime.pending_session = Some(crate::session::ProductSession {
        workspace_root: Some(workspace_directory.path().to_path_buf()),
        workspace_id: Some(workspace.descriptor().workspace_id),
        last_navigation_scope: crate::session::SavedNavigationScope::WorkspaceRoot,
        last_document: Some(crate::session::SavedDocument::Note {
            note_id: notora_core::NoteId::generate(),
        }),
        ..crate::session::ProductSession::default()
    });

    app.restore_pending_session();
    assert!(app.session_restore_runtime.is_active());
    app.receive_system_open_paths(vec![external_path.clone()]);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        app.drain_product_events();
        let restored = !app.session_restore_runtime.is_active();
        let focused_external = matches!(
            app.action_runtime.state().library.selected_card,
            Some(DocumentIdentity::ExternalFile(_))
        );
        if restored
            && app.editor_runtime_tab_count() == 1
            && focused_external
            && app.action_runtime.state().library.navigation_scope == NavigationScope::ExternalFiles
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "system open target should remain focused after session restore completes"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let canonical_path =
        std::fs::canonicalize(&external_path).expect("external fixture path should canonicalize");
    assert!(matches!(
        app.action_runtime.state().library.selected_card,
        Some(DocumentIdentity::ExternalFile(external_file_id))
            if matches!(
                app.action_runtime.state().external_files.session(external_file_id),
                Some(ExternalFileSession::Existing { canonical_path: known_path, .. })
                    if known_path.as_path() == canonical_path.as_path()
            )
    ));
}
