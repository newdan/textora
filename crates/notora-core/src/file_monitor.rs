use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::event::{ModifyKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::WORKSPACE_METADATA_DIRECTORY_NAME;

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);
const MACOS_FINDER_METADATA_FILE_NAME: &str = ".DS_Store";
const MACOS_RESOURCE_FORK_PREFIX: &str = "._";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileBatch {
    pub relative_paths: Vec<PathBuf>,
    pub changes: Vec<WorkspaceFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceFileChange {
    Renamed { from: PathBuf, to: PathBuf, tracker: Option<u64> },
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

#[derive(Debug)]
pub enum WorkspaceFileMonitorEvent {
    Ready,
    Changes(WorkspaceFileBatch),
    Failed(WorkspaceFileMonitorError),
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
    ) -> Result<(Self, mpsc::Receiver<WorkspaceFileMonitorEvent>), WorkspaceFileMonitorError> {
        Self::start_with_backend(root, |event_sender| {
            RecommendedWatcher::new(
                move |event| {
                    let _ = event_sender.send(event);
                },
                Config::default(),
            )
        })
    }

    fn start_with_backend<W: Watcher + 'static>(
        root: PathBuf,
        create_watcher: impl FnOnce(mpsc::Sender<notify::Result<Event>>) -> notify::Result<W>
        + Send
        + 'static,
    ) -> Result<(Self, mpsc::Receiver<WorkspaceFileMonitorEvent>), WorkspaceFileMonitorError> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("notora-file-monitor".to_owned())
            .spawn(move || run_worker(root, command_receiver, result_sender, create_watcher))
            .map_err(|_| WorkspaceFileMonitorError::WorkerDisconnected)?;

        Ok((Self { command_sender, worker: Some(worker) }, result_receiver))
    }

    pub fn shutdown(&mut self) {
        let _ = self.command_sender.send(MonitorCommand::Shutdown);
        // Platform registration and backend destruction may wait inside the OS.
        // The worker owns those resources; callers only request its eventual exit.
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
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

fn run_worker<W: Watcher>(
    root: PathBuf,
    command_receiver: mpsc::Receiver<MonitorCommand>,
    result_sender: mpsc::Sender<WorkspaceFileMonitorEvent>,
    create_watcher: impl FnOnce(mpsc::Sender<notify::Result<Event>>) -> notify::Result<W>,
) {
    let (event_sender, event_receiver) = mpsc::channel();
    let mut watcher = match create_watcher(event_sender) {
        Ok(watcher) => watcher,
        Err(error) => {
            let _ = result_sender
                .send(WorkspaceFileMonitorEvent::Failed(WorkspaceFileMonitorError::Watcher(error)));
            return;
        }
    };
    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        let _ = result_sender
            .send(WorkspaceFileMonitorEvent::Failed(WorkspaceFileMonitorError::Watcher(error)));
        return;
    }
    if command_receiver.try_recv().is_ok()
        || result_sender.send(WorkspaceFileMonitorEvent::Ready).is_err()
    {
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
    result_sender: &mpsc::Sender<WorkspaceFileMonitorEvent>,
    debounce: Duration,
) {
    let mut pending_events = Vec::new();
    loop {
        match command_receiver.recv_timeout(debounce) {
            Ok(MonitorCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let drained = drain_events(event_receiver, &mut pending_events);
                if !send_batch(root, result_sender, &mut pending_events) {
                    return;
                }
                if let Err(error) = drained {
                    let _ = result_sender.send(WorkspaceFileMonitorEvent::Failed(
                        WorkspaceFileMonitorError::Watcher(error),
                    ));
                    return;
                }
            }
        }
    }
}

fn drain_events(
    event_receiver: &mpsc::Receiver<notify::Result<notify::Event>>,
    pending_events: &mut Vec<Event>,
) -> notify::Result<()> {
    while let Ok(event) = event_receiver.try_recv() {
        pending_events.push(event?);
    }
    Ok(())
}

fn send_batch(
    root: &Path,
    sender: &mpsc::Sender<WorkspaceFileMonitorEvent>,
    pending_events: &mut Vec<Event>,
) -> bool {
    let changes = normalize_changes(root, std::mem::take(pending_events));
    let relative_paths = changes
        .iter()
        .flat_map(change_paths)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if relative_paths.is_empty() {
        return true;
    }
    sender
        .send(WorkspaceFileMonitorEvent::Changes(WorkspaceFileBatch { relative_paths, changes }))
        .is_ok()
}

fn normalize_changes(root: &Path, events: Vec<Event>) -> Vec<WorkspaceFileChange> {
    let mut changes = Vec::new();
    let mut rename_sources = Vec::new();
    let mut rename_targets = Vec::new();
    for event in events {
        let tracker = event.attrs.tracker().and_then(|value| u64::try_from(value).ok());
        match event.kind {
            EventKind::Modify(ModifyKind::Name(RenameMode::Both | RenameMode::Any))
                if event.paths.len() == 2 =>
            {
                let from = relative_path(root, &event.paths[0]);
                let to = relative_path(root, &event.paths[1]);
                if let (Some(from), Some(to)) = (from, to) {
                    push_unique_change(
                        &mut changes,
                        WorkspaceFileChange::Renamed { from, to, tracker },
                    );
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                rename_sources.extend(
                    event
                        .paths
                        .into_iter()
                        .filter_map(|path| relative_path(root, &path).map(|path| (path, tracker))),
                );
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                rename_targets.extend(
                    event
                        .paths
                        .into_iter()
                        .filter_map(|path| relative_path(root, &path).map(|path| (path, tracker))),
                );
            }
            EventKind::Create(_) => {
                push_paths_as_changes(root, event.paths, &mut changes, WorkspaceFileChange::Created)
            }
            EventKind::Remove(_) => {
                push_paths_as_changes(root, event.paths, &mut changes, WorkspaceFileChange::Removed)
            }
            EventKind::Access(_) => {}
            _ => push_paths_as_changes(
                root,
                event.paths,
                &mut changes,
                WorkspaceFileChange::Modified,
            ),
        }
    }
    pair_split_renames(&mut changes, rename_sources, rename_targets);
    changes
}

fn pair_split_renames(
    changes: &mut Vec<WorkspaceFileChange>,
    mut sources: Vec<(PathBuf, Option<u64>)>,
    mut targets: Vec<(PathBuf, Option<u64>)>,
) {
    let mut source_index = 0;
    while source_index < sources.len() {
        let tracker = sources[source_index].1;
        let matching_target = tracker
            .and_then(|tracker| targets.iter().position(|target| target.1 == Some(tracker)))
            .or_else(|| (sources.len() == 1 && targets.len() == 1).then_some(0));
        let Some(target_index) = matching_target else {
            source_index += 1;
            continue;
        };
        let (from, source_tracker) = sources.remove(source_index);
        let (to, target_tracker) = targets.remove(target_index);
        push_unique_change(
            changes,
            WorkspaceFileChange::Renamed { from, to, tracker: source_tracker.or(target_tracker) },
        );
    }
    for (path, _) in sources {
        push_unique_change(changes, WorkspaceFileChange::Removed(path));
    }
    for (path, _) in targets {
        push_unique_change(changes, WorkspaceFileChange::Created(path));
    }
}

fn push_paths_as_changes(
    root: &Path,
    paths: Vec<PathBuf>,
    changes: &mut Vec<WorkspaceFileChange>,
    make_change: impl Fn(PathBuf) -> WorkspaceFileChange,
) {
    for path in paths {
        if let Some(relative_path) = relative_path(root, &path) {
            push_unique_change(changes, make_change(relative_path));
        }
    }
}

fn push_unique_change(changes: &mut Vec<WorkspaceFileChange>, change: WorkspaceFileChange) {
    if !changes.contains(&change) {
        changes.push(change);
    }
}

fn change_paths(change: &WorkspaceFileChange) -> Vec<&PathBuf> {
    match change {
        WorkspaceFileChange::Renamed { from, to, .. } => vec![from, to],
        WorkspaceFileChange::Created(path)
        | WorkspaceFileChange::Modified(path)
        | WorkspaceFileChange::Removed(path) => vec![path],
    }
}

fn relative_path(root: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(root).ok().map(Path::to_path_buf).filter(|path| !should_ignore(path))
}

#[cfg(test)]
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
            || file_name.starts_with(crate::workspace::NOTE_RELOCATION_TEMPORARY_FILE_PREFIX)
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

    use super::{MonitorCommand, WorkspaceFileChange, filter_relative_paths, run_event_batch_loop};

    struct BlockedRegistration {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl notify::Watcher for BlockedRegistration {
        fn new<F: notify::EventHandler>(_: F, _: notify::Config) -> notify::Result<Self> {
            panic!("test backend requires an explicit registration gate");
        }

        fn watch(&mut self, _: &Path, _: notify::RecursiveMode) -> notify::Result<()> {
            self.entered.send(()).expect("registration should report entry");
            self.release.recv().expect("test should release registration");
            Ok(())
        }

        fn unwatch(&mut self, _: &Path) -> notify::Result<()> {
            Ok(())
        }

        fn kind() -> notify::WatcherKind {
            notify::WatcherKind::PollWatcher
        }
    }

    #[test]
    fn workspace_monitor_start_does_not_wait_for_platform_registration() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let watcher = BlockedRegistration { entered: entered_sender, release: release_receiver };
        let (started_sender, started_receiver) = mpsc::channel();
        let caller = thread::spawn(move || {
            let monitor = super::WorkspaceFileMonitor::start_with_backend(
                PathBuf::from("/workspace"),
                move |_| Ok(watcher),
            );
            started_sender.send(monitor).expect("start outcome should be observed");
        });
        entered_receiver.recv_timeout(Duration::from_secs(2)).expect("registration should start");
        let started_while_blocked = started_receiver.recv_timeout(Duration::from_secs(1));
        release_sender.send(()).expect("registration should be released");
        caller.join().expect("start caller should finish");
        let was_nonblocking = started_while_blocked.is_ok();
        let (mut monitor, _) = started_while_blocked
            .or_else(|_| started_receiver.recv_timeout(Duration::from_secs(2)))
            .expect("start should finish after release")
            .expect("test backend should start");
        monitor.shutdown();
        assert!(was_nonblocking, "workspace activation must not wait for platform registration");
    }

    #[test]
    fn workspace_monitor_shutdown_returns_during_blocked_registration() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let watcher = BlockedRegistration { entered: entered_sender, release: release_receiver };
        let (mut monitor, _) = super::WorkspaceFileMonitor::start_with_backend(
            PathBuf::from("/workspace"),
            move |_| Ok(watcher),
        )
        .expect("worker should start");
        entered_receiver.recv_timeout(Duration::from_secs(2)).expect("registration should enter");
        let (stopped_sender, stopped_receiver) = mpsc::channel();
        let caller = thread::spawn(move || {
            monitor.shutdown();
            stopped_sender.send(()).expect("shutdown should report completion");
        });
        let stopped_while_blocked = stopped_receiver.recv_timeout(Duration::from_secs(1));
        release_sender.send(()).expect("test should release registration");
        caller.join().expect("shutdown caller should finish");
        assert!(stopped_while_blocked.is_ok(), "shutdown must not join a blocked platform backend");
    }

    #[test]
    fn workspace_monitor_initialization_failure_is_reported_asynchronously() {
        let (mut monitor, events) = super::WorkspaceFileMonitor::start_with_backend(
            PathBuf::from("/workspace"),
            |_| -> notify::Result<BlockedRegistration> {
                Err(notify::Error::generic("injected initialization failure"))
            },
        )
        .expect("worker creation should succeed before backend initialization");
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(super::WorkspaceFileMonitorEvent::Failed(
                super::WorkspaceFileMonitorError::Watcher(_)
            ))
        ));
        monitor.shutdown();
    }

    #[test]
    fn workspace_monitor_reports_backend_event_errors() {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            run_event_batch_loop(
                Path::new("/workspace"),
                &command_receiver,
                &event_receiver,
                &result_sender,
                Duration::from_millis(10),
            )
        });
        event_sender
            .send(Err(notify::Error::generic("injected stream failure")))
            .expect("backend error should be injected");
        assert!(matches!(
            result_receiver.recv_timeout(Duration::from_secs(2)),
            Ok(super::WorkspaceFileMonitorEvent::Failed(
                super::WorkspaceFileMonitorError::Watcher(_)
            ))
        ));
        drop(command_sender);
        worker.join().expect("failed monitor event loop should stop");
    }

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

        let super::WorkspaceFileMonitorEvent::Changes(batch) = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("rename events should be emitted as one batch")
        else {
            panic!("injected rename events must produce a changes batch");
        };
        assert_eq!(
            batch.relative_paths,
            vec![PathBuf::from("after.md"), PathBuf::from("before.md")]
        );
        assert_eq!(
            batch.changes,
            vec![WorkspaceFileChange::Renamed {
                from: "before.md".into(),
                to: "after.md".into(),
                tracker: None,
            }]
        );
        command_sender.send(MonitorCommand::Shutdown).expect("worker should accept shutdown");
        worker.join().expect("injected event worker should stop cleanly");
    }
}
