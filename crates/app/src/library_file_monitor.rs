use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
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

#[derive(Debug)]
pub(crate) enum MonitorEvent {
    PathsChanged(BTreeSet<PathBuf>),
    RootsUpdated,
    Failed(MonitorError),
}

enum RequestedRoots {
    Active(BTreeSet<PathBuf>),
    Stopped,
}

pub(crate) struct LibraryFileMonitor {
    command_sender: mpsc::Sender<WorkerMessage>,
    result_receiver: mpsc::Receiver<MonitorEvent>,
    requested_roots: Arc<Mutex<RequestedRoots>>,
    worker: Option<JoinHandle<()>>,
}

impl LibraryFileMonitor {
    pub(crate) fn spawn(wake: impl Fn() + Send + 'static) -> Result<Self, MonitorError> {
        Self::spawn_with_backend(wake, |event_sender| {
            RecommendedWatcher::new(
                move |event| {
                    let _ = event_sender.send(WorkerMessage::Notify(event));
                },
                Config::default(),
            )
        })
    }

    fn spawn_with_backend<W: Watcher + 'static>(
        wake: impl Fn() + Send + 'static,
        create_watcher: impl FnOnce(mpsc::Sender<WorkerMessage>) -> notify::Result<W> + Send + 'static,
    ) -> Result<Self, MonitorError> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let requested_roots = Arc::new(Mutex::new(RequestedRoots::Active(BTreeSet::new())));
        let worker_roots = Arc::clone(&requested_roots);
        let event_sender = command_sender.clone();
        let worker = thread::Builder::new()
            .name("textora-library-file-monitor".to_owned())
            .spawn(move || match create_watcher(event_sender) {
                Ok(watcher) => {
                    monitor_loop(watcher, command_receiver, worker_roots, result_sender, wake);
                }
                Err(error) => {
                    publish_event(&result_sender, &wake, MonitorEvent::Failed(error.into()))
                }
            })
            .map_err(|error| MonitorError::WatchFailed { message: error.to_string() })?;
        Ok(Self { command_sender, result_receiver, requested_roots, worker: Some(worker) })
    }

    pub(crate) fn replace_roots(&self, roots: Vec<PathBuf>) -> Result<(), MonitorError> {
        {
            let mut requested =
                self.requested_roots.lock().expect("root request lock must be healthy");
            let RequestedRoots::Active(current) = &mut *requested else {
                return Err(MonitorError::WorkerUnavailable);
            };
            *current = roots.into_iter().collect();
        }
        self.command_sender
            .send(WorkerMessage::RefreshRoots)
            .map_err(|_| MonitorError::WorkerUnavailable)
    }

    pub(crate) fn try_recv(&self) -> Option<MonitorEvent> {
        self.result_receiver.try_recv().ok()
    }

    pub(crate) fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        *self.requested_roots.lock().expect("root request lock must be healthy") =
            RequestedRoots::Stopped;
        let _ = self.command_sender.send(WorkerMessage::Shutdown);
        // A platform registration/destructor can block inside the OS. The worker owns
        // its backend and exits after that call returns; window teardown must not join it.
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for LibraryFileMonitor {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.stop();
        }
    }
}

impl From<notify::Error> for MonitorError {
    fn from(error: notify::Error) -> Self {
        Self::WatchFailed { message: error.to_string() }
    }
}

enum WorkerMessage {
    RefreshRoots,
    Notify(notify::Result<notify::Event>),
    Shutdown,
}

fn publish_event(sender: &mpsc::Sender<MonitorEvent>, wake: &impl Fn(), event: MonitorEvent) {
    if sender.send(event).is_ok() {
        wake();
    }
}

fn monitor_loop<W: Watcher>(
    mut watcher: W,
    command_receiver: mpsc::Receiver<WorkerMessage>,
    requested_roots: Arc<Mutex<RequestedRoots>>,
    result_sender: mpsc::Sender<MonitorEvent>,
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
            Ok(WorkerMessage::RefreshRoots) => {
                let Some(roots) = latest_requested_roots(&requested_roots) else { break };
                let event = match replace_watched_roots(&mut watcher, &mut watched_roots, roots) {
                    Ok(true) => Some(MonitorEvent::RootsUpdated),
                    Ok(false) => None,
                    Err(error) => Some(MonitorEvent::Failed(error)),
                };
                if let Some(event) = event {
                    publish_event(&result_sender, &wake, event);
                }
            }
            Ok(WorkerMessage::Notify(Ok(event))) => {
                for path in event.paths {
                    if !should_ignore_path(&path) {
                        pending_paths.insert(path);
                    }
                }
            }
            Ok(WorkerMessage::Notify(Err(error))) => {
                publish_event(&result_sender, &wake, MonitorEvent::Failed(error.into()));
            }
            Ok(WorkerMessage::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(paths) = pending_paths.take_if_due() {
            publish_event(&result_sender, &wake, MonitorEvent::PathsChanged(paths));
        }
    }
}

fn latest_requested_roots(requested_roots: &Mutex<RequestedRoots>) -> Option<Vec<PathBuf>> {
    match &*requested_roots.lock().expect("root request lock must be healthy") {
        RequestedRoots::Active(roots) => Some(roots.iter().cloned().collect()),
        RequestedRoots::Stopped => None,
    }
}

fn replace_watched_roots<W: Watcher>(
    watcher: &mut W,
    watched_roots: &mut BTreeSet<PathBuf>,
    roots: Vec<PathBuf>,
) -> Result<bool, MonitorError> {
    let requested = roots.into_iter().collect::<BTreeSet<_>>();
    let removed = watched_roots.difference(&requested).cloned().collect::<Vec<_>>();
    let added = requested.difference(watched_roots).cloned().collect::<Vec<_>>();
    let changed = !removed.is_empty() || !added.is_empty();
    if !changed {
        return Ok(false);
    }
    let mut subscriptions = watcher.paths_mut();
    let update = apply_subscription_changes(subscriptions.as_mut(), watched_roots, removed, added);
    // Commit successful changes even if a later path failed, so a batch backend
    // restarts the retained subscriptions rather than remaining stopped.
    let commit = subscriptions.commit().map_err(MonitorError::from);
    update.and(commit)?;
    Ok(true)
}

fn apply_subscription_changes(
    subscriptions: &mut dyn notify::PathsMut,
    watched_roots: &mut BTreeSet<PathBuf>,
    removed: Vec<PathBuf>,
    added: Vec<PathBuf>,
) -> Result<(), MonitorError> {
    for root in removed {
        subscriptions.remove(&root)?;
        watched_roots.remove(&root);
    }
    for root in added {
        subscriptions.add(&root, RecursiveMode::Recursive)?;
        watched_roots.insert(root);
    }
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

    #[derive(Default)]
    struct RecordingWatcher {
        operations: Vec<(bool, PathBuf)>,
        commit_count: usize,
        operation_sender: Option<mpsc::Sender<(bool, PathBuf)>>,
        rejected_path: Option<PathBuf>,
        registration_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    }

    impl RecordingWatcher {
        fn record_operation(&mut self, watching: bool, path: &Path) {
            let operation = (watching, path.to_owned());
            self.operations.push(operation.clone());
            if let Some(sender) = &self.operation_sender {
                let _ = sender.send(operation);
            }
        }
    }

    struct RecordingSubscriptions<'a>(&'a mut RecordingWatcher);

    impl notify::PathsMut for RecordingSubscriptions<'_> {
        fn add(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
            self.0.watch(path, mode)
        }

        fn remove(&mut self, path: &Path) -> notify::Result<()> {
            self.0.unwatch(path)
        }

        fn commit(self: Box<Self>) -> notify::Result<()> {
            self.0.commit_count += 1;
            Ok(())
        }
    }

    impl Watcher for RecordingWatcher {
        fn new<F: notify::EventHandler>(_: F, _: Config) -> notify::Result<Self> {
            Ok(Self::default())
        }

        fn watch(&mut self, path: &Path, _: RecursiveMode) -> notify::Result<()> {
            if let Some((entered, release)) = self.registration_gate.take() {
                entered.send(()).expect("registration should report entry");
                release.recv().expect("test should release registration");
            }
            if self.rejected_path.as_deref() == Some(path) {
                return Err(notify::Error::generic("injected registration failure"));
            }
            self.record_operation(true, path);
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
            self.record_operation(false, path);
            Ok(())
        }

        fn paths_mut(&mut self) -> Box<dyn notify::PathsMut + '_> {
            Box::new(RecordingSubscriptions(self))
        }

        fn kind() -> notify::WatcherKind {
            notify::WatcherKind::PollWatcher
        }
    }

    #[test]
    fn unchanged_roots_keep_existing_subscriptions() {
        let retained = PathBuf::from("/retained");
        let removed = PathBuf::from("/removed");
        let added = PathBuf::from("/added");
        let mut watched = BTreeSet::from([retained.clone(), removed.clone()]);
        let mut watcher = RecordingWatcher::default();
        replace_watched_roots(&mut watcher, &mut watched, vec![retained.clone(), added.clone()])
            .expect("changed roots should update");
        assert_eq!(watcher.operations, vec![(false, removed), (true, added.clone())]);
        assert_eq!(watcher.commit_count, 1, "changed paths should commit as one batch");
        watcher.operations.clear();
        replace_watched_roots(&mut watcher, &mut watched, vec![added, retained])
            .expect("identical roots should succeed");
        assert!(watcher.operations.is_empty());
        assert_eq!(watcher.commit_count, 1, "unchanged roots must not restart the backend");
    }

    #[test]
    fn partial_registration_failure_preserves_actual_subscription_state() {
        let retained = PathBuf::from("/retained");
        let removed = PathBuf::from("/removed");
        let added = PathBuf::from("/added");
        let rejected = PathBuf::from("/rejected");
        let mut watched = BTreeSet::from([retained.clone(), removed]);
        let mut watcher = RecordingWatcher {
            rejected_path: Some(rejected.clone()),
            ..RecordingWatcher::default()
        };
        assert!(
            replace_watched_roots(
                &mut watcher,
                &mut watched,
                vec![retained.clone(), added.clone(), rejected]
            )
            .is_err()
        );
        assert_eq!(watched, BTreeSet::from([retained, added]));
        assert_eq!(watcher.commit_count, 1, "partial updates must restart retained subscriptions");
    }

    #[test]
    fn root_requests_return_while_platform_registration_is_blocked() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let watcher = RecordingWatcher {
            registration_gate: Some((entered_sender, release_receiver)),
            ..RecordingWatcher::default()
        };
        let monitor = LibraryFileMonitor::spawn_with_backend(|| {}, move |_| Ok(watcher))
            .expect("test monitor should start");
        let (completed_sender, completed_receiver) = mpsc::channel();
        let (stopped_sender, stopped_receiver) = mpsc::channel();
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();
        let caller = thread::spawn(move || {
            let result = monitor.replace_roots(vec![PathBuf::from("/blocked")]);
            completed_sender.send(result).expect("request outcome should be observed");
            shutdown_receiver.recv().expect("registration must enter before shutdown");
            monitor.shutdown();
            stopped_sender.send(()).expect("shutdown should report completion");
        });
        entered_receiver.recv_timeout(Duration::from_secs(2)).expect("registration should start");
        let outcome = completed_receiver.recv_timeout(Duration::from_secs(1));
        shutdown_sender.send(()).expect("test should request shutdown after registration enters");
        let stopped = stopped_receiver.recv_timeout(Duration::from_secs(1));
        release_sender.send(()).expect("blocked registration should be released");
        caller.join().expect("caller should finish after release");
        assert!(outcome.is_ok(), "root request must not wait for platform registration");
        assert!(stopped.is_ok(), "shutdown must not wait for platform registration");
    }

    #[test]
    fn pending_root_requests_use_latest_snapshot() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (operation_sender, operation_receiver) = mpsc::channel();
        let watcher = RecordingWatcher {
            registration_gate: Some((entered_sender, release_receiver)),
            operation_sender: Some(operation_sender),
            ..RecordingWatcher::default()
        };
        let monitor = LibraryFileMonitor::spawn_with_backend(|| {}, move |_| Ok(watcher))
            .expect("test monitor should start");
        monitor.replace_roots(vec![PathBuf::from("/blocked")]).expect("first request should queue");
        entered_receiver.recv_timeout(Duration::from_secs(2)).expect("registration should start");
        monitor
            .replace_roots(vec![PathBuf::from("/obsolete")])
            .expect("obsolete request should queue");
        monitor.replace_roots(vec![PathBuf::from("/latest")]).expect("latest request should queue");
        release_sender.send(()).expect("registration should be released");
        let operations = (0..3)
            .map(|_| {
                operation_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .expect("subscription should update")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            operations,
            vec![
                (true, PathBuf::from("/blocked")),
                (false, PathBuf::from("/blocked")),
                (true, PathBuf::from("/latest")),
            ]
        );
        monitor.shutdown();
    }

    #[test]
    fn backend_initialization_does_not_block_caller_and_reports_failure() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (wake_sender, wake_receiver) = mpsc::channel();
        let monitor = LibraryFileMonitor::spawn_with_backend(
            move || {
                let _ = wake_sender.send(());
            },
            move |_| -> notify::Result<RecordingWatcher> {
                entered_sender.send(()).expect("backend should start");
                release_receiver.recv().expect("test should release initialization");
                Err(notify::Error::generic("injected initialization failure"))
            },
        )
        .expect("worker should start without waiting for backend");
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("backend should enter initialization");
        monitor
            .replace_roots(vec![PathBuf::from("/queued")])
            .expect("request should queue during initialization");
        release_sender.send(()).expect("initialization should be released");
        wake_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("initialization failure should wake UI");
        assert!(matches!(
            monitor.try_recv(),
            Some(MonitorEvent::Failed(MonitorError::WatchFailed { .. }))
        ));
        monitor.shutdown();
    }

    #[test]
    fn completed_registration_wakes_ui_for_file_safety_recheck() {
        let (wake_sender, wake_receiver) = mpsc::channel();
        let monitor = LibraryFileMonitor::spawn_with_backend(
            move || {
                let _ = wake_sender.send(());
            },
            |_| Ok(RecordingWatcher::default()),
        )
        .expect("test monitor should start");
        monitor.replace_roots(vec![PathBuf::from("/watched")]).expect("root should queue");
        wake_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("registration completion should wake UI");
        assert!(matches!(monitor.try_recv(), Some(MonitorEvent::RootsUpdated)));
        monitor.shutdown();
    }

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
            if let Some(MonitorEvent::PathsChanged(paths)) = monitor.try_recv()
                && paths.contains(path)
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
        LibraryFileMonitor::spawn_with_backend(
            || {},
            |event_sender| {
                PollWatcher::new(
                    move |event| {
                        let _ = event_sender.send(WorkerMessage::Notify(event));
                    },
                    Config::default().with_poll_interval(Duration::from_millis(50)),
                )
            },
        )
        .expect("poll watcher should start for tests")
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
