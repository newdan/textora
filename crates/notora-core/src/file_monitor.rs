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

    let mut pending_paths = Vec::new();
    loop {
        match command_receiver.recv_timeout(DEFAULT_DEBOUNCE) {
            Ok(MonitorCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drain_events(&event_receiver, &mut pending_paths);
                send_batch(&root, &result_sender, &mut pending_paths);
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
) {
    let relative_paths = filter_relative_paths(root, std::mem::take(pending_paths));
    if !relative_paths.is_empty() {
        let _ = sender.send(WorkspaceFileBatch { relative_paths });
    }
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
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::filter_relative_paths;

    #[test]
    fn filter_ignores_metadata_and_deduplicates_paths() {
        let root = Path::new("/workspace");
        let paths = vec![
            PathBuf::from("/workspace/note.md"),
            PathBuf::from("/workspace/note.md"),
            PathBuf::from("/workspace/.notora/catalog.sqlite3"),
            PathBuf::from("/workspace/._note.md"),
        ];

        assert_eq!(filter_relative_paths(root, paths), vec![PathBuf::from("note.md")]);
    }
}
