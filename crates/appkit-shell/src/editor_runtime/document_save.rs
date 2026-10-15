//! 不可变文档保存快照与异步执行协议。

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use appkit_core::document::DocumentSaveError;
use appkit_core::file_safety::DiskRevision;
use appkit_core::workspace::types::TabId;

type PayloadTransform = dyn Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync + 'static;

#[derive(Clone)]
pub struct SavePayloadTransform(std::sync::Arc<PayloadTransform>);

impl SavePayloadTransform {
    pub fn new(
        transform: impl Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    ) -> Self {
        Self(std::sync::Arc::new(transform))
    }

    fn apply(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        (self.0)(plaintext)
    }
}

impl std::fmt::Debug for SavePayloadTransform {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SavePayloadTransform(<opaque>)")
    }
}

/// 可脱离 runtime 在 worker 中执行的保存快照。
#[derive(Debug, PartialEq, Eq)]
pub struct PreparedDocumentSave {
    pub tab_id: TabId,
    pub path: PathBuf,
    pub serialized_contents: Vec<u8>,
    pub expected_disk_revision: Option<DiskRevision>,
    pub content_revision: u64,
}

/// 保存快照准备失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavePrepareError {
    UnknownTab { tab_id: TabId },
    Untitled { tab_id: TabId },
    SubmitFailed { message: String },
}

impl std::fmt::Display for SavePrepareError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTab { tab_id } => write!(formatter, "unknown tab {}", tab_id.as_u64()),
            Self::Untitled { tab_id } => {
                write!(formatter, "tab {} has no file path", tab_id.as_u64())
            }
            Self::SubmitFailed { message } => {
                write!(formatter, "save worker unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for SavePrepareError {}

/// worker 执行完成后回到 runtime 的结果。
pub struct SaveCompletion {
    pub tab_id: TabId,
    pub content_revision: u64,
    pub result: Result<DiskRevision, DocumentSaveError>,
}

pub(crate) struct SaveSession {
    sender: Sender<SaveCompletion>,
    receiver: Receiver<SaveCompletion>,
}

impl SaveSession {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self { sender, receiver }
    }

    pub(crate) fn submit(
        &self,
        prepared: PreparedDocumentSave,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), String> {
        self.submit_with_transform(prepared, None, wake)
    }

    pub(crate) fn submit_with_transform(
        &self,
        prepared: PreparedDocumentSave,
        transform: Option<SavePayloadTransform>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), String> {
        let operation = self.save_operation(prepared, transform, wake);
        thread::Builder::new()
            .name("textora-save".to_owned())
            .spawn(move || {
                let _ = operation(None);
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn submit_on_file_worker(
        &self,
        worker: &appkit_core::file_safety::FileSafetyWorker,
        prepared: PreparedDocumentSave,
        transform: Option<SavePayloadTransform>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), String> {
        let path = prepared.path.clone();
        let expected_revision = prepared.expected_disk_revision.clone();
        let operation = self.save_operation(prepared, transform, wake);
        worker
            .submit(appkit_core::file_safety::FileSafetyCommand::Save {
                path,
                expected_revision,
                execute: Box::new(operation),
            })
            .map_err(|error| error.to_string())
    }

    fn save_operation(
        &self,
        mut prepared: PreparedDocumentSave,
        transform: Option<SavePayloadTransform>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> impl FnOnce(Option<DiskRevision>) -> Option<DiskRevision> + Send + 'static {
        let sender = self.sender.clone();
        move |saved_revision| {
            if prepared.expected_disk_revision.is_some() && saved_revision.is_some() {
                prepared.expected_disk_revision = saved_revision;
            }
            let completion = execute_serialized_save(prepared, transform);
            let saved_revision = completion.result.as_ref().ok().cloned();
            if sender.send(completion).is_ok() {
                wake();
            }
            saved_revision
        }
    }

    pub(crate) fn drain(&self) -> Vec<SaveCompletion> {
        self.receiver.try_iter().collect()
    }
}

/// 在产品 worker/thread pool 中执行不可变保存快照。
pub fn execute_prepared_save(prepared: PreparedDocumentSave) -> SaveCompletion {
    execute_serialized_save(prepared, None)
}

pub fn execute_prepared_save_with_transform(
    prepared: PreparedDocumentSave,
    transform: SavePayloadTransform,
) -> SaveCompletion {
    execute_serialized_save(prepared, Some(transform))
}

fn execute_serialized_save(
    mut prepared: PreparedDocumentSave,
    transform: Option<SavePayloadTransform>,
) -> SaveCompletion {
    if let Some(transform) = transform {
        prepared.serialized_contents = match transform.apply(&prepared.serialized_contents) {
            Ok(contents) => contents,
            Err(message) => {
                return SaveCompletion {
                    tab_id: prepared.tab_id,
                    content_revision: prepared.content_revision,
                    result: Err(DocumentSaveError::Io {
                        message: format!("payload transform failed: {message}"),
                    }),
                };
            }
        };
    }
    let result = appkit_core::file_safety::save_serialized_if_unchanged(
        &prepared.path,
        prepared.expected_disk_revision.as_ref(),
        &prepared.serialized_contents,
    )
    .map_err(map_save_error);

    SaveCompletion { tab_id: prepared.tab_id, content_revision: prepared.content_revision, result }
}

fn map_save_error(error: appkit_core::file_safety::FileSafetyError) -> DocumentSaveError {
    let message = error.to_string();
    match error {
        appkit_core::file_safety::FileSafetyError::ConcurrentModification => {
            DocumentSaveError::ConcurrentModification
        }
        appkit_core::file_safety::FileSafetyError::Io { .. }
        | appkit_core::file_safety::FileSafetyError::InvalidPath { .. } => {
            DocumentSaveError::Io { message }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::editor_plugin::EditorPluginFactory;
    use crate::editor_runtime::{EditorRuntime, EditorRuntimeConfig, OpenDisposition};
    use crate::prepared_tab::PreparedTab;
    use crate::tab_runtime::TabRuntime;
    use appkit_core::file_safety::capture_revision;
    use ui::plugin::PluginFactory;

    fn tab_id() -> TabId {
        let mut allocator = appkit_core::workspace::types::TabIdAllocator::new();
        allocator.allocate()
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should be after UNIX epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("textora-{label}-{suffix}"));
            fs::create_dir_all(&path).expect("save protocol test directory should be created");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn runtime() -> EditorRuntime {
        let mut registry = ui::plugin::PluginRegistry::new();
        registry.register(Box::new(EditorPluginFactory));
        let routes = crate::view_route::ViewRouteTable::new(
            Vec::new(),
            &std::collections::HashSet::from([ui::plugin::PLUGIN_EDITOR]),
        )
        .expect("save protocol route table should be valid");
        EditorRuntime::new(EditorRuntimeConfig {
            plugin_registry: registry,
            view_routes: routes,
            initial_settings: ui::Settings::new(),
            initial_theme: ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark()),
            snapshots_directory: PathBuf::from("snapshots"),
        })
        .expect("save protocol runtime should construct")
    }

    fn install_dirty_saved_bom_crlf_tab(runtime: &mut EditorRuntime, path: &std::path::Path) {
        let mut text_buffer =
            core::buffer::TextBuffer::new(false).expect("test text buffer should be writable");
        text_buffer.write_raw("clean\n段落".as_bytes());
        let mut document = appkit_core::document::DocumentModel::new(text_buffer);
        document.file_path = Some(path.to_path_buf());
        document.disk_revision =
            Some(capture_revision(path).expect("initial disk revision should capture"));
        document.dirty = true;
        document.crlf = true;
        document.had_bom = true;
        fs::write(path, document.serialized_contents_for_save())
            .expect("pending local save should have reached disk");
        runtime.install_prepared_tab(
            PreparedTab::new(document, TabRuntime::new(EditorPluginFactory.create())),
            None,
            OpenDisposition::Persistent,
        );
    }

    #[test]
    fn file_safety_compares_the_serialized_bom_and_crlf_snapshot() {
        let directory = TestDirectory::new("serialized-file-safety");
        let path = directory.path().join("notes.md");
        fs::write(&path, "baseline").expect("initial file should be written");
        let mut runtime = runtime();
        install_dirty_saved_bom_crlf_tab(&mut runtime, &path);
        let (wake_sender, wake_receiver) = std::sync::mpsc::channel();
        runtime.start_file_safety_worker(move || {
            let _ = wake_sender.send(());
        });

        assert_eq!(runtime.submit_file_safety_checks("local"), 1);
        wake_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("file safety worker should finish the queued snapshot");
        let observations = runtime.drain_file_safety_observations();

        assert_eq!(observations.len(), 1);
        assert!(matches!(
            observations[0].outcome,
            Ok(appkit_core::file_safety::FileSafetyOutcome::Unchanged)
        ));
        assert_eq!(
            std::fs::read_dir(directory.path()).expect("test directory should be readable").count(),
            1
        );
    }

    #[test]
    fn worker_executes_snapshot_and_returns_actual_disk_revision() {
        let directory = TestDirectory::new("save");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("save protocol baseline should be written");
        let baseline = capture_revision(&path).expect("save protocol baseline should capture");
        let completion = execute_prepared_save(PreparedDocumentSave {
            tab_id: tab_id(),
            path: path.clone(),
            serialized_contents: b"new".to_vec(),
            expected_disk_revision: Some(baseline),
            content_revision: 4,
        });

        let revision = completion.result.expect("unchanged baseline should save");
        assert_eq!(revision.path, path);
        assert_eq!(
            fs::read_to_string(&revision.path).expect("saved file should be readable"),
            "new"
        );
    }

    fn install_dirty_save_race_tab(runtime: &mut EditorRuntime, path: &std::path::Path) -> TabId {
        fs::write(path, "old").expect("write save-race baseline");
        let mut buffer = core::buffer::TextBuffer::new(false).expect("writable save-race buffer");
        buffer.write_raw(b"old");
        buffer.mark_as_clean();
        let mut document = appkit_core::document::DocumentModel::new(buffer);
        document.file_path = Some(path.to_path_buf());
        document.disk_revision = Some(capture_revision(path).expect("capture save-race baseline"));
        document.replace_range(0..3, "saved");
        let _ = runtime.install_prepared_tab(
            PreparedTab::new(document, TabRuntime::new(EditorPluginFactory.create())),
            None,
            OpenDisposition::Persistent,
        );
        runtime.active_tab_id().expect("installed save-race tab")
    }

    #[test]
    fn file_safety_serializes_local_save_and_subsequent_edits() {
        use appkit_core::file_safety::FileSafetyOutcome;
        for initially_tracked in [false, true] {
            for apply_completion in [false, true] {
                let directory = TestDirectory::new("save-observation-race");
                let path = directory.path().join("notes.md");
                let mut runtime = runtime();
                let tab_id = install_dirty_save_race_tab(&mut runtime, &path);
                let (check_sender, check_receiver) = mpsc::channel();
                runtime.start_file_safety_worker(move || {
                    let _ = check_sender.send(());
                });
                if initially_tracked {
                    assert_eq!(runtime.submit_file_safety_checks("local"), 1);
                    check_receiver
                        .recv_timeout(Duration::from_secs(2))
                        .expect("initial reconciliation");
                    assert_eq!(runtime.drain_file_safety_observations().len(), 1);
                }
                let prepared = runtime.prepare_save(tab_id).expect("prepare local save");
                let (save_sender, save_receiver) = mpsc::channel();
                runtime
                    .submit_save(prepared, move || {
                        let _ = save_sender.send(());
                    })
                    .expect("queue local save");
                save_receiver.recv_timeout(Duration::from_secs(2)).expect("save completion");
                let completion =
                    runtime.drain_save_completions().pop().expect("completed local save");
                assert!(completion.result.is_ok());
                if apply_completion {
                    let _ = runtime.apply_save_completion(completion);
                }
                runtime
                    .tab_session_mut(tab_id)
                    .expect("save-race tab")
                    .document
                    .replace_range(0..5, "later");
                assert_eq!(runtime.submit_file_safety_checks("local"), 1);
                check_receiver.recv_timeout(Duration::from_secs(2)).expect("observe later edit");
                let observations = runtime.drain_file_safety_observations();
                assert!(
                    matches!(observations.as_slice(), [observation] if matches!(observation.outcome, Ok(FileSafetyOutcome::Unchanged))),
                    "own save must not conflict with a subsequent edit: tracked={initially_tracked}, completion={apply_completion}, {observations:?}"
                );
                assert_eq!(fs::read_dir(directory.path()).expect("fixture files").count(), 1);
                fs::write(&path, "remote").expect("write actual external change");
                assert_eq!(runtime.submit_file_safety_checks("local"), 1);
                check_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .expect("observe actual external change");
                assert!(
                    matches!(runtime.drain_file_safety_observations().as_slice(), [observation] if matches!(observation.outcome, Ok(FileSafetyOutcome::Conflict { .. })))
                );
            }
        }
    }

    #[test]
    fn local_save_queue_preserves_the_latest_snapshot_before_completions_are_applied() {
        let directory = TestDirectory::new("consecutive-local-saves");
        let path = directory.path().join("notes.md");
        let mut runtime = runtime();
        let tab_id = install_dirty_save_race_tab(&mut runtime, &path);
        runtime.start_file_safety_worker(|| {});
        let first = runtime.prepare_save(tab_id).expect("prepare first local save");
        runtime
            .tab_session_mut(tab_id)
            .expect("consecutive-save tab")
            .document
            .replace_range(0..5, "latest");
        let second = runtime.prepare_save(tab_id).expect("prepare latest local save");
        runtime
            .tab_session_mut(tab_id)
            .expect("latest queued tab")
            .document
            .replace_range(0..6, "final");
        let third = runtime.prepare_save(tab_id).expect("prepare final local save");
        let (wake_sender, wake_receiver) = mpsc::channel();
        let snapshots = [first, second, third];
        let snapshot_count = snapshots.len();
        for prepared in snapshots {
            let sender = wake_sender.clone();
            runtime
                .submit_save(prepared, move || {
                    let _ = sender.send(());
                })
                .expect("queue consecutive save");
        }
        for _ in 0..snapshot_count {
            wake_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("consecutive save completion");
        }
        let completions = runtime.drain_save_completions();
        assert_eq!(completions.len(), snapshot_count);
        for completion in completions {
            assert!(
                completion.result.is_ok(),
                "a preceding local save must not invalidate a later local snapshot"
            );
            let _ = runtime.apply_save_completion(completion);
        }
        assert_eq!(fs::read_to_string(path).expect("latest saved bytes"), "final");
        assert!(!runtime.document_summary(tab_id).expect("saved tab").dirty);
    }

    fn complete_runtime_save(
        runtime: &mut EditorRuntime,
        prepared: PreparedDocumentSave,
    ) -> SaveCompletion {
        let (sender, receiver) = mpsc::channel();
        runtime
            .submit_save(prepared, move || {
                let _ = sender.send(());
            })
            .expect("submit runtime save");
        receiver.recv_timeout(Duration::from_secs(2)).expect("runtime save completion");
        runtime.drain_save_completions().pop().expect("completed runtime save")
    }

    #[test]
    fn local_save_queue_keeps_explicit_external_baselines_and_rejects_new_external_changes() {
        let directory = TestDirectory::new("explicit-save-baselines");
        let path = directory.path().join("notes.md");
        let mut runtime = runtime();
        let tab_id = install_dirty_save_race_tab(&mut runtime, &path);
        runtime.start_file_safety_worker(|| {});
        let prepared = runtime.prepare_save(tab_id).expect("first queued snapshot");
        let completion = complete_runtime_save(&mut runtime, prepared);
        assert!(completion.result.is_ok());
        let _ = runtime.apply_save_completion(completion);
        fs::write(&path, "external").expect("write accepted external baseline");
        let baseline = capture_revision(&path).expect("capture accepted external revision");
        let tab = runtime.tab_session_mut(tab_id).expect("explicit baseline tab");
        tab.document.disk_revision = Some(baseline);
        tab.document.replace_range(0..5, "merged");
        let prepared =
            runtime.prepare_save(tab_id).expect("save based on explicit external revision");
        let completion = complete_runtime_save(&mut runtime, prepared);
        assert!(
            completion.result.is_ok(),
            "known own-save history must not replace a different explicit baseline"
        );
        let _ = runtime.apply_save_completion(completion);
        let prepared = runtime.prepare_save(tab_id).expect("prepare protected queued save");
        fs::write(&path, "new external").expect("external edit after snapshot preparation");
        let completion = complete_runtime_save(&mut runtime, prepared);
        assert!(matches!(completion.result, Err(DocumentSaveError::ConcurrentModification)));
        assert_eq!(fs::read_to_string(path).expect("protected external bytes"), "new external");
    }

    #[test]
    fn file_safety_waits_for_a_local_save_before_observing_a_newer_edit() {
        let directory = TestDirectory::new("ordered-save-observation");
        let path = directory.path().join("notes.md");
        let mut runtime = runtime();
        let tab_id = install_dirty_save_race_tab(&mut runtime, &path);
        let (check_sender, check_receiver) = mpsc::channel();
        runtime.start_file_safety_worker(move || {
            let _ = check_sender.send(());
        });
        let prepared = runtime.prepare_save(tab_id).expect("prepare gated local save");
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let release = std::sync::Mutex::new(release_receiver);
        let transform = SavePayloadTransform::new(move |bytes| {
            entered_sender.send(()).expect("announce gated save");
            release.lock().expect("gated save release lock").recv().expect("release gated save");
            Ok(bytes.to_vec())
        });
        let (save_sender, save_receiver) = mpsc::channel();
        runtime
            .submit_save_with_transform(prepared, transform, move || {
                let _ = save_sender.send(());
            })
            .expect("queue gated save");
        entered_receiver.recv_timeout(Duration::from_secs(2)).expect("save reached gate");
        runtime
            .tab_session_mut(tab_id)
            .expect("gated save tab")
            .document
            .replace_range(0..5, "later");
        assert_eq!(runtime.submit_file_safety_checks("local"), 1);
        let observed_while_save_blocked = check_receiver.try_recv().is_ok();
        release_sender.send(()).expect("allow atomic save to proceed");
        save_receiver.recv_timeout(Duration::from_secs(2)).expect("gated save completed");
        check_receiver.recv_timeout(Duration::from_secs(2)).expect("ordered observation completed");
        assert!(!observed_while_save_blocked, "file observation must share the save queue");
        assert!(
            matches!(runtime.drain_file_safety_observations().as_slice(), [observation] if matches!(observation.outcome, Ok(appkit_core::file_safety::FileSafetyOutcome::Unchanged)))
        );
        assert_eq!(fs::read_dir(directory.path()).expect("ordered fixture files").count(), 1);
    }

    #[test]
    fn worker_rejects_external_modification_without_overwriting_it() {
        let directory = TestDirectory::new("race");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("save race baseline should be written");
        let baseline = capture_revision(&path).expect("save race baseline should capture");
        fs::write(&path, "external").expect("external edit should be written");
        let completion = execute_prepared_save(PreparedDocumentSave {
            tab_id: tab_id(),
            path: path.clone(),
            serialized_contents: b"local".to_vec(),
            expected_disk_revision: Some(baseline),
            content_revision: 4,
        });

        assert!(matches!(completion.result, Err(DocumentSaveError::ConcurrentModification)));
        assert_eq!(fs::read_to_string(path).expect("external file should remain"), "external");
    }

    #[test]
    fn worker_transforms_payload_before_writing() {
        let directory = TestDirectory::new("transform");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("transform baseline should be written");
        let baseline = capture_revision(&path).expect("transform baseline should capture");
        let prepared = PreparedDocumentSave {
            tab_id: tab_id(),
            path: path.clone(),
            serialized_contents: b"plaintext".to_vec(),
            expected_disk_revision: Some(baseline),
            content_revision: 5,
        };

        let completion = execute_prepared_save_with_transform(
            prepared,
            SavePayloadTransform::new(|plaintext| {
                let mut transformed = b"wrapped:".to_vec();
                transformed.extend_from_slice(plaintext);
                Ok(transformed)
            }),
        );

        completion.result.expect("payload transform should save");
        assert_eq!(
            fs::read(path).expect("transformed file should be readable"),
            b"wrapped:plaintext"
        );
    }

    #[test]
    fn save_session_wakes_product_and_drains_completion() {
        let directory = TestDirectory::new("session");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("save session baseline should be written");
        let baseline = capture_revision(&path).expect("save session baseline should capture");
        let session = SaveSession::new();
        let (wake_sender, wake_receiver) = mpsc::channel();

        session
            .submit(
                PreparedDocumentSave {
                    tab_id: tab_id(),
                    path: path.clone(),
                    serialized_contents: b"new".to_vec(),
                    expected_disk_revision: Some(baseline),
                    content_revision: 8,
                },
                move || {
                    wake_sender.send(()).expect("save session wake receiver should be alive");
                },
            )
            .expect("save session worker should start");

        wake_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("save session should wake the product after completion");
        let completions = session.drain();

        assert_eq!(completions.len(), 1);
        assert!(completions[0].result.is_ok());
        assert_eq!(fs::read_to_string(path).expect("saved file should be readable"), "new");
    }

    #[test]
    fn runtime_prepare_and_apply_save_uses_stable_tab_id() {
        let directory = TestDirectory::new("runtime");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("runtime save baseline should be written");
        let baseline = capture_revision(&path).expect("runtime save baseline should capture");
        let mut text_buffer =
            core::buffer::TextBuffer::new(false).expect("runtime save buffer should construct");
        text_buffer.write_raw(b"new");
        text_buffer.mark_as_clean();
        let mut document = appkit_core::document::DocumentModel::new(text_buffer);
        document.file_path = Some(path.clone());
        document.disk_revision = Some(baseline);
        document.insert_at_cursor(b"!");

        let mut runtime = runtime();
        let install = runtime.install_prepared_tab(
            PreparedTab::new(document, TabRuntime::new(EditorPluginFactory.create())),
            None,
            OpenDisposition::Persistent,
        );
        let tab_id = match install.notifications[0] {
            crate::editor_runtime::EditorNotification::ActiveDocumentChanged {
                tab_id: Some(id),
            } => id,
            _ => panic!("install should report the active stable tab id"),
        };
        let prepared = runtime.prepare_save(tab_id).expect("dirty file should prepare");
        let completion = execute_prepared_save(prepared);
        let outcome = runtime.apply_save_completion(completion);

        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            crate::editor_runtime::EditorNotification::SaveCompleted { tab_id: id, .. } if *id == tab_id
        )));
        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            crate::editor_runtime::EditorNotification::DirtyChanged { tab_id: id, dirty: false } if *id == tab_id
        )));
        assert!(!runtime.document_summary(tab_id).expect("saved tab should remain").dirty);
    }

    #[test]
    fn late_save_completion_after_close_is_ignored() {
        let directory = TestDirectory::new("late");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("late save baseline should be written");
        let baseline = capture_revision(&path).expect("late save baseline should capture");
        let mut text_buffer =
            core::buffer::TextBuffer::new(false).expect("late save buffer should construct");
        text_buffer.write_raw(b"new");
        text_buffer.mark_as_clean();
        let mut document = appkit_core::document::DocumentModel::new(text_buffer);
        document.file_path = Some(path.clone());
        document.disk_revision = Some(baseline);

        let mut runtime = runtime();
        let install = runtime.install_prepared_tab(
            PreparedTab::new(document, TabRuntime::new(EditorPluginFactory.create())),
            None,
            OpenDisposition::Persistent,
        );
        let tab_id = runtime.active_tab_id().expect("installed tab should be active");
        let completion =
            execute_prepared_save(runtime.prepare_save(tab_id).expect("save should prepare"));
        let _ = runtime.confirm_close(tab_id, crate::editor_runtime::CloseConfirmation::Saved);

        assert!(runtime.document_summary(tab_id).is_none());
        assert!(runtime.apply_save_completion(completion).notifications.is_empty());
        let _ = install;
    }
}
