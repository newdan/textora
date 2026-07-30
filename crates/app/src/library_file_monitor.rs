use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TEXTORA_SAVE_TEMP_PREFIX: &str = ".textora-save-";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MonitorError {
    WatchFailed { message: String },
    WorkerUnavailable,
}

impl fmt::Display for MonitorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WatchFailed { message } => {
                write!(formatter, "file monitor watch failed: {message}")
            }
            Self::WorkerUnavailable => formatter.write_str("file monitor worker is unavailable"),
        }
    }
}

impl std::error::Error for MonitorError {}

pub(crate) struct ExternalPathBatch {
    pub(crate) paths: BTreeSet<PathBuf>,
    pub(crate) observed_at: Instant,
}

pub(crate) struct LibraryFileMonitor {
    command_sender: mpsc::Sender<WorkerMessage>,
    result_receiver: mpsc::Receiver<ExternalPathBatch>,
    worker: Option<JoinHandle<()>>,
}

impl LibraryFileMonitor {
    pub(crate) fn spawn(wake: impl Fn() + Send + 'static) -> Result<Self, MonitorError> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let event_sender = command_sender.clone();
        let watcher = RecommendedWatcher::new(
            move |event| {
                let _ = event_sender.send(WorkerMessage::Notify(event));
            },
            Config::default(),
        )
        .map_err(|error| MonitorError::WatchFailed { message: error.to_string() })?;
        let worker = thread::Builder::new()
            .name("textora-library-file-monitor".to_owned())
            .spawn(move || monitor_loop(watcher, command_receiver, result_sender, wake))
            .map_err(|error| MonitorError::WatchFailed { message: error.to_string() })?;

        Ok(Self { command_sender, result_receiver, worker: Some(worker) })
    }

    pub(crate) fn replace_roots(&self, roots: Vec<PathBuf>) -> Result<(), MonitorError> {
        let (response_sender, response_receiver) = mpsc::channel();
        self.command_sender
            .send(WorkerMessage::ReplaceRoots { roots, response_sender })
            .map_err(|_| MonitorError::WorkerUnavailable)?;
        response_receiver.recv().map_err(|_| MonitorError::WorkerUnavailable)?
    }

    pub(crate) fn try_recv(&self) -> Option<ExternalPathBatch> {
        self.result_receiver.try_recv().ok()
    }

    pub(crate) fn shutdown(mut self) {
        let _ = self.command_sender.send(WorkerMessage::Shutdown);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("library file monitor worker should stop cleanly");
        }
    }
}

impl Drop for LibraryFileMonitor {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.command_sender.send(WorkerMessage::Shutdown);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

enum WorkerMessage {
    ReplaceRoots { roots: Vec<PathBuf>, response_sender: mpsc::Sender<Result<(), MonitorError>> },
    Notify(notify::Result<notify::Event>),
    Shutdown,
}

fn monitor_loop<W: Watcher>(
    mut watcher: W,
    command_receiver: mpsc::Receiver<WorkerMessage>,
    result_sender: mpsc::Sender<ExternalPathBatch>,
    wake: impl Fn() + Send + 'static,
) {
    let mut watched_roots = BTreeSet::new();
    let mut pending_paths = PendingPaths::new();

    loop {
        let timeout = pending_paths
            .time_until_flush()
            .unwrap_or(WORKER_POLL_INTERVAL)
            .min(WORKER_POLL_INTERVAL);
        match command_receiver.recv_timeout(timeout) {
            Ok(WorkerMessage::ReplaceRoots { roots, response_sender }) => {
                let result = replace_watched_roots(&mut watcher, &mut watched_roots, roots);
                let _ = response_sender.send(result);
            }
            Ok(WorkerMessage::Notify(Ok(event))) => {
                for path in event.paths {
                    if !should_ignore_path(&path) {
                        pending_paths.insert(path);
                    }
                }
            }
            Ok(WorkerMessage::Notify(Err(_error))) => {}
            Ok(WorkerMessage::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if let Some(paths) = pending_paths.take_if_due()
            && result_sender.send(ExternalPathBatch { paths, observed_at: Instant::now() }).is_ok()
        {
            wake();
        }
    }
}

fn replace_watched_roots<W: Watcher>(
    watcher: &mut W,
    watched_roots: &mut BTreeSet<PathBuf>,
    roots: Vec<PathBuf>,
) -> Result<(), MonitorError> {
    for root in watched_roots.iter() {
        watcher
            .unwatch(root)
            .map_err(|error| MonitorError::WatchFailed { message: error.to_string() })?;
    }
    let roots = roots.into_iter().collect::<BTreeSet<_>>();
    for root in &roots {
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| MonitorError::WatchFailed { message: error.to_string() })?;
    }
    *watched_roots = roots;
    Ok(())
}

struct PendingPaths {
    paths: BTreeSet<PathBuf>,
    flush_at: Option<Instant>,
}

impl PendingPaths {
    fn new() -> Self {
        Self { paths: BTreeSet::new(), flush_at: None }
    }

    fn insert(&mut self, path: PathBuf) {
        self.paths.insert(path);
        self.flush_at = Some(Instant::now() + DEBOUNCE_WINDOW);
    }

    fn len(&self) -> usize {
        self.paths.len()
    }

    fn time_until_flush(&self) -> Option<Duration> {
        self.flush_at.map(|flush_at| flush_at.saturating_duration_since(Instant::now()))
    }

    fn take_if_due(&mut self) -> Option<BTreeSet<PathBuf>> {
        if self.flush_at.is_some_and(|flush_at| Instant::now() >= flush_at) {
            self.flush_at = None;
            return Some(std::mem::take(&mut self.paths));
        }
        None
    }
}

fn should_ignore_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(TEXTORA_SAVE_TEMP_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::PollWatcher;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn production_source() -> &'static str {
        include_str!("library_file_monitor.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("library monitor production source should precede tests")
    }

    #[test]
    fn production_monitor_uses_platform_event_backend() {
        let source = production_source();
        for forbidden_parts in
            [["Poll", "Watcher"], ["with_poll", "_interval"], ["with_compare", "_contents"]]
        {
            let forbidden = forbidden_parts.concat();
            assert!(
                !source.contains(&forbidden),
                "production monitor must not contain {forbidden}"
            );
        }
        assert!(source.contains("RecommendedWatcher"));
    }

    fn wait_for_path(monitor: &LibraryFileMonitor, path: &PathBuf) -> bool {
        wait_for_path_with_timeout(monitor, path, Duration::from_secs(3))
    }

    fn wait_for_path_with_timeout(
        monitor: &LibraryFileMonitor,
        path: &PathBuf,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(batch) = monitor.try_recv()
                && batch.paths.contains(path)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn write_until_observed(monitor: &LibraryFileMonitor, path: &PathBuf, contents: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            std::fs::write(path, contents).expect("file write should succeed");
            if wait_for_path_with_timeout(monitor, path, Duration::from_millis(250)) {
                return true;
            }
        }
        false
    }

    fn spawn_poll_monitor_for_test() -> LibraryFileMonitor {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let event_sender = command_sender.clone();
        let watcher = PollWatcher::new(
            move |event| {
                let _ = event_sender.send(WorkerMessage::Notify(event));
            },
            Config::default().with_poll_interval(Duration::from_millis(50)),
        )
        .expect("poll watcher should start for tests");
        let worker = thread::Builder::new()
            .name("textora-library-file-monitor-test".to_owned())
            .spawn(move || monitor_loop(watcher, command_receiver, result_sender, || {}))
            .expect("test monitor worker should start");

        LibraryFileMonitor { command_sender, result_receiver, worker: Some(worker) }
    }

    #[test]
    fn monitor_reports_recursive_file_changes() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let monitored_root =
            directory.path().canonicalize().expect("temporary directory should canonicalize");
        let nested = monitored_root.join("nested");
        std::fs::create_dir(&nested).expect("nested directory should exist");
        let path = nested.join("notes.md");
        let monitor = spawn_poll_monitor_for_test();
        monitor.replace_roots(vec![monitored_root]).expect("root should be watched");

        assert!(write_until_observed(&monitor, &path, "created"));
        assert!(write_until_observed(&monitor, &path, "modified"));
        let renamed = nested.join("renamed.md");
        std::fs::rename(&path, &renamed).expect("file should be renamed");
        assert!(wait_for_path(&monitor, &path) || wait_for_path(&monitor, &renamed));
        std::fs::remove_file(&renamed).expect("file should be removed");
        assert!(wait_for_path(&monitor, &renamed));
        monitor.shutdown();
    }

    #[test]
    fn monitor_debounces_repeated_paths_into_one_batch() {
        let mut pending = PendingPaths::new();
        let path = PathBuf::from("/tmp/notes.md");
        pending.insert(path.clone());
        pending.insert(path.clone());
        pending.insert(PathBuf::from("/tmp/other.md"));

        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn monitor_ignores_textora_save_temporary_files() {
        assert!(should_ignore_path(PathBuf::from("/tmp/.textora-save-1-notes.md.tmp").as_path()));
        assert!(!should_ignore_path(PathBuf::from("/tmp/.textora-conflict-1-notes.md").as_path()));
    }
}
