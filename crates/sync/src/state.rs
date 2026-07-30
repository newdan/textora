use std::time::Duration;

use crate::{FolderPhase, FolderStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionObservation {
    Disabled,
    Connecting,
    Available,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryObservation {
    pub connection: ConnectionObservation,
    pub remote_device_configured: bool,
    pub remote_device_connected: bool,
    pub remote_folder_configured: bool,
    pub folder_phase: Option<FolderPhase>,
    pub folder_status: Option<FolderStatus>,
    pub configuration_drift: bool,
    pub error_summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LibrarySyncState {
    Disabled,
    Connecting,
    Unavailable,
    AwaitingRemoteDevice,
    AwaitingRemoteFolder,
    Scanning,
    Syncing { remaining_bytes: u64, completion_percent: f64 },
    UpToDate,
    Paused,
    ConfigurationMismatch,
    Error { summary: String },
}

pub fn reduce_library_state(observation: &LibraryObservation) -> LibrarySyncState {
    match observation.connection {
        ConnectionObservation::Disabled => return LibrarySyncState::Disabled,
        ConnectionObservation::Connecting => return LibrarySyncState::Connecting,
        ConnectionObservation::Unavailable => return LibrarySyncState::Unavailable,
        ConnectionObservation::Available => {}
    }

    if observation.configuration_drift {
        return LibrarySyncState::ConfigurationMismatch;
    }
    if !observation.remote_device_configured || !observation.remote_device_connected {
        return LibrarySyncState::AwaitingRemoteDevice;
    }
    if !observation.remote_folder_configured {
        return LibrarySyncState::AwaitingRemoteFolder;
    }
    if let Some(summary) = &observation.error_summary {
        return LibrarySyncState::Error { summary: summary.clone() };
    }

    match observation.folder_phase {
        Some(FolderPhase::Scanning) => LibrarySyncState::Scanning,
        Some(FolderPhase::Syncing) => {
            let (remaining_bytes, completion_percent) = observation
                .folder_status
                .as_ref()
                .map_or((0, 0.0), |status| (status.need_bytes, status.completion_percent));
            LibrarySyncState::Syncing { remaining_bytes, completion_percent }
        }
        Some(FolderPhase::Paused) => LibrarySyncState::Paused,
        Some(FolderPhase::Error) => {
            LibrarySyncState::Error { summary: "Syncthing reported a folder error".to_owned() }
        }
        Some(FolderPhase::Idle) => {
            let is_up_to_date = observation
                .folder_status
                .as_ref()
                .is_some_and(|status| status.need_items == 0 && status.completion_percent >= 100.0);
            if is_up_to_date {
                LibrarySyncState::UpToDate
            } else {
                let (remaining_bytes, completion_percent) = observation
                    .folder_status
                    .as_ref()
                    .map_or((0, 0.0), |status| (status.need_bytes, status.completion_percent));
                LibrarySyncState::Syncing { remaining_bytes, completion_percent }
            }
        }
        Some(FolderPhase::Unknown) | None => LibrarySyncState::Error {
            summary: "Syncthing reported an unknown folder state".to_owned(),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EventCursor(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncEventKind {
    DeviceConnected,
    DeviceDisconnected,
    FolderStateChanged,
    ItemFinished,
    ConfigurationChanged,
    RemoteError,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncEvent {
    Remote { id: u64, kind: SyncEventKind },
    FullRefreshRequired,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteEvent {
    pub id: u64,
    pub kind: Option<SyncEventKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventBatch {
    pub next_cursor: EventCursor,
    pub events: Vec<SyncEvent>,
}

pub fn reduce_event_batch(cursor: EventCursor, remote_events: Vec<RemoteEvent>) -> EventBatch {
    if remote_events.is_empty() {
        return EventBatch { next_cursor: cursor, events: Vec::new() };
    }

    let mut next_cursor = cursor.0;
    let mut expected_id = cursor.0.checked_add(1);
    let mut requires_full_refresh = false;
    let mut events = Vec::new();

    for remote_event in remote_events {
        if remote_event.id <= next_cursor {
            requires_full_refresh = true;
            continue;
        }
        if let Some(expected) = expected_id
            && remote_event.id != expected
        {
            requires_full_refresh = true;
        }
        next_cursor = remote_event.id;
        expected_id = remote_event.id.checked_add(1);
        if let Some(kind) = remote_event.kind {
            events.push(SyncEvent::Remote { id: remote_event.id, kind });
        }
    }

    if requires_full_refresh {
        EventBatch {
            next_cursor: EventCursor(next_cursor.max(cursor.0)),
            events: vec![SyncEvent::FullRefreshRequired],
        }
    } else {
        EventBatch { next_cursor: EventCursor(next_cursor), events }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryFailure {
    Connection,
    Authentication,
    IncompatibleVersion,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackoffAction {
    RetryAfter(Duration),
    Stop,
}

#[derive(Debug, Default)]
pub struct RetryBackoff {
    consecutive_failures: u32,
}

impl RetryBackoff {
    const MAX_FAILURES: u32 = 5;
    const MAX_DELAY_SECONDS: u64 = 30;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
    }

    pub fn on_failure(&mut self, failure: RetryFailure) -> BackoffAction {
        if matches!(failure, RetryFailure::Authentication | RetryFailure::IncompatibleVersion) {
            return BackoffAction::Stop;
        }
        if self.consecutive_failures >= Self::MAX_FAILURES {
            return BackoffAction::Stop;
        }

        self.consecutive_failures += 1;
        let exponent = self.consecutive_failures.saturating_sub(1).min(5);
        let delay_seconds = 2_u64.pow(exponent).min(Self::MAX_DELAY_SECONDS);
        BackoffAction::RetryAfter(Duration::from_secs(delay_seconds))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        BackoffAction, ConnectionObservation, EventCursor, LibraryObservation, LibrarySyncState,
        RemoteEvent, RetryBackoff, RetryFailure, SyncEvent, SyncEventKind, reduce_event_batch,
        reduce_library_state,
    };
    use crate::{FolderPhase, FolderStatus};

    const FOLDER_ID: &str = "notes";

    fn available_observation() -> LibraryObservation {
        LibraryObservation {
            connection: ConnectionObservation::Available,
            remote_device_configured: true,
            remote_device_connected: true,
            remote_folder_configured: true,
            folder_phase: Some(FolderPhase::Idle),
            folder_status: Some(FolderStatus {
                phase: FolderPhase::Idle,
                need_bytes: 0,
                need_items: 0,
                completion_percent: 100.0,
                errors: 0,
            }),
            configuration_drift: false,
            error_summary: None,
        }
    }

    #[test]
    fn reduces_connection_and_provisioning_states_by_fixed_priority() {
        let mut observation = available_observation();
        observation.connection = ConnectionObservation::Disabled;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::Disabled);

        observation.connection = ConnectionObservation::Connecting;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::Connecting);

        observation.connection = ConnectionObservation::Unavailable;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::Unavailable);

        observation.connection = ConnectionObservation::Available;
        observation.remote_device_configured = false;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::AwaitingRemoteDevice);

        observation.remote_device_configured = true;
        observation.remote_device_connected = false;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::AwaitingRemoteDevice);

        observation.remote_device_connected = true;
        observation.remote_folder_configured = false;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::AwaitingRemoteFolder);

        observation.remote_folder_configured = true;
        observation.configuration_drift = true;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::ConfigurationMismatch);
    }

    #[test]
    fn reduces_folder_phases_without_turning_remote_offline_into_an_error() {
        let mut observation = available_observation();
        observation.folder_phase = Some(FolderPhase::Scanning);
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::Scanning);

        observation.folder_phase = Some(FolderPhase::Syncing);
        observation.folder_status = Some(FolderStatus {
            phase: FolderPhase::Syncing,
            need_bytes: 10,
            need_items: 2,
            completion_percent: 90.0,
            errors: 0,
        });
        assert_eq!(
            reduce_library_state(&observation),
            LibrarySyncState::Syncing { remaining_bytes: 10, completion_percent: 90.0 }
        );

        observation.folder_phase = Some(FolderPhase::Paused);
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::Paused);

        observation.folder_phase = Some(FolderPhase::Idle);
        observation.folder_status = Some(FolderStatus {
            phase: FolderPhase::Idle,
            need_bytes: 0,
            need_items: 0,
            completion_percent: 100.0,
            errors: 0,
        });
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::UpToDate);

        observation.error_summary = Some("remote error".to_owned());
        assert_eq!(
            reduce_library_state(&observation),
            LibrarySyncState::Error { summary: "remote error".to_owned() }
        );

        observation.error_summary = None;
        observation.remote_device_connected = false;
        assert_eq!(reduce_library_state(&observation), LibrarySyncState::AwaitingRemoteDevice);
    }

    #[test]
    fn event_reduction_preserves_empty_cursor_and_detects_gaps_or_rollbacks() {
        let cursor = EventCursor(4);
        let empty = reduce_event_batch(cursor, Vec::new());
        assert_eq!(empty.next_cursor, cursor);
        assert!(empty.events.is_empty());

        let unknown = reduce_event_batch(cursor, vec![RemoteEvent { id: 5, kind: None }]);
        assert_eq!(unknown.next_cursor, EventCursor(5));
        assert!(unknown.events.is_empty());

        let contiguous = reduce_event_batch(
            cursor,
            vec![RemoteEvent { id: 5, kind: Some(SyncEventKind::FolderStateChanged) }],
        );
        assert_eq!(contiguous.next_cursor, EventCursor(5));
        assert_eq!(
            contiguous.events,
            vec![SyncEvent::Remote { id: 5, kind: SyncEventKind::FolderStateChanged }]
        );

        let gap = reduce_event_batch(
            cursor,
            vec![RemoteEvent { id: 7, kind: Some(SyncEventKind::ItemFinished) }],
        );
        assert_eq!(gap.next_cursor, EventCursor(7));
        assert_eq!(gap.events, vec![SyncEvent::FullRefreshRequired]);

        let rollback = reduce_event_batch(
            cursor,
            vec![RemoteEvent { id: 3, kind: Some(SyncEventKind::RemoteError) }],
        );
        assert_eq!(rollback.next_cursor, cursor);
        assert_eq!(rollback.events, vec![SyncEvent::FullRefreshRequired]);
    }

    #[test]
    fn retry_backoff_is_bounded_and_stops_for_non_retryable_failures() {
        let mut backoff = RetryBackoff::new();
        assert_eq!(
            backoff.on_failure(RetryFailure::Connection),
            BackoffAction::RetryAfter(Duration::from_secs(1))
        );
        assert_eq!(
            backoff.on_failure(RetryFailure::Connection),
            BackoffAction::RetryAfter(Duration::from_secs(2))
        );
        for _ in 0..3 {
            let _ = backoff.on_failure(RetryFailure::Connection);
        }
        assert_eq!(backoff.on_failure(RetryFailure::Connection), BackoffAction::Stop);
        assert_eq!(backoff.on_failure(RetryFailure::Authentication), BackoffAction::Stop);
        backoff.reset();
        assert_eq!(
            backoff.on_failure(RetryFailure::Other),
            BackoffAction::RetryAfter(Duration::from_secs(1))
        );
        let _ = FOLDER_ID;
    }
}
