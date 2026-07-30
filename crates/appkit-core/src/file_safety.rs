use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

pub use core::disk_revision::DiskRevision;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileSafetyNotice {
    CleanDocumentReloaded { path: PathBuf },
    ConflictCopyCreated { original: PathBuf, conflict: PathBuf },
    DocumentDetachedAfterDeletion { original: PathBuf },
    ConflictCopyFailed { original: PathBuf, message: String },
    AmbiguousRename { original: PathBuf },
}

#[derive(Debug)]
pub enum FileSafetyError {
    Io { operation: &'static str, source: std::io::Error },
    ConcurrentModification,
    InvalidPath { operation: &'static str },
}

impl fmt::Display for FileSafetyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => {
                write!(formatter, "file safety {operation} failed: {source}")
            }
            Self::ConcurrentModification => {
                formatter.write_str("file changed while an atomic save was being prepared")
            }
            Self::InvalidPath { operation } => {
                write!(formatter, "file safety received an invalid path during {operation}")
            }
        }
    }
}

impl std::error::Error for FileSafetyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::ConcurrentModification | Self::InvalidPath { .. } => None,
        }
    }
}

const CONFLICT_FILE_PREFIX: &str = ".textora-conflict-";
const MAX_CONFLICT_NAME_ATTEMPTS: u32 = 100;

pub fn create_conflict_copy(
    original: &Path,
    bytes: &[u8],
    local_device_short_id: &str,
) -> Result<PathBuf, FileSafetyError> {
    let parent = original
        .parent()
        .ok_or(FileSafetyError::InvalidPath { operation: "create conflict copy" })?;
    let stem = original
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or(FileSafetyError::InvalidPath { operation: "create conflict copy" })?;
    let extension = original.extension().and_then(|extension| extension.to_str());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FileSafetyError::InvalidPath { operation: "create conflict copy" })?
        .as_secs();
    let timestamp = conflict_timestamp(timestamp);
    for attempt in 0..MAX_CONFLICT_NAME_ATTEMPTS {
        let suffix = if attempt == 0 {
            format!("{timestamp}-{local_device_short_id}")
        } else {
            format!("{timestamp}-{local_device_short_id}-{attempt}")
        };
        let filename = match extension {
            Some(extension) => format!("{stem}{CONFLICT_FILE_PREFIX}{suffix}.{extension}"),
            None => format!("{stem}{CONFLICT_FILE_PREFIX}{suffix}"),
        };
        let path = parent.join(filename);
        let result = OpenOptions::new().write(true).create_new(true).open(&path);
        let mut file = match result {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(FileSafetyError::Io { operation: "create conflict copy", source });
            }
        };
        if let Err(source) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&path);
            return Err(FileSafetyError::Io { operation: "write conflict copy", source });
        }
        return Ok(path);
    }
    Err(FileSafetyError::Io {
        operation: "create conflict copy",
        source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "conflict name exhausted"),
    })
}

fn conflict_timestamp(timestamp: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86_400;
    const SECONDS_PER_HOUR: u64 = 3_600;
    const SECONDS_PER_MINUTE: u64 = 60;
    let days = (timestamp / SECONDS_PER_DAY) as i64;
    let seconds_today = timestamp % SECONDS_PER_DAY;
    let hours = seconds_today / SECONDS_PER_HOUR;
    let minutes = (seconds_today % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let seconds = seconds_today % SECONDS_PER_MINUTE;

    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 { shifted_days } else { shifted_days - 146_096 } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    format!("{year:04}{month:02}{day:02}-{hours:02}{minutes:02}{seconds:02}")
}

#[derive(Debug)]
pub enum FileSafetyOutcome {
    Unchanged,
    Reload { content: String, revision: DiskRevision },
    Conflict { conflict: PathBuf, content: String, revision: DiskRevision },
    Renamed { new_path: PathBuf, revision: DiskRevision },
    AmbiguousRename { original: PathBuf },
    Deleted,
}

pub struct FileSafetyMonitor {
    revisions: BTreeMap<PathBuf, DiskRevision>,
}

impl Default for FileSafetyMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSafetyMonitor {
    pub fn new() -> Self {
        Self { revisions: BTreeMap::new() }
    }

    pub fn track(&mut self, path: &Path) -> Result<DiskRevision, FileSafetyError> {
        let revision = capture_revision(path)?;
        self.revisions.insert(path.to_owned(), revision.clone());
        Ok(revision)
    }

    pub fn untrack(&mut self, path: &Path) {
        self.revisions.remove(path);
    }

    pub fn confirm(&mut self, revision: DiskRevision) {
        self.revisions.insert(revision.path.clone(), revision);
    }

    pub fn observe(
        &mut self,
        path: &Path,
        dirty: bool,
        current_content: &str,
        local_device_short_id: &str,
    ) -> Result<FileSafetyOutcome, FileSafetyError> {
        let expected = self
            .revisions
            .get(path)
            .cloned()
            .ok_or(FileSafetyError::InvalidPath { operation: "observe untracked file" })?;
        let current = match read_file_state(path) {
            Ok(current) => current,
            Err(FileSafetyError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return self.handle_missing_path(path, expected);
            }
            Err(error) => return Err(error),
        };
        if expected == current.revision {
            return Ok(FileSafetyOutcome::Unchanged);
        }
        self.confirm(current.revision.clone());
        if !dirty {
            return Ok(FileSafetyOutcome::Reload {
                content: String::from_utf8_lossy(&current.bytes).into_owned(),
                revision: current.revision,
            });
        }
        let conflict =
            create_conflict_copy(path, current_content.as_bytes(), local_device_short_id)?;
        Ok(FileSafetyOutcome::Conflict {
            conflict,
            content: String::from_utf8_lossy(&current.bytes).into_owned(),
            revision: current.revision,
        })
    }

    pub fn reconcile_dirty_snapshot(
        &mut self,
        path: &Path,
        baseline: &DiskRevision,
        current_content: &str,
        local_device_short_id: &str,
    ) -> Result<FileSafetyOutcome, FileSafetyError> {
        self.revisions.insert(path.to_owned(), baseline.clone());
        self.observe(path, true, current_content, local_device_short_id)
    }

    fn handle_missing_path(
        &mut self,
        path: &Path,
        expected: DiskRevision,
    ) -> Result<FileSafetyOutcome, FileSafetyError> {
        let candidates = sibling_file_paths(path);
        match choose_rename_candidate(path, &expected, &candidates)? {
            RenameResolution::Follow(new_path) => {
                let revision = capture_revision(&new_path)?;
                self.revisions.remove(path);
                self.revisions.insert(new_path.clone(), revision.clone());
                Ok(FileSafetyOutcome::Renamed { new_path, revision })
            }
            RenameResolution::Deleted => {
                self.untrack(path);
                Ok(FileSafetyOutcome::Deleted)
            }
            RenameResolution::Ambiguous { original } => {
                self.untrack(path);
                Ok(FileSafetyOutcome::AmbiguousRename { original })
            }
        }
    }
}

pub enum FileSafetyCommand {
    Track {
        path: PathBuf,
    },
    Observe {
        request_id: u64,
        path: PathBuf,
        dirty: bool,
        content_revision: u64,
        current_content: String,
        local_device_short_id: String,
    },
    ReconcileDirtySnapshot {
        request_id: u64,
        path: PathBuf,
        baseline: DiskRevision,
        content_revision: u64,
        current_content: String,
        local_device_short_id: String,
    },
    Shutdown,
}

pub enum FileSafetyResult {
    Tracked {
        path: PathBuf,
        outcome: Result<DiskRevision, FileSafetyError>,
    },
    Observed {
        request_id: u64,
        path: PathBuf,
        dirty: bool,
        content_revision: u64,
        outcome: Result<FileSafetyOutcome, FileSafetyError>,
    },
}

type FileSafetyWake = Arc<dyn Fn() + Send + Sync + 'static>;

pub struct FileSafetyWorker {
    command_sender: mpsc::Sender<FileSafetyCommand>,
    result_receiver: mpsc::Receiver<FileSafetyResult>,
    worker: Option<JoinHandle<()>>,
}

impl FileSafetyWorker {
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let wake = Arc::new(wake);
        let worker = thread::Builder::new()
            .name("textora-file-safety".to_owned())
            .spawn(move || file_safety_worker_loop(command_receiver, result_sender, wake))
            .expect("file safety worker should start");
        Self { command_sender, result_receiver, worker: Some(worker) }
    }

    pub fn submit(&self, command: FileSafetyCommand) -> Result<(), FileSafetyError> {
        self.command_sender.send(command).map_err(|_| FileSafetyError::Io {
            operation: "submit file safety request",
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker stopped"),
        })
    }

    pub fn try_recv(&self) -> Option<FileSafetyResult> {
        self.result_receiver.try_recv().ok()
    }

    pub fn shutdown(mut self) {
        let _ = self.command_sender.send(FileSafetyCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("file safety worker should stop cleanly");
        }
    }
}

impl Drop for FileSafetyWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.command_sender.send(FileSafetyCommand::Shutdown);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn file_safety_worker_loop(
    receiver: mpsc::Receiver<FileSafetyCommand>,
    sender: mpsc::Sender<FileSafetyResult>,
    wake: FileSafetyWake,
) {
    let mut monitor = FileSafetyMonitor::new();
    while let Ok(command) = receiver.recv() {
        match command {
            FileSafetyCommand::Track { path } => {
                let outcome = monitor.track(&path);
                send_file_safety_result(
                    &sender,
                    &wake,
                    FileSafetyResult::Tracked { path, outcome },
                );
            }
            FileSafetyCommand::Observe {
                request_id,
                path,
                dirty,
                content_revision,
                current_content,
                local_device_short_id,
            } => {
                let outcome =
                    monitor.observe(&path, dirty, &current_content, &local_device_short_id);
                send_file_safety_result(
                    &sender,
                    &wake,
                    FileSafetyResult::Observed {
                        request_id,
                        path,
                        dirty,
                        content_revision,
                        outcome,
                    },
                );
            }
            FileSafetyCommand::ReconcileDirtySnapshot {
                request_id,
                path,
                baseline,
                content_revision,
                current_content,
                local_device_short_id,
            } => {
                let outcome = monitor.reconcile_dirty_snapshot(
                    &path,
                    &baseline,
                    &current_content,
                    &local_device_short_id,
                );
                send_file_safety_result(
                    &sender,
                    &wake,
                    FileSafetyResult::Observed {
                        request_id,
                        path,
                        dirty: true,
                        content_revision,
                        outcome,
                    },
                );
            }
            FileSafetyCommand::Shutdown => break,
        }
    }
}

fn send_file_safety_result(
    sender: &mpsc::Sender<FileSafetyResult>,
    wake: &FileSafetyWake,
    result: FileSafetyResult,
) {
    if sender.send(result).is_ok() {
        wake();
    }
}

pub fn capture_revision(path: &Path) -> Result<DiskRevision, FileSafetyError> {
    let revision = core::disk_revision::read_disk_revision(path).map_err(|error| {
        let source = match error {
            core::file::FileError::Io(source) => source,
            core::file::FileError::Binary => {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "binary file detected")
            }
        };
        FileSafetyError::Io { operation: "read file revision", source }
    })?;
    revision.ok_or_else(|| FileSafetyError::Io {
        operation: "read file revision",
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "file does not exist"),
    })
}

pub fn save_if_unchanged(
    path: &Path,
    expected_revision: &DiskRevision,
    expected_buffer_revision: u64,
    current_buffer_revision: u64,
    bytes: &[u8],
) -> Result<DiskRevision, FileSafetyError> {
    if expected_buffer_revision != current_buffer_revision {
        return Err(FileSafetyError::ConcurrentModification);
    }
    let current_revision = capture_revision(path)?;
    if *expected_revision != current_revision {
        return Err(FileSafetyError::ConcurrentModification);
    }
    save_serialized_if_unchanged(path, Some(expected_revision), bytes)
}

/// Atomically write immutable serialized contents against an optional disk baseline.
pub fn save_serialized_if_unchanged(
    path: &Path,
    expected_revision: Option<&DiskRevision>,
    bytes: &[u8],
) -> Result<DiskRevision, FileSafetyError> {
    core::file::save_file_if_unchanged(path, bytes, expected_revision).map_err(
        |error| match error {
            core::file::SaveError::ConcurrentModification { .. } => {
                FileSafetyError::ConcurrentModification
            }
            core::file::SaveError::Io { source, .. } => {
                FileSafetyError::Io { operation: "atomically save file", source }
            }
            core::file::SaveError::ReadOnly => FileSafetyError::Io {
                operation: "atomically save file",
                source: std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "file is read-only",
                ),
            },
        },
    )
}

pub fn choose_rename_candidate(
    original: &Path,
    original_revision: &DiskRevision,
    candidates: &[PathBuf],
) -> Result<RenameResolution, FileSafetyError> {
    let mut matches = Vec::new();
    for candidate in candidates {
        let revision = match capture_revision(candidate) {
            Ok(revision) => revision,
            Err(FileSafetyError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if revision.content_hash == original_revision.content_hash {
            matches.push(candidate.clone());
        }
    }
    match matches.as_slice() {
        [candidate] => Ok(RenameResolution::Follow(candidate.clone())),
        [] => Ok(RenameResolution::Deleted),
        _ => Ok(RenameResolution::Ambiguous { original: original.to_owned() }),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenameResolution {
    Follow(PathBuf),
    Deleted,
    Ambiguous { original: PathBuf },
}

fn read_file_state(path: &Path) -> Result<FileState, FileSafetyError> {
    let bytes = fs::read(path)
        .map_err(|source| FileSafetyError::Io { operation: "read file for revision", source })?;
    let revision = capture_revision(path)?;
    Ok(FileState { revision, bytes })
}

fn sibling_file_paths(path: &Path) -> Vec<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Ok(entries) = fs::read_dir(parent) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            if entry.path() == path {
                return None;
            }
            entry.file_type().ok().filter(|file_type| file_type.is_file()).map(|_| entry.path())
        })
        .collect()
}

struct FileState {
    revision: DiskRevision,
    bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        DiskRevision, FileSafetyCommand, FileSafetyMonitor, FileSafetyOutcome, FileSafetyResult,
        FileSafetyWorker, RenameResolution, capture_revision, choose_rename_candidate,
        create_conflict_copy, save_if_unchanged,
    };

    #[test]
    fn clean_change_reloads_and_updates_the_revision() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("file should be written");
        let mut monitor = FileSafetyMonitor::new();
        let baseline = monitor.track(&path).expect("baseline should capture");
        fs::write(&path, "new").expect("external change should be written");
        let outcome =
            monitor.observe(&path, false, "old", "local").expect("change should be observed");
        assert!(matches!(outcome, FileSafetyOutcome::Reload { content, .. } if content == "new"));
        assert_ne!(capture_revision(&path).expect("revision should capture"), baseline);
    }

    #[test]
    fn dirty_change_creates_conflict_copy_without_overwriting_original() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "disk").expect("file should be written");
        let mut monitor = FileSafetyMonitor::new();
        monitor.track(&path).expect("baseline should capture");
        fs::write(&path, "remote").expect("external change should be written");
        let outcome = monitor
            .observe(&path, true, "local edits", "ABC123")
            .expect("change should be observed");
        let conflict = match outcome {
            FileSafetyOutcome::Conflict { conflict, .. } => conflict,
            other => panic!("expected conflict copy, got {other:?}"),
        };
        let file_name = conflict
            .file_name()
            .and_then(|name| name.to_str())
            .expect("conflict copy should have a UTF-8 file name");
        let conflict_name = file_name
            .strip_prefix("notes.textora-conflict-")
            .and_then(|name| name.strip_suffix(".md"))
            .expect("conflict copy should use the documented name");
        let mut name_parts = conflict_name.split('-');
        let date = name_parts.next().expect("conflict name should include a date");
        let time = name_parts.next().expect("conflict name should include a time");
        let device = name_parts.next().expect("conflict name should include a device");
        assert_eq!(date.len(), 8);
        assert_eq!(time.len(), 6);
        assert!(date.chars().all(|character| character.is_ascii_digit()));
        assert!(time.chars().all(|character| character.is_ascii_digit()));
        assert_eq!(device, "ABC123");
        assert!(name_parts.next().is_none());
        assert_eq!(fs::read_to_string(&path).expect("original should remain"), "remote");
        assert_eq!(
            fs::read_to_string(conflict).expect("conflict copy should exist"),
            "local edits"
        );
    }

    #[test]
    fn deletion_detaches_without_writing_back() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "content").expect("file should be written");
        let mut monitor = FileSafetyMonitor::new();
        monitor.track(&path).expect("baseline should capture");
        fs::remove_file(&path).expect("file should be deleted");
        assert!(matches!(
            monitor.observe(&path, true, "dirty", "ABC123").expect("deletion should be observed"),
            FileSafetyOutcome::Deleted
        ));
        assert!(!path.exists());
    }

    #[test]
    fn unique_same_content_candidate_is_followed_as_a_rename() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("old.md");
        let renamed = directory.path().join("new.md");
        fs::write(&original, "content").expect("original should be written");
        let mut monitor = FileSafetyMonitor::new();
        monitor.track(&original).expect("baseline should capture");
        fs::remove_file(&original).expect("original should be deleted");
        fs::write(&renamed, "content").expect("renamed file should be written");

        let outcome = monitor
            .observe(&original, false, "content", "local")
            .expect("rename should be observed");

        assert!(
            matches!(outcome, FileSafetyOutcome::Renamed { new_path, .. } if new_path == renamed)
        );
    }

    #[test]
    fn ambiguous_same_content_candidates_detach_and_report_ambiguity() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("old.md");
        fs::write(&original, "content").expect("original should be written");
        let mut monitor = FileSafetyMonitor::new();
        monitor.track(&original).expect("baseline should capture");
        fs::remove_file(&original).expect("original should be deleted");
        fs::write(directory.path().join("first.md"), "content")
            .expect("first candidate should be written");
        fs::write(directory.path().join("second.md"), "content")
            .expect("second candidate should be written");

        let outcome = monitor
            .observe(&original, true, "local edits", "local")
            .expect("ambiguous rename should be observed");

        assert!(matches!(
            outcome,
            FileSafetyOutcome::AmbiguousRename { original: path } if path == original
        ));
    }

    #[test]
    fn save_requires_expected_disk_and_buffer_versions() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("file should be written");
        let baseline = capture_revision(&path).expect("baseline should capture");
        let error = save_if_unchanged(&path, &baseline, 7, 8, b"new")
            .expect_err("changed buffer revision should abort save");
        assert!(matches!(error, super::FileSafetyError::ConcurrentModification));
        fs::write(&path, "external").expect("external change should be written");
        let error = save_if_unchanged(&path, &baseline, 7, 7, b"new")
            .expect_err("changed disk revision should abort save");
        assert!(matches!(error, super::FileSafetyError::ConcurrentModification));
        assert_eq!(fs::read_to_string(&path).expect("file should remain"), "external");
    }

    #[test]
    fn reconcile_dirty_snapshot_compares_against_saved_baseline() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "baseline").expect("file should be written");

        let mut monitor = FileSafetyMonitor::new();
        let baseline = capture_revision(&path).expect("baseline should capture");
        fs::write(&path, "remote").expect("external change should be written");

        let outcome = monitor
            .reconcile_dirty_snapshot(&path, &baseline, "local edits", "local")
            .expect("offline change should be reconciled");

        assert!(
            matches!(outcome, FileSafetyOutcome::Conflict { content, .. } if content == "remote")
        );
    }

    #[test]
    fn rename_follows_only_a_unique_content_match() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("old.md");
        let first = directory.path().join("new.md");
        let second = directory.path().join("copy.md");
        fs::write(&original, "same").expect("original should be written");
        fs::write(&first, "same").expect("candidate should be written");
        let revision = capture_revision(&original).expect("revision should capture");
        assert_eq!(
            choose_rename_candidate(&original, &revision, std::slice::from_ref(&first))
                .expect("rename should resolve"),
            RenameResolution::Follow(first.clone())
        );
        fs::write(&second, "same").expect("second candidate should be written");
        assert_eq!(
            choose_rename_candidate(&original, &revision, &[first, second])
                .expect("ambiguous rename should resolve"),
            RenameResolution::Ambiguous { original }
        );
    }

    #[test]
    fn worker_keeps_disk_reads_off_the_calling_thread() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "old").expect("file should be written");
        let worker = FileSafetyWorker::spawn(|| {});
        worker
            .submit(FileSafetyCommand::Track { path: path.clone() })
            .expect("track should submit");
        wait_for_worker_result(&worker, |result| {
            matches!(result, FileSafetyResult::Tracked { .. })
        });
        fs::write(&path, "new").expect("external change should be written");
        worker
            .submit(FileSafetyCommand::Observe {
                request_id: 1,
                path: path.clone(),
                dirty: false,
                content_revision: 0,
                current_content: "old".to_owned(),
                local_device_short_id: "local".to_owned(),
            })
            .expect("observe should submit");
        let result = wait_for_worker_result(&worker, |result| {
            matches!(result, FileSafetyResult::Observed { request_id: 1, .. })
        });
        assert!(matches!(
            result,
            FileSafetyResult::Observed {
                outcome: Ok(FileSafetyOutcome::Reload { content, .. }),
                ..
            } if content == "new"
        ));
        worker.shutdown();
    }

    #[test]
    fn worker_reconciles_dirty_snapshot_against_persisted_baseline() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "baseline").expect("file should be written");
        let baseline = capture_revision(&path).expect("baseline should capture");
        fs::write(&path, "remote").expect("external change should be written");

        let worker = FileSafetyWorker::spawn(|| {});
        worker
            .submit(FileSafetyCommand::ReconcileDirtySnapshot {
                request_id: 7,
                path: path.clone(),
                baseline,
                content_revision: 0,
                current_content: "local".to_owned(),
                local_device_short_id: "local".to_owned(),
            })
            .expect("reconcile should submit");
        let result = wait_for_worker_result(&worker, |result| {
            matches!(result, FileSafetyResult::Observed { request_id: 7, .. })
        });
        assert!(matches!(
            result,
            FileSafetyResult::Observed {
                outcome: Ok(FileSafetyOutcome::Conflict { content, .. }),
                ..
            } if content == "remote"
        ));
        worker.shutdown();
    }

    fn wait_for_worker_result(
        worker: &FileSafetyWorker,
        matches_result: impl Fn(&FileSafetyResult) -> bool,
    ) -> FileSafetyResult {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(result) = worker.try_recv()
                && matches_result(&result)
            {
                return result;
            }
            assert!(std::time::Instant::now() < deadline, "file safety worker timed out");
            std::thread::yield_now();
        }
    }

    #[allow(dead_code)]
    fn _keep_revision_type(_: DiskRevision) {}

    #[test]
    fn creates_collision_safe_copy_with_extension_and_unicode_name() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("笔记.md");

        let first = create_conflict_copy(&original, b"local", "ABC123")
            .expect("first conflict copy should be created");
        let second = create_conflict_copy(&original, b"local-2", "ABC123")
            .expect("second conflict copy should be collision safe");

        assert_ne!(first, second);
        assert!(first.file_name().unwrap().to_string_lossy().contains("笔记.textora-conflict-"));
        assert!(first.extension().is_some_and(|extension| extension == "md"));
        assert_eq!(fs::read(&first).expect("first copy should be readable"), b"local");
        assert_eq!(fs::read(&second).expect("second copy should be readable"), b"local-2");
    }

    #[test]
    fn copy_does_not_overwrite_existing_syncthing_conflict_file() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let original = directory.path().join("notes.md");
        let syncthing_conflict = directory.path().join(".sync-conflict-notes.md");
        fs::write(&syncthing_conflict, b"remote").expect("existing conflict should be written");

        let copy = create_conflict_copy(&original, b"local", "local")
            .expect("Textora copy should be created");

        assert_ne!(copy, syncthing_conflict);
        assert_eq!(fs::read(&syncthing_conflict).expect("existing file should remain"), b"remote");
    }
}
