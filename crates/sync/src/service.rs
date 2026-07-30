use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{
    DeviceConfig, DeviceId, EventCursor, FolderConfig, FolderId, FolderStatus, InstanceInfo,
    RetryBackoff, RetryFailure, SyncError, SyncEvent, SyncthingClient,
};

pub enum SyncCommand {
    Probe { request_id: u64 },
    Refresh { request_id: u64, folders: Vec<FolderId> },
    ConfigureDevice { request_id: u64, config: DeviceConfig },
    ConfigureFolder { request_id: u64, config: FolderConfig },
    ReadDeviceConfig { request_id: u64, device_id: DeviceId },
    ReadFolderConfig { request_id: u64, folder_id: FolderId },
    ReadPendingFolders { request_id: u64 },
    EnsureManagedIgnores { request_id: u64, folder_id: FolderId },
    RepairManagedIgnores { request_id: u64, folder_id: FolderId },
    RemoveFolder { request_id: u64, folder_id: FolderId },
    PauseFolder { request_id: u64, folder_id: FolderId, paused: bool },
    PauseDevice { request_id: u64, device_id: DeviceId, paused: bool },
    ScanFolder { request_id: u64, folder_id: FolderId },
    Subscribe { cursor: EventCursor },
    Shutdown,
}

pub enum SyncResult {
    Probe { request_id: u64, outcome: Result<InstanceInfo, SyncError> },
    Refresh { request_id: u64, outcome: Result<Vec<(FolderId, FolderStatus)>, SyncError> },
    DeviceConfigured { request_id: u64, outcome: Result<DeviceConfig, SyncError> },
    FolderConfigured { request_id: u64, outcome: Result<FolderConfig, SyncError> },
    DeviceConfigRead { request_id: u64, outcome: Result<Option<DeviceConfig>, SyncError> },
    FolderConfigRead { request_id: u64, outcome: Result<Option<FolderConfig>, SyncError> },
    PendingFoldersRead { request_id: u64, outcome: Result<Vec<crate::PendingFolder>, SyncError> },
    IgnoresEnsured { request_id: u64, outcome: Result<Vec<String>, SyncError> },
    IgnoresRepaired { request_id: u64, outcome: Result<Vec<String>, SyncError> },
    FolderRemoved { request_id: u64, outcome: Result<(), SyncError> },
    FolderPaused { request_id: u64, outcome: Result<FolderConfig, SyncError> },
    DevicePaused { request_id: u64, outcome: Result<DeviceConfig, SyncError> },
    FolderScanned { request_id: u64, outcome: Result<(), SyncError> },
}

trait SyncTransport: Send + Sync + 'static {
    fn probe(&self) -> Result<InstanceInfo, SyncError>;
    fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus, SyncError>;
    fn events_since(
        &self,
        cursor: EventCursor,
        timeout_seconds: u16,
    ) -> Result<Vec<SyncEvent>, SyncError>;
    fn put_device(&self, config: &DeviceConfig) -> Result<DeviceConfig, SyncError>;
    fn put_folder(&self, config: &FolderConfig) -> Result<FolderConfig, SyncError>;
    fn device_config(&self, device_id: &DeviceId) -> Result<Option<DeviceConfig>, SyncError>;
    fn folder_config(&self, folder_id: &FolderId) -> Result<Option<FolderConfig>, SyncError>;
    fn pending_folders(&self) -> Result<Vec<crate::PendingFolder>, SyncError>;
    fn ensure_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError>;
    fn repair_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError>;
    fn remove_folder(&self, folder_id: &FolderId) -> Result<(), SyncError>;
    fn patch_folder_paused(
        &self,
        folder_id: &FolderId,
        paused: bool,
    ) -> Result<FolderConfig, SyncError>;
    fn pause_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError>;
    fn resume_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError>;
    fn scan_folder(&self, folder_id: &FolderId) -> Result<(), SyncError>;
}

struct ClientTransport {
    client: Arc<SyncthingClient>,
}

impl SyncTransport for ClientTransport {
    fn probe(&self) -> Result<InstanceInfo, SyncError> {
        self.client.probe()
    }

    fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus, SyncError> {
        self.client.folder_status(folder)
    }

    fn events_since(
        &self,
        cursor: EventCursor,
        timeout_seconds: u16,
    ) -> Result<Vec<SyncEvent>, SyncError> {
        self.client.events_since(cursor, timeout_seconds)
    }

    fn put_device(&self, config: &DeviceConfig) -> Result<DeviceConfig, SyncError> {
        self.client.put_device(config)
    }

    fn put_folder(&self, config: &FolderConfig) -> Result<FolderConfig, SyncError> {
        self.client.put_folder(config)
    }

    fn device_config(&self, device_id: &DeviceId) -> Result<Option<DeviceConfig>, SyncError> {
        self.client.device_config(device_id)
    }

    fn folder_config(&self, folder_id: &FolderId) -> Result<Option<FolderConfig>, SyncError> {
        self.client.folder_config(folder_id)
    }

    fn pending_folders(&self) -> Result<Vec<crate::PendingFolder>, SyncError> {
        self.client.pending_folders()
    }

    fn ensure_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
        self.client.ensure_managed_ignores(folder_id)
    }

    fn repair_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
        self.client.repair_managed_ignores(folder_id)
    }

    fn remove_folder(&self, folder_id: &FolderId) -> Result<(), SyncError> {
        self.client.remove_folder(folder_id)
    }

    fn patch_folder_paused(
        &self,
        folder_id: &FolderId,
        paused: bool,
    ) -> Result<FolderConfig, SyncError> {
        self.client.patch_folder_paused(folder_id, paused)
    }

    fn pause_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError> {
        self.client.pause_device(device_id)
    }

    fn resume_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError> {
        self.client.resume_device(device_id)
    }

    fn scan_folder(&self, folder_id: &FolderId) -> Result<(), SyncError> {
        self.client.scan_folder(folder_id)
    }
}

type WakeCallback = Arc<dyn Fn() + Send + Sync + 'static>;

pub struct SyncService {
    command_sender: mpsc::Sender<SyncCommand>,
    subscription_sender: mpsc::Sender<Option<EventCursor>>,
    result_receiver: mpsc::Receiver<SyncResult>,
    event_receiver: mpsc::Receiver<SyncEvent>,
    shutdown: Arc<AtomicBool>,
    command_thread: Option<JoinHandle<()>>,
    event_thread: Option<JoinHandle<()>>,
}

impl SyncService {
    pub fn spawn(client: SyncthingClient, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let transport = Arc::new(ClientTransport { client: Arc::new(client) });
        Self::spawn_with_transport(transport, wake)
    }

    fn spawn_with_transport(
        transport: Arc<dyn SyncTransport>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (subscription_sender, subscription_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(wake);

        let command_shutdown = shutdown.clone();
        let command_wake = wake.clone();
        let command_transport = transport.clone();
        let command_thread = thread::Builder::new()
            .name("textora-syncthing-command".to_owned())
            .spawn(move || {
                command_loop(
                    command_transport,
                    command_receiver,
                    result_sender,
                    command_shutdown,
                    command_wake,
                );
            })
            .expect("Syncthing command worker should start");

        let event_shutdown = shutdown.clone();
        let event_wake = wake;
        let event_thread = thread::Builder::new()
            .name("textora-syncthing-event".to_owned())
            .spawn(move || {
                event_loop(
                    transport,
                    subscription_receiver,
                    event_sender,
                    event_shutdown,
                    event_wake,
                );
            })
            .expect("Syncthing event worker should start");

        Self {
            command_sender,
            subscription_sender,
            result_receiver,
            event_receiver,
            shutdown,
            command_thread: Some(command_thread),
            event_thread: Some(event_thread),
        }
    }

    pub fn submit(&self, command: SyncCommand) -> Result<(), SyncError> {
        match command {
            SyncCommand::Subscribe { cursor } => self
                .subscription_sender
                .send(Some(cursor))
                .map_err(|_| SyncError::ConnectionRefused),
            SyncCommand::Shutdown => {
                self.shutdown.store(true, Ordering::Release);
                self.command_sender
                    .send(SyncCommand::Shutdown)
                    .map_err(|_| SyncError::ConnectionRefused)
            }
            command => self.command_sender.send(command).map_err(|_| SyncError::ConnectionRefused),
        }
    }

    pub fn try_recv(&self) -> Option<SyncResult> {
        self.result_receiver.try_recv().ok()
    }

    pub fn try_recv_event(&self) -> Option<SyncEvent> {
        self.event_receiver.try_recv().ok()
    }

    pub fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.command_sender.send(SyncCommand::Shutdown);
        let _ = self.subscription_sender.send(None);
        join_worker(self.command_thread.take());
        join_worker(self.event_thread.take());
    }
}

fn command_loop(
    transport: Arc<dyn SyncTransport>,
    receiver: mpsc::Receiver<SyncCommand>,
    sender: mpsc::Sender<SyncResult>,
    shutdown: Arc<AtomicBool>,
    wake: WakeCallback,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            SyncCommand::Probe { request_id } => {
                send_result(
                    &sender,
                    &wake,
                    SyncResult::Probe { request_id, outcome: transport.probe() },
                );
            }
            SyncCommand::Refresh { request_id, folders } => {
                let outcome = refresh_folders(transport.as_ref(), folders);
                send_result(&sender, &wake, SyncResult::Refresh { request_id, outcome });
            }
            SyncCommand::ConfigureDevice { request_id, config } => {
                let outcome = transport.put_device(&config);
                send_result(&sender, &wake, SyncResult::DeviceConfigured { request_id, outcome });
            }
            SyncCommand::ConfigureFolder { request_id, config } => {
                let outcome = transport.put_folder(&config);
                send_result(&sender, &wake, SyncResult::FolderConfigured { request_id, outcome });
            }
            SyncCommand::ReadDeviceConfig { request_id, device_id } => {
                let outcome = transport.device_config(&device_id);
                send_result(&sender, &wake, SyncResult::DeviceConfigRead { request_id, outcome });
            }
            SyncCommand::ReadFolderConfig { request_id, folder_id } => {
                let outcome = transport.folder_config(&folder_id);
                send_result(&sender, &wake, SyncResult::FolderConfigRead { request_id, outcome });
            }
            SyncCommand::ReadPendingFolders { request_id } => {
                let outcome = transport.pending_folders();
                send_result(&sender, &wake, SyncResult::PendingFoldersRead { request_id, outcome });
            }
            SyncCommand::EnsureManagedIgnores { request_id, folder_id } => {
                let outcome = transport.ensure_managed_ignores(&folder_id);
                send_result(&sender, &wake, SyncResult::IgnoresEnsured { request_id, outcome });
            }
            SyncCommand::RepairManagedIgnores { request_id, folder_id } => {
                let outcome = transport.repair_managed_ignores(&folder_id);
                send_result(&sender, &wake, SyncResult::IgnoresRepaired { request_id, outcome });
            }
            SyncCommand::RemoveFolder { request_id, folder_id } => {
                let outcome = transport.remove_folder(&folder_id);
                send_result(&sender, &wake, SyncResult::FolderRemoved { request_id, outcome });
            }
            SyncCommand::PauseFolder { request_id, folder_id, paused } => {
                let outcome = transport.patch_folder_paused(&folder_id, paused);
                send_result(&sender, &wake, SyncResult::FolderPaused { request_id, outcome });
            }
            SyncCommand::PauseDevice { request_id, device_id, paused } => {
                let outcome = if paused {
                    transport.pause_device(&device_id)
                } else {
                    transport.resume_device(&device_id)
                };
                send_result(&sender, &wake, SyncResult::DevicePaused { request_id, outcome });
            }
            SyncCommand::ScanFolder { request_id, folder_id } => {
                let outcome = transport.scan_folder(&folder_id);
                send_result(&sender, &wake, SyncResult::FolderScanned { request_id, outcome });
            }
            SyncCommand::Shutdown => break,
            SyncCommand::Subscribe { .. } => {}
        }
        if shutdown.load(Ordering::Acquire) {
            break;
        }
    }
}

fn refresh_folders(
    transport: &dyn SyncTransport,
    folders: Vec<FolderId>,
) -> Result<Vec<(FolderId, FolderStatus)>, SyncError> {
    let mut statuses = Vec::with_capacity(folders.len());
    for folder in folders {
        let status = transport.folder_status(&folder)?;
        statuses.push((folder, status));
    }
    Ok(statuses)
}

fn event_loop(
    transport: Arc<dyn SyncTransport>,
    receiver: mpsc::Receiver<Option<EventCursor>>,
    sender: mpsc::Sender<SyncEvent>,
    shutdown: Arc<AtomicBool>,
    wake: WakeCallback,
) {
    let mut cursor = match receive_subscription(&receiver, &shutdown) {
        Some(cursor) => cursor,
        None => return,
    };
    let mut backoff = RetryBackoff::new();

    while !shutdown.load(Ordering::Acquire) {
        while let Ok(subscription) = receiver.try_recv() {
            let Some(new_cursor) = subscription else { return };
            cursor = new_cursor;
        }

        match transport.events_since(cursor, 1) {
            Ok(events) => {
                backoff.reset();
                let mut requires_resubscribe = false;
                let mut emitted = false;
                for event in events {
                    match event {
                        SyncEvent::Remote { id, kind } => {
                            cursor = EventCursor(cursor.0.max(id));
                            send_event(&sender, &wake, SyncEvent::Remote { id, kind });
                            emitted = true;
                        }
                        SyncEvent::FullRefreshRequired => {
                            send_event(&sender, &wake, SyncEvent::FullRefreshRequired);
                            requires_resubscribe = true;
                            emitted = true;
                        }
                    }
                }
                if requires_resubscribe {
                    cursor = match receive_subscription(&receiver, &shutdown) {
                        Some(cursor) => cursor,
                        None => return,
                    };
                } else if !emitted {
                    match interruptible_wait(&receiver, &shutdown, Duration::from_millis(50)) {
                        WaitOutcome::Continue => {}
                        WaitOutcome::Resubscribe(new_cursor) => cursor = new_cursor,
                        WaitOutcome::Stop => return,
                    }
                }
            }
            Err(error) => {
                let failure = retry_failure(&error);
                match backoff.on_failure(failure) {
                    crate::BackoffAction::RetryAfter(delay) => {
                        match interruptible_wait(&receiver, &shutdown, delay) {
                            WaitOutcome::Continue => {}
                            WaitOutcome::Resubscribe(new_cursor) => cursor = new_cursor,
                            WaitOutcome::Stop => return,
                        }
                    }
                    crate::BackoffAction::Stop => {
                        return;
                    }
                }
            }
        }
    }
}

fn receive_subscription(
    receiver: &mpsc::Receiver<Option<EventCursor>>,
    shutdown: &AtomicBool,
) -> Option<EventCursor> {
    while !shutdown.load(Ordering::Acquire) {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(cursor)) => return Some(cursor),
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    None
}

enum WaitOutcome {
    Continue,
    Resubscribe(EventCursor),
    Stop,
}

fn interruptible_wait(
    receiver: &mpsc::Receiver<Option<EventCursor>>,
    shutdown: &AtomicBool,
    duration: Duration,
) -> WaitOutcome {
    match receiver.recv_timeout(duration) {
        Ok(Some(cursor)) => WaitOutcome::Resubscribe(cursor),
        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => WaitOutcome::Stop,
        Err(mpsc::RecvTimeoutError::Timeout) if shutdown.load(Ordering::Acquire) => {
            WaitOutcome::Stop
        }
        Err(mpsc::RecvTimeoutError::Timeout) => WaitOutcome::Continue,
    }
}

fn retry_failure(error: &SyncError) -> RetryFailure {
    match error {
        SyncError::Authentication => RetryFailure::Authentication,
        SyncError::IncompatibleVersion { .. } => RetryFailure::IncompatibleVersion,
        SyncError::ConnectionRefused | SyncError::RequestTimeout { .. } => RetryFailure::Connection,
        _ => RetryFailure::Other,
    }
}

fn send_result(sender: &mpsc::Sender<SyncResult>, wake: &WakeCallback, result: SyncResult) {
    if sender.send(result).is_ok() {
        wake();
    }
}

fn send_event(sender: &mpsc::Sender<SyncEvent>, wake: &WakeCallback, event: SyncEvent) {
    if sender.send(event).is_ok() {
        wake();
    }
}

fn join_worker(worker: Option<JoinHandle<()>>) {
    if let Some(worker) = worker {
        worker.join().expect("Syncthing worker should stop cleanly");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::{SyncCommand, SyncResult, SyncService, SyncTransport};
    use crate::{
        DeviceConfig, DeviceId, EventCursor, FolderConfig, FolderId, FolderPhase, FolderStatus,
        InstanceInfo, SyncError, SyncEvent, SyncEventKind,
    };

    const DEVICE_ID: &str = "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG";

    struct FakeTransport {
        calls: Mutex<Vec<String>>,
    }

    impl FakeTransport {
        fn new() -> Self {
            Self { calls: Mutex::new(Vec::new()) }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("fake calls lock should work").clone()
        }
    }

    impl SyncTransport for FakeTransport {
        fn probe(&self) -> Result<InstanceInfo, SyncError> {
            self.calls.lock().expect("fake calls lock should work").push("probe".to_owned());
            Ok(InstanceInfo {
                version: semver::Version::new(2, 1, 1),
                device_id: DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse"),
            })
        }

        fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("folder:{}", folder.as_str()));
            Ok(FolderStatus {
                phase: FolderPhase::Idle,
                need_bytes: 0,
                need_items: 0,
                completion_percent: 100.0,
                errors: 0,
            })
        }

        fn events_since(
            &self,
            cursor: EventCursor,
            _timeout_seconds: u16,
        ) -> Result<Vec<SyncEvent>, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("events:{}", cursor.0));
            if cursor.0 == 0 {
                Ok(vec![SyncEvent::Remote { id: 1, kind: SyncEventKind::FolderStateChanged }])
            } else {
                Ok(Vec::new())
            }
        }

        fn put_device(&self, config: &DeviceConfig) -> Result<DeviceConfig, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("put-device:{}", config.device_id.as_str()));
            Ok(config.clone())
        }

        fn put_folder(&self, config: &FolderConfig) -> Result<FolderConfig, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("put-folder:{}", config.folder_id.as_str()));
            Ok(config.clone())
        }

        fn device_config(&self, device_id: &DeviceId) -> Result<Option<DeviceConfig>, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("read-device:{}", device_id.as_str()));
            Ok(None)
        }

        fn folder_config(&self, folder_id: &FolderId) -> Result<Option<FolderConfig>, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("read-folder:{}", folder_id.as_str()));
            Ok(None)
        }

        fn pending_folders(&self) -> Result<Vec<crate::PendingFolder>, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push("pending-folders".to_owned());
            Ok(Vec::new())
        }

        fn ensure_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("ignores:{}", folder_id.as_str()));
            Ok(Vec::new())
        }

        fn repair_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("repair-ignores:{}", folder_id.as_str()));
            Ok(Vec::new())
        }

        fn remove_folder(&self, folder_id: &FolderId) -> Result<(), SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("remove-folder:{}", folder_id.as_str()));
            Ok(())
        }

        fn patch_folder_paused(
            &self,
            folder_id: &FolderId,
            paused: bool,
        ) -> Result<FolderConfig, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("pause-folder:{}:{paused}", folder_id.as_str()));
            Err(SyncError::ConfigurationMismatch { operation: "fake pause folder" })
        }

        fn pause_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("pause-device:{}", device_id.as_str()));
            Err(SyncError::ConfigurationMismatch { operation: "fake pause device" })
        }

        fn resume_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("resume-device:{}", device_id.as_str()));
            Err(SyncError::ConfigurationMismatch { operation: "fake resume device" })
        }

        fn scan_folder(&self, folder_id: &FolderId) -> Result<(), SyncError> {
            self.calls
                .lock()
                .expect("fake calls lock should work")
                .push(format!("scan-folder:{}", folder_id.as_str()));
            Ok(())
        }
    }

    #[test]
    fn commands_are_serialized_and_results_keep_request_ids() {
        let transport = Arc::new(FakeTransport::new());
        let wake_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake_counter = wake_count.clone();
        let service = SyncService::spawn_with_transport(transport.clone(), move || {
            wake_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });

        service.submit(SyncCommand::Probe { request_id: 7 }).expect("probe should submit");
        let probe = receive_result(&service);
        assert!(matches!(probe, SyncResult::Probe { request_id: 7, outcome: Ok(_) }));

        let folder = FolderId::new("notes".to_owned()).expect("folder ID should parse");
        service
            .submit(SyncCommand::Refresh { request_id: 8, folders: vec![folder] })
            .expect("refresh should submit");
        let refresh = receive_result(&service);
        assert!(matches!(refresh, SyncResult::Refresh { request_id: 8, outcome: Ok(_) }));

        service
            .submit(SyncCommand::Subscribe { cursor: EventCursor(0) })
            .expect("subscription should submit");
        let event = receive_event(&service);
        assert_eq!(event, SyncEvent::Remote { id: 1, kind: SyncEventKind::FolderStateChanged });

        service.shutdown();
        let calls = transport.calls();
        assert_eq!(calls[0], "probe");
        assert_eq!(calls[1], "folder:notes");
        assert!(calls.iter().any(|call| call == "events:0"));
        assert!(wake_count.load(std::sync::atomic::Ordering::Relaxed) >= 3);
    }

    fn receive_result(service: &SyncService) -> SyncResult {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = service.try_recv() {
                return result;
            }
            assert!(std::time::Instant::now() < deadline, "service result timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn receive_event(service: &SyncService) -> SyncEvent {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(event) = service.try_recv_event() {
                return event;
            }
            assert!(std::time::Instant::now() < deadline, "service event timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
