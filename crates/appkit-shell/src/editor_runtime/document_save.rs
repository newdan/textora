//! 不可变文档保存快照与异步执行协议。

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use appkit_core::document::DocumentSaveError;
use appkit_core::file_safety::DiskRevision;
use appkit_core::workspace::types::TabId;

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
        let sender = self.sender.clone();
        thread::Builder::new()
            .name("textora-save".to_owned())
            .spawn(move || {
                let completion = execute_prepared_save(prepared);
                if sender.send(completion).is_ok() {
                    wake();
                }
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn drain(&self) -> Vec<SaveCompletion> {
        self.receiver.try_iter().collect()
    }
}

/// 在产品 worker/thread pool 中执行不可变保存快照。
pub fn execute_prepared_save(prepared: PreparedDocumentSave) -> SaveCompletion {
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
