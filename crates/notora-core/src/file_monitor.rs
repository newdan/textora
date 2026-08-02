use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

use crate::WORKSPACE_METADATA_DIRECTORY_NAME;

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);
const MACOS_FINDER_METADATA_FILE_NAME: &str = ".DS_Store";
const MACOS_RESOURCE_FORK_PREFIX: &str = "._";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileBatch {
    pub relative_paths: Vec<PathBuf>,
}

pub struct WorkspaceFileMonitor {
    command_sender: mpsc::Sender<MonitorCommand>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub enum WorkspaceFileMonitorError {
    Watcher(notify::Error),
    WorkerDisconnected,
}

impl std::fmt::Display for WorkspaceFileMonitorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Watcher(source) => write!(formatter, "workspace file watcher failed: {source}"),
            Self::WorkerDisconnected => {
                formatter.write_str("workspace file watcher worker disconnected")
            }
        }
    }
}

impl std::error::Error for WorkspaceFileMonitorError {}

impl WorkspaceFileMonitor {
    pub fn start(
        root: PathBuf,
    ) -> Result<(Self, mpsc::Receiver<WorkspaceFileBatch>), WorkspaceFileMonitorError> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let (startup_sender, startup_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("notora-file-monitor".to_owned())
            .spawn(move || run_worker(root, command_receiver, result_sender, startup_sender))
            .map_err(|_| WorkspaceFileMonitorError::WorkerDisconnected)?;
        startup_receiver.recv().map_err(|_| WorkspaceFileMonitorError::WorkerDisconnected)??;

        Ok((Self { command_sender, worker: Some(worker) }, result_receiver))
    }

    pub fn shutdown(&mut self) {
        let _ = self.command_sender.send(MonitorCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for WorkspaceFileMonitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum MonitorCommand {
    Shutdown,
}

fn run_worker(
    root: PathBuf,
    command_receiver: mpsc::Receiver<MonitorCommand>,
    result_sender: mpsc::Sender<WorkspaceFileBatch>,
    startup_sender: mpsc::Sender<Result<(), WorkspaceFileMonitorError>>,
) {
    let (event_sender, event_receiver) = mpsc::channel();
    let mut watcher = match RecommendedWatcher::new(
        move |event| {
            let _ = event_sender.send(event);
        },
        Config::default(),
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            let _ = startup_sender.send(Err(WorkspaceFileMonitorError::Watcher(error)));
            return;
        }
    };
    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        let _ = startup_sender.send(Err(WorkspaceFileMonitorError::Watcher(error)));
        return;
    }
    if startup_sender.send(Ok(())).is_err() {
        return;
    }

    run_event_batch_loop(
        &root,
        &command_receiver,
        &event_receiver,
        &result_sender,
        DEFAULT_DEBOUNCE,
    );
}

fn run_event_batch_loop(
    root: &Path,
    command_receiver: &mpsc::Receiver<MonitorCommand>,
    event_receiver: &mpsc::Receiver<notify::Result<notify::Event>>,
    result_sender: &mpsc::Sender<WorkspaceFileBatch>,
    debounce: Duration,
) {
    let mut pending_paths = Vec::new();
    loop {
        match command_receiver.recv_timeout(debounce) {
            Ok(MonitorCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drain_events(event_receiver, &mut pending_paths);
                if !send_batch(root, result_sender, &mut pending_paths) {
                    return;
                }
            }
        }
    }
}

fn drain_events(
    event_receiver: &mpsc::Receiver<notify::Result<notify::Event>>,
    pending_paths: &mut Vec<PathBuf>,
) {
    while let Ok(Ok(event)) = event_receiver.try_recv() {
        pending_paths.extend(event.paths);
    }
}

fn send_batch(
    root: &Path,
    sender: &mpsc::Sender<WorkspaceFileBatch>,
    pending_paths: &mut Vec<PathBuf>,
) -> bool {
    let relative_paths = filter_relative_paths(root, std::mem::take(pending_paths));
    if relative_paths.is_empty() {
        return true;
    }
    sender.send(WorkspaceFileBatch { relative_paths }).is_ok()
}

fn filter_relative_paths(root: &Path, paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| path.strip_prefix(root).ok().map(Path::to_path_buf))
        .filter(|path| !should_ignore(path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn should_ignore(relative_path: &Path) -> bool {
    let Some(first_component) = relative_path.components().next() else {
        return true;
    };
    if first_component.as_os_str() == WORKSPACE_METADATA_DIRECTORY_NAME {
        return true;
    }
    relative_path.file_name().and_then(|name| name.to_str()).is_none_or(|file_name| {
        file_name == MACOS_FINDER_METADATA_FILE_NAME
            || file_name.starts_with(MACOS_RESOURCE_FORK_PREFIX)
            || is_atomic_write_temporary_file(file_name)
    })
}

fn is_atomic_write_temporary_file(file_name: &str) -> bool {
    let Some(identifier) = file_name.strip_prefix('.').and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((prefix, counter)) = identifier.rsplit_once('.') else {
        return false;
    };
    let Some((_, process_id)) = prefix.rsplit_once('.') else {
        return false;
    };
    !process_id.is_empty()
        && process_id.bytes().all(|byte| byte.is_ascii_digit())
        && !counter.is_empty()
        && counter.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use notify::event::{ModifyKind, RenameMode};
    use notify::{Event, EventKind};

    use super::{MonitorCommand, filter_relative_paths, run_event_batch_loop};

    #[test]
    fn filter_ignores_metadata_and_deduplicates_paths() {
        let root = Path::new("/workspace");
        let paths = vec![
            PathBuf::from("/workspace/note.md"),
            PathBuf::from("/workspace/note.md"),
            PathBuf::from("/workspace/.notora/catalog.sqlite3"),
            PathBuf::from("/workspace/._note.md"),
            PathBuf::from("/workspace/.note.md.1234.7.tmp"),
        ];

        assert_eq!(filter_relative_paths(root, paths), vec![PathBuf::from("note.md")]);
    }

    #[test]
    fn injected_events_debounce_split_rename_events_into_one_normalized_batch() {
        let root = PathBuf::from("/workspace");
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            run_event_batch_loop(
                &root,
                &command_receiver,
                &event_receiver,
                &result_sender,
                Duration::from_millis(10),
            )
        });
        event_sender
            .send(Ok(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
                .add_path(PathBuf::from("/workspace/before.md"))))
            .expect("rename source event should send");
        event_sender
            .send(Ok(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To)))
                .add_path(PathBuf::from("/workspace/after.md"))))
            .expect("rename destination event should send");

        let batch = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("rename events should be emitted as one batch");
        assert_eq!(
            batch.relative_paths,
            vec![PathBuf::from("after.md"), PathBuf::from("before.md")]
        );
        command_sender.send(MonitorCommand::Shutdown).expect("worker should accept shutdown");
        worker.join().expect("injected event worker should stop cleanly");
    }
}
