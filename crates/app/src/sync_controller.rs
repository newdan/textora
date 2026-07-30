use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use semver::Version;
use textora_sync::{
    ApiKey, DeviceConfig, EventCursor, FolderConfig, FolderId, InstanceInfo, LoopbackEndpoint,
    PendingFolder, RemoteDeviceSpec, SYNCTHING_DYNAMIC_ADDRESS, SyncCommand, SyncError, SyncEvent,
    SyncResult, SyncService,
};

use crate::library_registry::{
    LibraryRecord, LibraryRegistrationState, LibraryRegistry, ProvisioningStage,
};
use crate::sync_connection_store::{
    StoredSyncConnection, SyncConnectionStore, SyncConnectionStoreError,
};
use crate::sync_secret_store::{MacKeychainSecretStore, SyncSecretStore, SyncSecretStoreError};

const DEFAULT_KEYCHAIN_ACCOUNT: &str = "default";
const CONTROLLER_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RequestId(pub(crate) u64);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SyncConnectionState {
    NotConfigured,
    Connecting,
    Connected { instance: InstanceInfo },
    AuthenticationRequired,
    Incompatible { found: Version },
    Unavailable { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SyncControllerSnapshot {
    pub(crate) connection: SyncConnectionState,
    pub(crate) endpoint: Option<String>,
    pub(crate) has_api_key: bool,
    pub(crate) last_request_id: Option<RequestId>,
    pub(crate) event_cursor: EventCursor,
    pub(crate) libraries: Vec<LibraryRecord>,
    pub(crate) pending_folders: Vec<PendingFolder>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SyncNotice {
    RemoteEvent(SyncEvent),
    LibraryError { request_id: RequestId, message: String },
    LocalError { message: String },
}

#[derive(Debug)]
pub(crate) enum SyncControllerError {
    InvalidApiKey,
    WorkerUnavailable,
}

impl fmt::Display for SyncControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidApiKey => formatter.write_str("invalid API key"),
            Self::WorkerUnavailable => formatter.write_str("Syncthing worker is unavailable"),
        }
    }
}

impl std::error::Error for SyncControllerError {}

enum ControllerCommand {
    Initialize { request_id: RequestId },
    TestConnection { request_id: RequestId, endpoint: LoopbackEndpoint, api_key: String },
    Configure { request_id: RequestId, endpoint: LoopbackEndpoint, new_api_key: String },
    PublishLibrary { request_id: RequestId, root: PathBuf, remote: RemoteDeviceSpec },
    AcceptRemoteLibrary { request_id: RequestId, folder_id: FolderId, empty_root: PathBuf },
    RepairLibrary { request_id: RequestId, library_id: String },
    ScanLibrary { request_id: RequestId, library_id: String },
    PauseLibrary { request_id: RequestId, library_id: String, paused: bool },
    RemoveLibraryMapping { request_id: RequestId, library_id: String },
    UnregisterLibrary { request_id: RequestId, library_id: String },
    Shutdown,
}

enum ControllerResult {
    State { request_id: RequestId, state: SyncConnectionState },
    Probe { request_id: RequestId, outcome: Result<InstanceInfo, SyncError> },
    Metadata { endpoint: Option<String>, has_api_key: bool },
    PendingFolders { folders: Vec<PendingFolder> },
    Event(SyncEvent),
    Registry { libraries: Vec<LibraryRecord> },
    Library { request_id: RequestId, record: Option<LibraryRecord> },
    LibraryError { request_id: RequestId, message: String },
}

pub(crate) struct SyncController {
    command_sender: mpsc::Sender<ControllerCommand>,
    result_receiver: mpsc::Receiver<ControllerResult>,
    snapshot: SyncControllerSnapshot,
    notices: Vec<SyncNotice>,
    next_request_id: u64,
    worker: Option<JoinHandle<()>>,
}

impl SyncController {
    pub(crate) fn new_default(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::new(SyncConnectionStore::default(), Arc::new(MacKeychainSecretStore::new()), wake)
    }

    pub(crate) fn new(
        store: SyncConnectionStore,
        secret_store: Arc<dyn SyncSecretStore>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let wake = Arc::new(wake);
        let worker = thread::Builder::new()
            .name("textora-syncthing-controller".to_owned())
            .spawn(move || {
                controller_worker(store, secret_store, command_receiver, result_sender, wake)
            })
            .expect("Syncthing controller worker should start");

        let mut controller = Self {
            command_sender,
            result_receiver,
            snapshot: SyncControllerSnapshot {
                connection: SyncConnectionState::Connecting,
                endpoint: None,
                has_api_key: false,
                last_request_id: None,
                event_cursor: EventCursor(0),
                libraries: Vec::new(),
                pending_folders: Vec::new(),
            },
            notices: Vec::new(),
            next_request_id: 1,
            worker: Some(worker),
        };
        let request_id = controller.allocate_request_id();
        if controller.command_sender.send(ControllerCommand::Initialize { request_id }).is_err() {
            controller.snapshot.connection = SyncConnectionState::Unavailable {
                message: "Syncthing controller worker is unavailable".to_owned(),
            };
        }
        controller
    }

    pub(crate) fn configure_connection(
        &mut self,
        endpoint: LoopbackEndpoint,
        new_api_key: String,
    ) -> Result<RequestId, SyncControllerError> {
        ApiKey::new(new_api_key.clone()).map_err(|_| SyncControllerError::InvalidApiKey)?;
        let request_id = self.allocate_request_id();
        self.command_sender
            .send(ControllerCommand::Configure { request_id, endpoint, new_api_key })
            .map_err(|_| SyncControllerError::WorkerUnavailable)?;
        self.snapshot.connection = SyncConnectionState::Connecting;
        self.snapshot.last_request_id = Some(request_id);
        Ok(request_id)
    }

    pub(crate) fn test_connection(
        &mut self,
        endpoint: LoopbackEndpoint,
        api_key: String,
    ) -> Result<RequestId, SyncControllerError> {
        ApiKey::new(api_key.clone()).map_err(|_| SyncControllerError::InvalidApiKey)?;
        let request_id = self.allocate_request_id();
        self.command_sender
            .send(ControllerCommand::TestConnection { request_id, endpoint, api_key })
            .map_err(|_| SyncControllerError::WorkerUnavailable)?;
        self.snapshot.last_request_id = Some(request_id);
        Ok(request_id)
    }

    pub(crate) fn publish_library(
        &mut self,
        root: PathBuf,
        remote: RemoteDeviceSpec,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::PublishLibrary {
            request_id: RequestId(0),
            root,
            remote,
        })
    }

    pub(crate) fn accept_remote_library(
        &mut self,
        folder_id: FolderId,
        empty_root: PathBuf,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::AcceptRemoteLibrary {
            request_id: RequestId(0),
            folder_id,
            empty_root,
        })
    }

    pub(crate) fn repair_library(
        &mut self,
        library_id: String,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::RepairLibrary {
            request_id: RequestId(0),
            library_id,
        })
    }

    pub(crate) fn scan_library(
        &mut self,
        library_id: String,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::ScanLibrary {
            request_id: RequestId(0),
            library_id,
        })
    }

    pub(crate) fn pause_library(
        &mut self,
        library_id: String,
        paused: bool,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::PauseLibrary {
            request_id: RequestId(0),
            library_id,
            paused,
        })
    }

    pub(crate) fn remove_library_mapping(
        &mut self,
        library_id: String,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::RemoveLibraryMapping {
            request_id: RequestId(0),
            library_id,
        })
    }

    pub(crate) fn unregister_library(
        &mut self,
        library_id: String,
    ) -> Result<RequestId, SyncControllerError> {
        self.submit_library_command(ControllerCommand::UnregisterLibrary {
            request_id: RequestId(0),
            library_id,
        })
    }

    fn submit_library_command(
        &mut self,
        command: ControllerCommand,
    ) -> Result<RequestId, SyncControllerError> {
        let request_id = self.allocate_request_id();
        let command = match command {
            ControllerCommand::PublishLibrary { root, remote, .. } => {
                ControllerCommand::PublishLibrary { request_id, root, remote }
            }
            ControllerCommand::AcceptRemoteLibrary { folder_id, empty_root, .. } => {
                ControllerCommand::AcceptRemoteLibrary { request_id, folder_id, empty_root }
            }
            ControllerCommand::RepairLibrary { library_id, .. } => {
                ControllerCommand::RepairLibrary { request_id, library_id }
            }
            ControllerCommand::ScanLibrary { library_id, .. } => {
                ControllerCommand::ScanLibrary { request_id, library_id }
            }
            ControllerCommand::PauseLibrary { library_id, paused, .. } => {
                ControllerCommand::PauseLibrary { request_id, library_id, paused }
            }
            ControllerCommand::RemoveLibraryMapping { library_id, .. } => {
                ControllerCommand::RemoveLibraryMapping { request_id, library_id }
            }
            ControllerCommand::UnregisterLibrary { library_id, .. } => {
                ControllerCommand::UnregisterLibrary { request_id, library_id }
            }
            _ => return Err(SyncControllerError::WorkerUnavailable),
        };
        self.command_sender.send(command).map_err(|_| SyncControllerError::WorkerUnavailable)?;
        self.snapshot.last_request_id = Some(request_id);
        Ok(request_id)
    }

    pub(crate) fn snapshot(&self) -> &SyncControllerSnapshot {
        &self.snapshot
    }

    pub(crate) fn drain_background(&mut self) {
        while let Ok(result) = self.result_receiver.try_recv() {
            match result {
                ControllerResult::State { request_id, state } => {
                    self.snapshot.connection = state;
                    self.snapshot.last_request_id = Some(request_id);
                }
                ControllerResult::Metadata { endpoint, has_api_key } => {
                    self.snapshot.endpoint = endpoint;
                    self.snapshot.has_api_key = has_api_key;
                }
                ControllerResult::PendingFolders { folders } => {
                    self.snapshot.pending_folders = folders;
                }
                ControllerResult::Probe { request_id, outcome } => {
                    self.snapshot.connection = match outcome {
                        Ok(instance) => SyncConnectionState::Connected { instance },
                        Err(error) => Self::connection_state_for_error(&error),
                    };
                    self.snapshot.last_request_id = Some(request_id);
                }
                ControllerResult::Registry { libraries } => {
                    self.snapshot.libraries = libraries;
                }
                ControllerResult::Library { request_id, record } => {
                    if let Some(record) = record {
                        replace_library(&mut self.snapshot.libraries, record);
                    }
                    self.snapshot.last_request_id = Some(request_id);
                }
                ControllerResult::LibraryError { request_id, message } => {
                    self.snapshot.last_request_id = Some(request_id);
                    self.notices.push(SyncNotice::LibraryError { request_id, message });
                }
                ControllerResult::Event(event) => {
                    if let SyncEvent::Remote { id, .. } = event {
                        self.snapshot.event_cursor =
                            EventCursor(self.snapshot.event_cursor.0.max(id));
                    }
                    self.notices.push(SyncNotice::RemoteEvent(event));
                }
            }
        }
    }

    pub(crate) fn drain_notices(&mut self) -> Vec<SyncNotice> {
        std::mem::take(&mut self.notices)
    }

    pub(crate) fn push_local_error(&mut self, message: String) {
        self.notices.push(SyncNotice::LocalError { message });
    }

    pub(crate) fn connection_state_for_error(error: &SyncError) -> SyncConnectionState {
        match error {
            SyncError::Authentication => SyncConnectionState::AuthenticationRequired,
            SyncError::IncompatibleVersion { found } => {
                SyncConnectionState::Incompatible { found: found.clone() }
            }
            _ => SyncConnectionState::Unavailable { message: error.to_string() },
        }
    }

    fn allocate_request_id(&mut self) -> RequestId {
        let request_id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }

    pub(crate) fn shutdown(mut self) {
        let _ = self.command_sender.send(ControllerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("Syncthing controller worker should stop cleanly");
        }
    }
}

impl Drop for SyncController {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.command_sender.send(ControllerCommand::Shutdown);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn controller_worker(
    store: SyncConnectionStore,
    secret_store: Arc<dyn SyncSecretStore>,
    receiver: mpsc::Receiver<ControllerCommand>,
    sender: mpsc::Sender<ControllerResult>,
    wake: Arc<dyn Fn() + Send + Sync + 'static>,
) {
    let mut service: Option<SyncService> = None;
    let mut registry = match LibraryRegistry::load(store.config_dir()) {
        Ok(registry) => registry,
        Err(error) => {
            send_result(
                &sender,
                &wake,
                ControllerResult::LibraryError {
                    request_id: RequestId(0),
                    message: error.to_string(),
                },
            );
            LibraryRegistry::new(store.config_dir())
        }
    };
    send_result(
        &sender,
        &wake,
        ControllerResult::Registry { libraries: registry.records().cloned().collect() },
    );
    let mut instance = None;
    let mut next_service_request_id = 1_u64;
    loop {
        match receiver.recv_timeout(CONTROLLER_POLL_INTERVAL) {
            Ok(command) => {
                if handle_controller_command(
                    &store,
                    &secret_store,
                    &sender,
                    &wake,
                    &mut service,
                    &mut registry,
                    &mut instance,
                    &mut next_service_request_id,
                    command,
                ) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        drain_service(&mut service, &sender, &wake, &mut instance);
    }
    drain_service(&mut service, &sender, &wake, &mut instance);
    if let Some(service) = service {
        service.shutdown();
    }
}

fn handle_controller_command(
    store: &SyncConnectionStore,
    secret_store: &Arc<dyn SyncSecretStore>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    service: &mut Option<SyncService>,
    registry: &mut LibraryRegistry,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
    command: ControllerCommand,
) -> bool {
    match command {
        ControllerCommand::Initialize { request_id } => {
            initialize_connection(store, secret_store, sender, wake, service, request_id);
            false
        }
        ControllerCommand::TestConnection { request_id, endpoint, api_key } => {
            test_connection(sender, wake, request_id, endpoint, api_key);
            false
        }
        ControllerCommand::Configure { request_id, endpoint, new_api_key } => {
            configure_connection(
                store,
                secret_store,
                sender,
                wake,
                service,
                request_id,
                endpoint,
                new_api_key,
            );
            false
        }
        ControllerCommand::PublishLibrary { request_id, root, remote } => {
            publish_library(
                service,
                registry,
                instance,
                sender,
                wake,
                request_id,
                root,
                remote,
                next_service_request_id,
            );
            false
        }
        ControllerCommand::AcceptRemoteLibrary { request_id, folder_id, empty_root } => {
            accept_remote_library(
                service,
                registry,
                instance,
                sender,
                wake,
                request_id,
                folder_id,
                empty_root,
                next_service_request_id,
            );
            false
        }
        ControllerCommand::RepairLibrary { request_id, library_id } => {
            repair_library(
                service,
                registry,
                instance,
                sender,
                wake,
                request_id,
                &library_id,
                next_service_request_id,
            );
            false
        }
        ControllerCommand::ScanLibrary { request_id, library_id } => {
            operate_on_library(
                service,
                registry,
                sender,
                wake,
                request_id,
                &library_id,
                LibraryOperation::Scan,
                next_service_request_id,
            );
            false
        }
        ControllerCommand::PauseLibrary { request_id, library_id, paused } => {
            operate_on_library(
                service,
                registry,
                sender,
                wake,
                request_id,
                &library_id,
                LibraryOperation::Pause { paused },
                next_service_request_id,
            );
            false
        }
        ControllerCommand::RemoveLibraryMapping { request_id, library_id } => {
            remove_library_mapping(registry, sender, wake, request_id, &library_id);
            false
        }
        ControllerCommand::UnregisterLibrary { request_id, library_id } => {
            unregister_library(
                service,
                registry,
                sender,
                wake,
                request_id,
                &library_id,
                next_service_request_id,
            );
            false
        }
        ControllerCommand::Shutdown => true,
    }
}

fn initialize_connection(
    store: &SyncConnectionStore,
    secret_store: &Arc<dyn SyncSecretStore>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    service: &mut Option<SyncService>,
    request_id: RequestId,
) {
    let stored = match store.load() {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            send_metadata(sender, wake, None, false);
            send_state(sender, wake, request_id, SyncConnectionState::NotConfigured);
            return;
        }
        Err(error) => {
            send_metadata(sender, wake, None, false);
            send_state(sender, wake, request_id, unavailable_from_store_error(error));
            return;
        }
    };
    let endpoint = stored.endpoint.as_str().to_owned();
    let api_key = match secret_store.load_api_key(&stored.keychain_account) {
        Ok(Some(api_key)) => api_key,
        Ok(None) => {
            send_metadata(sender, wake, Some(endpoint), false);
            send_state(sender, wake, request_id, SyncConnectionState::AuthenticationRequired);
            return;
        }
        Err(error) => {
            send_metadata(sender, wake, Some(endpoint), false);
            send_state(sender, wake, request_id, unavailable_from_secret_error(error));
            return;
        }
    };
    send_metadata(sender, wake, Some(endpoint), true);
    start_service(service, sender, wake, request_id, stored.endpoint, api_key);
}

fn configure_connection(
    store: &SyncConnectionStore,
    secret_store: &Arc<dyn SyncSecretStore>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    service: &mut Option<SyncService>,
    request_id: RequestId,
    endpoint: LoopbackEndpoint,
    new_api_key: String,
) {
    let connection = StoredSyncConnection {
        endpoint: endpoint.clone(),
        keychain_account: DEFAULT_KEYCHAIN_ACCOUNT.to_owned(),
    };
    if let Err(error) = secret_store.save_api_key(&connection.keychain_account, &new_api_key) {
        send_state(sender, wake, request_id, unavailable_from_secret_error(error));
        return;
    }
    if let Err(error) = store.save(&connection) {
        let _ = secret_store.delete_api_key(&connection.keychain_account);
        send_state(sender, wake, request_id, unavailable_from_store_error(error));
        return;
    }
    send_metadata(sender, wake, Some(endpoint.as_str().to_owned()), true);
    let api_key = match ApiKey::new(new_api_key) {
        Ok(api_key) => api_key,
        Err(_) => {
            send_state(sender, wake, request_id, SyncConnectionState::AuthenticationRequired);
            return;
        }
    };
    start_service(service, sender, wake, request_id, endpoint, api_key);
}

fn test_connection(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    endpoint: LoopbackEndpoint,
    api_key: String,
) {
    let outcome = ApiKey::new(api_key)
        .map_err(|_| SyncError::Authentication)
        .and_then(|api_key| textora_sync::SyncthingClient::new(endpoint, api_key)?.probe());
    send_probe(sender, wake, request_id, outcome);
}

fn start_service(
    service: &mut Option<SyncService>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    endpoint: LoopbackEndpoint,
    api_key: ApiKey,
) {
    if let Some(old_service) = service.take() {
        old_service.shutdown();
    }
    let client = match textora_sync::SyncthingClient::new(endpoint, api_key) {
        Ok(client) => client,
        Err(error) => {
            send_probe(sender, wake, request_id, Err(error));
            return;
        }
    };
    let new_service = SyncService::spawn(client, || {});
    if let Err(error) = new_service.submit(SyncCommand::Probe { request_id: request_id.0 }) {
        send_probe(sender, wake, request_id, Err(error));
        new_service.shutdown();
        return;
    }
    let _ = new_service.submit(SyncCommand::ReadPendingFolders { request_id: request_id.0 });
    send_state(sender, wake, request_id, SyncConnectionState::Connecting);
    *service = Some(new_service);
}

fn drain_service(
    service: &mut Option<SyncService>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
) {
    let Some(service) = service else { return };
    while let Some(result) = service.try_recv() {
        match result {
            SyncResult::Probe { request_id, outcome } => {
                if let Ok(probed_instance) = &outcome {
                    *instance = Some(probed_instance.clone());
                }
                send_probe(sender, wake, RequestId(request_id), outcome);
            }
            SyncResult::Refresh { request_id, outcome: _ } => {
                send_state(sender, wake, RequestId(request_id), SyncConnectionState::Connecting)
            }
            SyncResult::DeviceConfigured { .. }
            | SyncResult::FolderConfigured { .. }
            | SyncResult::DeviceConfigRead { .. }
            | SyncResult::FolderConfigRead { .. }
            | SyncResult::IgnoresEnsured { .. }
            | SyncResult::IgnoresRepaired { .. }
            | SyncResult::FolderRemoved { .. }
            | SyncResult::FolderPaused { .. }
            | SyncResult::DevicePaused { .. }
            | SyncResult::FolderScanned { .. } => {}
            SyncResult::PendingFoldersRead { outcome, .. } => {
                if let Ok(folders) = outcome {
                    send_result(sender, wake, ControllerResult::PendingFolders { folders });
                }
            }
        }
    }
    while let Some(event) = service.try_recv_event() {
        send_result(sender, wake, ControllerResult::Event(event));
    }
}

enum LibraryOperation {
    Scan,
    Pause { paused: bool },
}

fn publish_library(
    service: &mut Option<SyncService>,
    registry: &mut LibraryRegistry,
    instance: &mut Option<InstanceInfo>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    root: PathBuf,
    remote: RemoteDeviceSpec,
    next_service_request_id: &mut u64,
) {
    let Some(service) = service.as_ref() else {
        send_library_error(sender, wake, request_id, "Syncthing connection is unavailable");
        return;
    };
    let record = match registry.register_published(root, &remote) {
        Ok(record) => record,
        Err(error) => {
            send_library_error(sender, wake, request_id, &error.to_string());
            return;
        }
    };
    let library_id = record.library_id.clone();
    if let Err(error) = registry.save() {
        send_library_error(sender, wake, request_id, &error.to_string());
        let _ = registry.remove(&library_id);
        return;
    }
    send_library(sender, wake, request_id, record.clone());

    let local_instance =
        match ensure_instance(service, instance, sender, wake, next_service_request_id) {
            Ok(instance) => instance,
            Err(error) => {
                fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
                return;
            }
        };
    let expected_device = DeviceConfig::new(
        remote.device_id.clone(),
        remote.name.clone(),
        remote.addresses.iter().map(|address| address.as_str().to_owned()).collect(),
        false,
    );
    let existing_device = match read_device_config(
        service,
        &remote.device_id,
        sender,
        wake,
        instance,
        next_service_request_id,
    ) {
        Ok(device) => device,
        Err(error) => {
            fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
            return;
        }
    };
    if existing_device.is_some() {
        fail_library(
            registry,
            sender,
            wake,
            request_id,
            &library_id,
            "remote device already exists; explicit configuration confirmation is required"
                .to_owned(),
        );
        return;
    }
    if let Err(error) =
        configure_device(service, expected_device, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) = update_library(
        registry,
        &library_id,
        Some(ProvisioningStage::RegisteringFolder),
        Some(true),
        None,
    ) {
        fail_library(registry, sender, wake, request_id, &library_id, error);
        return;
    }

    let folder_id = registry
        .get(&library_id)
        .map(|record| record.folder_id.clone())
        .expect("registered library should remain available");
    let expected_folder = FolderConfig::new(
        folder_id.clone(),
        folder_label(&record.root, &library_id),
        record.root.clone(),
        false,
        vec![local_instance.device_id.clone(), remote.device_id.clone()],
    );
    let existing_folder = match read_folder_config(
        service,
        &folder_id,
        sender,
        wake,
        instance,
        next_service_request_id,
    ) {
        Ok(folder) => folder,
        Err(error) => {
            fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
            return;
        }
    };
    if existing_folder.is_some() {
        fail_library(
            registry,
            sender,
            wake,
            request_id,
            &library_id,
            "folder ID already exists; explicit configuration confirmation is required".to_owned(),
        );
        return;
    }
    if let Err(error) =
        configure_folder(service, expected_folder, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) = update_library(
        registry,
        &library_id,
        Some(ProvisioningStage::ConfiguringIgnores),
        None,
        Some(true),
    ) {
        fail_library(registry, sender, wake, request_id, &library_id, error);
        return;
    }
    if let Err(error) =
        ensure_ignores(service, &folder_id, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) =
        scan_folder(service, &folder_id, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) = update_library(
        registry,
        &library_id,
        Some(ProvisioningStage::AwaitingRemoteAcceptance),
        None,
        None,
    ) {
        fail_library(registry, sender, wake, request_id, &library_id, error);
        return;
    }
    if let Some(record) = registry.get(&library_id).cloned() {
        send_library(sender, wake, request_id, record);
    }
}

fn accept_remote_library(
    service: &mut Option<SyncService>,
    registry: &mut LibraryRegistry,
    instance: &mut Option<InstanceInfo>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    folder_id: FolderId,
    empty_root: PathBuf,
    next_service_request_id: &mut u64,
) {
    let Some(service) = service.as_ref() else {
        send_library_error(sender, wake, request_id, "Syncthing connection is unavailable");
        return;
    };
    let root = match registry.canonical_empty_root(&empty_root) {
        Ok(root) => root,
        Err(error) => {
            send_library_error(sender, wake, request_id, &error.to_string());
            return;
        }
    };
    let pending_folders =
        match read_pending_folders(service, sender, wake, instance, next_service_request_id) {
            Ok(folders) => folders,
            Err(error) => {
                send_library_error(sender, wake, request_id, &error.to_string());
                return;
            }
        };
    let Some(pending) = pending_folders.into_iter().find(|pending| pending.folder_id == folder_id)
    else {
        send_library_error(sender, wake, request_id, "remote folder invitation was not found");
        return;
    };
    let remote = RemoteDeviceSpec {
        device_id: pending.offered_by.clone(),
        name: pending.label.clone().unwrap_or_else(|| folder_id.as_str().to_owned()),
        addresses: Vec::new(),
    };
    let record = match registry.register_accepted_remote(root, folder_id.clone(), &remote) {
        Ok(record) => record,
        Err(error) => {
            send_library_error(sender, wake, request_id, &error.to_string());
            return;
        }
    };
    let library_id = record.library_id.clone();
    if let Err(error) = registry.save() {
        send_library_error(sender, wake, request_id, &error.to_string());
        let _ = registry.remove(&library_id);
        return;
    }
    send_library(sender, wake, request_id, record.clone());

    let local_instance =
        match ensure_instance(service, instance, sender, wake, next_service_request_id) {
            Ok(instance) => instance,
            Err(error) => {
                fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
                return;
            }
        };
    let existing_device = match read_device_config(
        service,
        &remote.device_id,
        sender,
        wake,
        instance,
        next_service_request_id,
    ) {
        Ok(device) => device,
        Err(error) => {
            fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
            return;
        }
    };
    let remote_addresses = existing_device
        .as_ref()
        .map(|device| device.addresses.clone())
        .unwrap_or_else(|| vec![SYNCTHING_DYNAMIC_ADDRESS.to_owned()]);
    if let Some(saved_record) = registry.get_mut(&library_id) {
        saved_record.remote.addresses = remote_addresses;
    }
    if let Err(error) = registry.save() {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if existing_device.is_none() {
        let expected_device = DeviceConfig::new(
            remote.device_id.clone(),
            remote.name.clone(),
            vec![SYNCTHING_DYNAMIC_ADDRESS.to_owned()],
            false,
        );
        if let Err(error) = configure_device(
            service,
            expected_device,
            sender,
            wake,
            instance,
            next_service_request_id,
        ) {
            fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
            return;
        }
        if let Err(error) = update_library(registry, &library_id, None, Some(true), None) {
            fail_library(registry, sender, wake, request_id, &library_id, error);
            return;
        }
    }
    let expected_folder = FolderConfig::new(
        folder_id.clone(),
        remote.name.clone(),
        record.root.clone(),
        false,
        vec![local_instance.device_id, remote.device_id],
    );
    let existing_folder = match read_folder_config(
        service,
        &folder_id,
        sender,
        wake,
        instance,
        next_service_request_id,
    ) {
        Ok(folder) => folder,
        Err(error) => {
            fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
            return;
        }
    };
    if existing_folder.is_some() {
        fail_library(
            registry,
            sender,
            wake,
            request_id,
            &library_id,
            "folder already exists locally; explicit configuration confirmation is required"
                .to_owned(),
        );
        return;
    }
    if let Err(error) =
        configure_folder(service, expected_folder, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) =
        ensure_ignores(service, &folder_id, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) =
        scan_folder(service, &folder_id, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Err(error) = update_library(registry, &library_id, None, None, None) {
        fail_library(registry, sender, wake, request_id, &library_id, error);
        return;
    }
    if let Some(record) = registry.get_mut(&library_id) {
        record.state = LibraryRegistrationState::Active;
    }
    if let Err(error) = registry.save() {
        fail_library(registry, sender, wake, request_id, &library_id, error.to_string());
        return;
    }
    if let Some(record) = registry.get(&library_id).cloned() {
        send_library(sender, wake, request_id, record);
    }
}

fn repair_library(
    service: &mut Option<SyncService>,
    registry: &mut LibraryRegistry,
    instance: &mut Option<InstanceInfo>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    library_id: &str,
    next_service_request_id: &mut u64,
) {
    let Some(service) = service.as_ref() else {
        send_library_error(sender, wake, request_id, "Syncthing connection is unavailable");
        return;
    };
    let Some(record) = registry.get(library_id).cloned() else {
        send_library_error(sender, wake, request_id, "library mapping was not found");
        return;
    };
    let local_instance =
        match ensure_instance(service, instance, sender, wake, next_service_request_id) {
            Ok(instance) => instance,
            Err(error) => {
                fail_library(registry, sender, wake, request_id, library_id, error.to_string());
                return;
            }
        };
    let expected_device = DeviceConfig::new(
        record.remote.device_id.clone(),
        record.remote.name.clone(),
        record.remote.addresses.clone(),
        false,
    );
    if let Err(error) =
        configure_device(service, expected_device, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    let expected_folder = FolderConfig::new(
        record.folder_id.clone(),
        folder_label(&record.root, library_id),
        record.root.clone(),
        false,
        vec![local_instance.device_id, record.remote.device_id.clone()],
    );
    if let Err(error) =
        configure_folder(service, expected_folder, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    let service_request_id = allocate_service_request_id(next_service_request_id);
    if let Err(error) = service.submit(SyncCommand::RepairManagedIgnores {
        request_id: service_request_id,
        folder_id: record.folder_id.clone(),
    }) {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    let ignore_result =
        wait_for_result_without_forwarding(service, service_request_id, |result| match result {
            SyncResult::IgnoresRepaired { request_id: result_id, outcome }
                if result_id == service_request_id =>
            {
                Some(outcome)
            }
            _ => None,
        });
    if let Err(error) = ignore_result {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    if let Err(error) =
        scan_folder(service, &record.folder_id, sender, wake, instance, next_service_request_id)
    {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    if let Some(record) = registry.get_mut(library_id) {
        record.state = match record.origin {
            crate::library_registry::LibraryOrigin::Published => {
                LibraryRegistrationState::Provisioning {
                    stage: ProvisioningStage::AwaitingRemoteAcceptance,
                }
            }
            crate::library_registry::LibraryOrigin::AcceptedRemote => {
                LibraryRegistrationState::Active
            }
        };
    }
    if let Err(error) = registry.save() {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    if let Some(record) = registry.get(library_id).cloned() {
        send_library(sender, wake, request_id, record);
    }
}

fn ensure_instance(
    service: &SyncService,
    instance: &mut Option<InstanceInfo>,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    next_service_request_id: &mut u64,
) -> Result<InstanceInfo, SyncError> {
    if let Some(instance) = instance.clone() {
        return Ok(instance);
    }
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::Probe { request_id })?;
    let instance_result =
        wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
            SyncResult::Probe { request_id: result_id, outcome } if result_id == request_id => {
                Some(outcome)
            }
            _ => None,
        })?;
    *instance = Some(instance_result.clone());
    Ok(instance_result)
}

fn read_device_config(
    service: &SyncService,
    device_id: &textora_sync::DeviceId,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<Option<DeviceConfig>, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ReadDeviceConfig { request_id, device_id: device_id.clone() })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::DeviceConfigRead { request_id: result_id, outcome }
            if result_id == request_id =>
        {
            Some(outcome)
        }
        _ => None,
    })
}

fn read_folder_config(
    service: &SyncService,
    folder_id: &FolderId,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<Option<FolderConfig>, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ReadFolderConfig { request_id, folder_id: folder_id.clone() })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::FolderConfigRead { request_id: result_id, outcome }
            if result_id == request_id =>
        {
            Some(outcome)
        }
        _ => None,
    })
}

fn read_pending_folders(
    service: &SyncService,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<Vec<textora_sync::PendingFolder>, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ReadPendingFolders { request_id })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::PendingFoldersRead { request_id: result_id, outcome }
            if result_id == request_id =>
        {
            Some(outcome)
        }
        _ => None,
    })
}

fn configure_device(
    service: &SyncService,
    config: DeviceConfig,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<DeviceConfig, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ConfigureDevice { request_id, config })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::DeviceConfigured { request_id: result_id, outcome }
            if result_id == request_id =>
        {
            Some(outcome)
        }
        _ => None,
    })
}

fn configure_folder(
    service: &SyncService,
    config: FolderConfig,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<FolderConfig, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ConfigureFolder { request_id, config })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::FolderConfigured { request_id: result_id, outcome }
            if result_id == request_id =>
        {
            Some(outcome)
        }
        _ => None,
    })
}

fn ensure_ignores(
    service: &SyncService,
    folder_id: &FolderId,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<Vec<String>, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service
        .submit(SyncCommand::EnsureManagedIgnores { request_id, folder_id: folder_id.clone() })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::IgnoresEnsured { request_id: result_id, outcome }
            if result_id == request_id =>
        {
            Some(outcome)
        }
        _ => None,
    })
}

fn scan_folder(
    service: &SyncService,
    folder_id: &FolderId,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    instance: &mut Option<InstanceInfo>,
    next_service_request_id: &mut u64,
) -> Result<(), SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ScanFolder { request_id, folder_id: folder_id.clone() })?;
    wait_for_sync_result(service, request_id, sender, wake, instance, |result| match result {
        SyncResult::FolderScanned { request_id: result_id, outcome } if result_id == request_id => {
            Some(outcome)
        }
        _ => None,
    })
}

fn wait_for_sync_result<T, Extract>(
    service: &SyncService,
    _request_id: u64,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    _instance: &mut Option<InstanceInfo>,
    mut extract: Extract,
) -> Result<T, SyncError>
where
    Extract: FnMut(SyncResult) -> Option<Result<T, SyncError>>,
{
    loop {
        if let Some(result) = service.try_recv() {
            if let Some(outcome) = extract(result) {
                return outcome;
            }
            continue;
        }
        if let Some(event) = service.try_recv_event() {
            send_result(sender, wake, ControllerResult::Event(event));
            continue;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn allocate_service_request_id(counter: &mut u64) -> u64 {
    let request_id = *counter;
    *counter = counter.saturating_add(1);
    request_id
}

fn update_library(
    registry: &mut LibraryRegistry,
    library_id: &str,
    stage: Option<ProvisioningStage>,
    device_created: Option<bool>,
    folder_created: Option<bool>,
) -> Result<(), String> {
    let record = registry
        .get_mut(library_id)
        .ok_or_else(|| "library mapping disappeared during provisioning".to_owned())?;
    if let Some(stage) = stage {
        record.state = LibraryRegistrationState::Provisioning { stage };
    }
    if let Some(created) = device_created {
        record.device_created_by_textora = created;
    }
    if let Some(created) = folder_created {
        record.folder_created_by_textora = created;
    }
    registry.save().map_err(|error| error.to_string())
}

fn operate_on_library(
    service: &mut Option<SyncService>,
    registry: &mut LibraryRegistry,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    library_id: &str,
    operation: LibraryOperation,
    next_service_request_id: &mut u64,
) {
    let Some(service) = service.as_ref() else {
        send_library_error(sender, wake, request_id, "Syncthing connection is unavailable");
        return;
    };
    let Some(record) = registry.get(library_id).cloned() else {
        send_library_error(sender, wake, request_id, "library mapping was not found");
        return;
    };
    let outcome = match operation {
        LibraryOperation::Scan => {
            scan_folder_sync(service, &record.folder_id, next_service_request_id)
        }
        LibraryOperation::Pause { paused } => {
            pause_folder_sync(service, &record.folder_id, paused, next_service_request_id)
                .map(|_| ())
        }
    };
    if let Err(error) = outcome {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    if let Some(record) = registry.get_mut(library_id) {
        record.state = match operation_state(&operation) {
            Some(true) => LibraryRegistrationState::Paused,
            Some(false) => LibraryRegistrationState::Active,
            None => LibraryRegistrationState::Active,
        };
    }
    if let Err(error) = registry.save() {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    if let Some(record) = registry.get(library_id).cloned() {
        send_library(sender, wake, request_id, record);
    }
}

fn operation_state(operation: &LibraryOperation) -> Option<bool> {
    match operation {
        LibraryOperation::Scan => None,
        LibraryOperation::Pause { paused } => Some(*paused),
    }
}

fn scan_folder_sync(
    service: &SyncService,
    folder_id: &FolderId,
    next_service_request_id: &mut u64,
) -> Result<(), SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::ScanFolder { request_id, folder_id: folder_id.clone() })?;
    wait_for_result_without_forwarding(service, request_id, |result| match result {
        SyncResult::FolderScanned { request_id: result_id, outcome } if result_id == request_id => {
            Some(outcome)
        }
        _ => None,
    })
}

fn pause_folder_sync(
    service: &SyncService,
    folder_id: &FolderId,
    paused: bool,
    next_service_request_id: &mut u64,
) -> Result<FolderConfig, SyncError> {
    let request_id = allocate_service_request_id(next_service_request_id);
    service.submit(SyncCommand::PauseFolder {
        request_id,
        folder_id: folder_id.clone(),
        paused,
    })?;
    wait_for_result_without_forwarding(service, request_id, |result| match result {
        SyncResult::FolderPaused { request_id: result_id, outcome } if result_id == request_id => {
            Some(outcome)
        }
        _ => None,
    })
}

fn wait_for_result_without_forwarding<T, Extract>(
    service: &SyncService,
    _request_id: u64,
    mut extract: Extract,
) -> Result<T, SyncError>
where
    Extract: FnMut(SyncResult) -> Option<Result<T, SyncError>>,
{
    loop {
        if let Some(result) = service.try_recv() {
            if let Some(outcome) = extract(result) {
                return outcome;
            }
            continue;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn remove_library_mapping(
    registry: &mut LibraryRegistry,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    library_id: &str,
) {
    if registry.remove(library_id).is_none() {
        send_library_error(sender, wake, request_id, "library mapping was not found");
        return;
    }
    if let Err(error) = registry.save() {
        send_library_error(sender, wake, request_id, &error.to_string());
        return;
    }
    send_result(
        sender,
        wake,
        ControllerResult::Registry { libraries: registry.records().cloned().collect() },
    );
}

fn unregister_library(
    service: &mut Option<SyncService>,
    registry: &mut LibraryRegistry,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    library_id: &str,
    next_service_request_id: &mut u64,
) {
    let Some(service) = service.as_ref() else {
        send_library_error(sender, wake, request_id, "Syncthing connection is unavailable");
        return;
    };
    let Some(record) = registry.get(library_id).cloned() else {
        send_library_error(sender, wake, request_id, "library mapping was not found");
        return;
    };
    let service_request_id = allocate_service_request_id(next_service_request_id);
    if let Err(error) = service.submit(SyncCommand::RemoveFolder {
        request_id: service_request_id,
        folder_id: record.folder_id,
    }) {
        send_library_error(sender, wake, request_id, &error.to_string());
        return;
    }
    let outcome =
        wait_for_result_without_forwarding(service, service_request_id, |result| match result {
            SyncResult::FolderRemoved { request_id: result_id, outcome }
                if result_id == service_request_id =>
            {
                Some(outcome)
            }
            _ => None,
        });
    if let Err(error) = outcome {
        fail_library(registry, sender, wake, request_id, library_id, error.to_string());
        return;
    }
    remove_library_mapping(registry, sender, wake, request_id, library_id);
}

fn fail_library(
    registry: &mut LibraryRegistry,
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    library_id: &str,
    message: String,
) {
    let record = registry.get_mut(library_id).map(|record| {
        record.state = LibraryRegistrationState::Error { message: message.clone() };
        record.clone()
    });
    let _ = registry.save();
    if let Some(record) = record {
        send_library(sender, wake, request_id, record);
    }
    send_library_error(sender, wake, request_id, &message);
}

fn send_library(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    record: LibraryRecord,
) {
    send_result(sender, wake, ControllerResult::Library { request_id, record: Some(record) });
}

fn send_library_error(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    message: &str,
) {
    send_result(
        sender,
        wake,
        ControllerResult::LibraryError { request_id, message: message.to_owned() },
    );
}

fn folder_label(root: &std::path::Path, fallback: &str) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| fallback.to_owned())
}

fn replace_library(libraries: &mut Vec<LibraryRecord>, record: LibraryRecord) {
    if let Some(existing) =
        libraries.iter_mut().find(|existing| existing.library_id == record.library_id)
    {
        *existing = record;
    } else {
        libraries.push(record);
    }
}

fn send_probe(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    outcome: Result<InstanceInfo, SyncError>,
) {
    send_result(sender, wake, ControllerResult::Probe { request_id, outcome });
}

fn send_state(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    request_id: RequestId,
    state: SyncConnectionState,
) {
    send_result(sender, wake, ControllerResult::State { request_id, state });
}

fn send_metadata(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    endpoint: Option<String>,
    has_api_key: bool,
) {
    send_result(sender, wake, ControllerResult::Metadata { endpoint, has_api_key });
}

fn send_result(
    sender: &mpsc::Sender<ControllerResult>,
    wake: &Arc<dyn Fn() + Send + Sync + 'static>,
    result: ControllerResult,
) {
    if sender.send(result).is_ok() {
        wake();
    }
}

fn unavailable_from_store_error(error: SyncConnectionStoreError) -> SyncConnectionState {
    SyncConnectionState::Unavailable { message: error.to_string() }
}

fn unavailable_from_secret_error(error: SyncSecretStoreError) -> SyncConnectionState {
    SyncConnectionState::Unavailable { message: error.to_string() }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use textora_sync::{ApiKey, LoopbackEndpoint, SyncError};

    use super::{SyncConnectionState, SyncController, SyncControllerError};
    use crate::sync_connection_store::{StoredSyncConnection, SyncConnectionStore};
    use crate::sync_secret_store::{SyncSecretStore, SyncSecretStoreError};

    struct FakeSecretStore {
        value: Mutex<Option<String>>,
    }

    impl FakeSecretStore {
        fn empty() -> Self {
            Self { value: Mutex::new(None) }
        }
    }

    impl SyncSecretStore for FakeSecretStore {
        fn load_api_key(&self, _account: &str) -> Result<Option<ApiKey>, SyncSecretStoreError> {
            let value = self.value.lock().expect("fake secret lock should work").clone();
            value
                .map(|secret| ApiKey::new(secret).map_err(|_| SyncSecretStoreError::InvalidSecret))
                .transpose()
        }

        fn save_api_key(
            &self,
            _account: &str,
            new_secret: &str,
        ) -> Result<(), SyncSecretStoreError> {
            self.value.lock().expect("fake secret lock should work").replace(new_secret.to_owned());
            Ok(())
        }

        fn delete_api_key(&self, _account: &str) -> Result<(), SyncSecretStoreError> {
            self.value.lock().expect("fake secret lock should work").take();
            Ok(())
        }
    }

    fn wait_for_state(controller: &mut SyncController, expected: fn(&SyncConnectionState) -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            controller.drain_background();
            if expected(&controller.snapshot().connection) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "controller state timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn missing_secret_maps_to_authentication_required_without_blocking_constructor() {
        let directory = tempfile::tempdir().expect("temporary config directory should exist");
        let store = SyncConnectionStore::new(directory.path().to_owned());
        store
            .save(&StoredSyncConnection {
                endpoint: LoopbackEndpoint::parse("http://127.0.0.1:8384")
                    .expect("endpoint should parse"),
                keychain_account: "account".to_owned(),
            })
            .expect("connection metadata should save");
        let secret_store = Arc::new(FakeSecretStore::empty());
        let mut controller = SyncController::new(store, secret_store, || {});

        wait_for_state(&mut controller, |state| {
            matches!(state, SyncConnectionState::AuthenticationRequired)
        });
        controller.shutdown();
    }

    #[test]
    fn invalid_api_key_is_rejected_before_background_submission() {
        let directory = tempfile::tempdir().expect("temporary config directory should exist");
        let store = SyncConnectionStore::new(directory.path().to_owned());
        let secret_store = Arc::new(FakeSecretStore::empty());
        let mut controller = SyncController::new(store, secret_store, || {});

        let error = controller
            .configure_connection(
                LoopbackEndpoint::parse("http://127.0.0.1:8384").expect("endpoint should parse"),
                "   ".to_owned(),
            )
            .expect_err("blank API key should be rejected");
        assert!(matches!(error, SyncControllerError::InvalidApiKey));
        controller.shutdown();
    }

    #[test]
    fn probe_errors_map_to_stable_connection_states() {
        assert!(matches!(
            SyncController::connection_state_for_error(&SyncError::Authentication),
            SyncConnectionState::AuthenticationRequired
        ));
        assert!(matches!(
            SyncController::connection_state_for_error(&SyncError::IncompatibleVersion {
                found: semver::Version::new(2, 2, 0)
            }),
            SyncConnectionState::Incompatible { .. }
        ));
        assert!(matches!(
            SyncController::connection_state_for_error(&SyncError::ConnectionRefused),
            SyncConnectionState::Unavailable { .. }
        ));
    }
}
