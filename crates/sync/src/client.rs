use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Method;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::dto::{
    ConnectionsResponse, DeviceConfig, DeviceConfigResponse, EventResponse, FolderConfig,
    FolderConfigResponse, FolderErrorsResponse, FolderStatusResponse, InstanceInfo, PendingDevice,
    PendingDeviceResponse, PendingFolder, PendingFolderResponse, SystemStatusResponse,
    SystemVersionResponse,
};
use crate::ignore::{
    IgnoreRequest, IgnoreResponse, append_managed_ignore_block, repair_managed_ignore_block,
};
use crate::{ApiKey, DeviceId, FolderId, FolderPhase, FolderStatus, LoopbackEndpoint, SyncError};
use crate::{EventCursor, RemoteEvent, SyncEvent, SyncEventKind, reduce_event_batch};

const API_KEY_HEADER: &str = "X-API-Key";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_SUPPORTED_VERSION: (u64, u64, u64) = (2, 1, 1);
const MAX_SUPPORTED_VERSION: (u64, u64, u64) = (2, 2, 0);
const MAX_EVENT_TIMEOUT_SECONDS: u16 = 60;

pub struct SyncthingClient {
    endpoint: LoopbackEndpoint,
    api_key: ApiKey,
    http: Client,
}

impl SyncthingClient {
    pub fn new(endpoint: LoopbackEndpoint, api_key: ApiKey) -> Result<Self, SyncError> {
        let http = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| SyncError::InvalidResponse {
                operation: "HTTP client construction",
                message: error.to_string(),
            })?;
        Ok(Self { endpoint, api_key, http })
    }

    pub fn probe(&self) -> Result<InstanceInfo, SyncError> {
        let version_response: SystemVersionResponse =
            self.get_json("system version", "/rest/system/version")?;
        let version = parse_version(&version_response.version)?;
        ensure_supported_version(&version)?;

        let status: SystemStatusResponse = self.get_json("system status", "/rest/system/status")?;
        let device_id =
            DeviceId::parse(status.device_id).map_err(|_| SyncError::InvalidResponse {
                operation: "system status",
                message: "response contains an invalid device ID".to_owned(),
            })?;

        Ok(InstanceInfo { version, device_id })
    }

    pub fn connections(&self) -> Result<Vec<DeviceId>, SyncError> {
        let response: ConnectionsResponse =
            self.get_json("system connections", "/rest/system/connections")?;
        response
            .connections
            .keys()
            .map(|device_id| {
                DeviceId::parse(device_id.clone()).map_err(|_| SyncError::InvalidResponse {
                    operation: "system connections",
                    message: "response contains an invalid device ID".to_owned(),
                })
            })
            .collect()
    }

    pub fn pending_devices(&self) -> Result<Vec<PendingDevice>, SyncError> {
        let response: BTreeMap<String, PendingDeviceResponse> =
            self.get_json("pending devices", "/rest/cluster/pending/devices")?;
        response
            .into_iter()
            .map(|(device_id, pending)| {
                let device_id =
                    DeviceId::parse(device_id).map_err(|_| SyncError::InvalidResponse {
                        operation: "pending devices",
                        message: "response contains an invalid device ID".to_owned(),
                    })?;
                Ok(PendingDevice { device_id, name: pending.name })
            })
            .collect()
    }

    pub fn pending_folders(&self) -> Result<Vec<PendingFolder>, SyncError> {
        let response: BTreeMap<String, PendingFolderResponse> =
            self.get_json("pending folders", "/rest/cluster/pending/folders")?;
        let mut folders = Vec::new();
        for (folder_id, pending) in response {
            let folder_id = FolderId::new(folder_id).map_err(|_| SyncError::InvalidResponse {
                operation: "pending folders",
                message: "response contains an invalid folder ID".to_owned(),
            })?;
            for (device_id, offer) in pending.offered_by {
                let offered_by =
                    DeviceId::parse(device_id).map_err(|_| SyncError::InvalidResponse {
                        operation: "pending folders",
                        message: "response contains an invalid device ID".to_owned(),
                    })?;
                folders.push(PendingFolder {
                    folder_id: folder_id.clone(),
                    label: offer.label,
                    offered_by,
                });
            }
        }
        Ok(folders)
    }

    pub fn device_config(&self, device_id: &DeviceId) -> Result<Option<DeviceConfig>, SyncError> {
        let path = format!("/rest/config/devices/{}", device_id.as_str());
        let Some(response) =
            self.get_json_optional::<DeviceConfigResponse>("device config", &path)?
        else {
            return Ok(None);
        };
        Ok(Some(parse_device_config(response, "device config", Some(device_id))?))
    }

    pub fn folder_config(&self, folder_id: &FolderId) -> Result<Option<FolderConfig>, SyncError> {
        let path = format!("/rest/config/folders/{}", folder_id.as_str());
        let Some(response) =
            self.get_json_optional::<FolderConfigResponse>("folder config", &path)?
        else {
            return Ok(None);
        };
        Ok(Some(parse_folder_config(response, "folder config", Some(folder_id))?))
    }

    pub fn default_device(&self) -> Result<DeviceConfig, SyncError> {
        let response: DeviceConfigResponse =
            self.get_json("default device config", "/rest/config/defaults/device")?;
        parse_device_config(response, "default device config", None)
    }

    pub fn default_folder(&self) -> Result<FolderConfig, SyncError> {
        let response: FolderConfigResponse =
            self.get_json("default folder config", "/rest/config/defaults/folder")?;
        parse_folder_config(response, "default folder config", None)
    }

    pub fn put_device(&self, expected: &DeviceConfig) -> Result<DeviceConfig, SyncError> {
        let path = format!("/rest/config/devices/{}", expected.device_id.as_str());
        let mut payload = match self.get_json_value_optional("device config", &path)? {
            Some(payload) => payload,
            None => self.get_json_value("default device config", "/rest/config/defaults/device")?,
        };
        let object = payload.as_object_mut().ok_or_else(|| SyncError::InvalidResponse {
            operation: "device config",
            message: "device configuration must be a JSON object".to_owned(),
        })?;
        object.insert("deviceID".to_owned(), Value::String(expected.device_id.as_str().to_owned()));
        object.insert("name".to_owned(), Value::String(expected.name.clone()));
        object.insert(
            "addresses".to_owned(),
            Value::Array(expected.addresses.iter().cloned().map(Value::String).collect()),
        );
        object.insert("paused".to_owned(), Value::Bool(expected.paused));
        self.put_json("put device config", &path, payload)?;
        let actual = self
            .device_config(&expected.device_id)?
            .ok_or_else(|| SyncError::ConfigurationMismatch { operation: "put device config" })?;
        verify_device_configuration(expected, &actual)?;
        Ok(actual)
    }

    pub fn put_folder(&self, expected: &FolderConfig) -> Result<FolderConfig, SyncError> {
        let path = format!("/rest/config/folders/{}", expected.folder_id.as_str());
        let mut payload = match self.get_json_value_optional("folder config", &path)? {
            Some(payload) => payload,
            None => self.get_json_value("default folder config", "/rest/config/defaults/folder")?,
        };
        let object = payload.as_object_mut().ok_or_else(|| SyncError::InvalidResponse {
            operation: "folder config",
            message: "folder configuration must be a JSON object".to_owned(),
        })?;
        let path_string = expected.path.to_str().ok_or_else(|| SyncError::InvalidResponse {
            operation: "put folder config",
            message: "folder path is not valid UTF-8".to_owned(),
        })?;
        object.insert("id".to_owned(), Value::String(expected.folder_id.as_str().to_owned()));
        object.insert("label".to_owned(), Value::String(expected.label.clone()));
        object.insert("path".to_owned(), Value::String(path_string.to_owned()));
        object.insert("paused".to_owned(), Value::Bool(expected.paused));
        let devices = device_membership_payload(object, expected);
        object.insert("devices".to_owned(), devices);
        self.put_json("put folder config", &path, payload)?;
        let actual = self
            .folder_config(&expected.folder_id)?
            .ok_or_else(|| SyncError::ConfigurationMismatch { operation: "put folder config" })?;
        verify_folder_configuration(expected, &actual)?;
        Ok(actual)
    }

    pub fn remove_folder(&self, folder_id: &FolderId) -> Result<(), SyncError> {
        let path = format!("/rest/config/folders/{}", folder_id.as_str());
        let response = self.send_request(Method::DELETE, "remove folder", &path, None)?;
        if response.status() != StatusCode::NOT_FOUND {
            ensure_success("remove folder", response)?;
        }
        if self.folder_config(folder_id)?.is_some() {
            return Err(SyncError::ConfigurationMismatch { operation: "remove folder" });
        }
        Ok(())
    }

    pub fn patch_folder_paused(
        &self,
        folder_id: &FolderId,
        paused: bool,
    ) -> Result<FolderConfig, SyncError> {
        let path = format!("/rest/config/folders/{}", folder_id.as_str());
        let mut payload = self
            .get_json_value_optional("folder config", &path)?
            .ok_or(SyncError::ConfigurationMismatch { operation: "patch folder paused" })?;
        let object = payload.as_object_mut().ok_or_else(|| SyncError::InvalidResponse {
            operation: "folder config",
            message: "folder configuration must be a JSON object".to_owned(),
        })?;
        object.insert("paused".to_owned(), Value::Bool(paused));
        self.put_json("patch folder paused", &path, payload)?;
        let actual = self
            .folder_config(folder_id)?
            .ok_or_else(|| SyncError::ConfigurationMismatch { operation: "patch folder paused" })?;
        if actual.paused != paused {
            return Err(SyncError::ConfigurationMismatch { operation: "patch folder paused" });
        }
        Ok(actual)
    }

    pub fn pause_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError> {
        self.set_device_paused(device_id, true)
    }

    pub fn resume_device(&self, device_id: &DeviceId) -> Result<DeviceConfig, SyncError> {
        self.set_device_paused(device_id, false)
    }

    pub fn scan_folder(&self, folder_id: &FolderId) -> Result<(), SyncError> {
        let path = format!("/rest/db/scan?folder={}", folder_id.as_str());
        let response = self.send_request(Method::POST, "scan folder", &path, None)?;
        ensure_success("scan folder", response).map(|_| ())
    }

    pub fn read_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
        let url = self.url_with_folder("/rest/db/ignores", folder_id)?;
        let response: IgnoreResponse = self.get_json_url("read folder ignores", url)?;
        Ok(response.ignore)
    }

    pub fn ensure_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
        let current = self.read_ignores(folder_id)?;
        let updated = append_managed_ignore_block(&current)?;
        if updated == current {
            return Ok(current);
        }
        self.write_ignores(folder_id, &updated)
    }

    pub fn repair_managed_ignores(&self, folder_id: &FolderId) -> Result<Vec<String>, SyncError> {
        let current = self.read_ignores(folder_id)?;
        let repaired = repair_managed_ignore_block(&current);
        self.write_ignores(folder_id, &repaired)
    }

    pub fn write_ignores(
        &self,
        folder_id: &FolderId,
        rules: &[String],
    ) -> Result<Vec<String>, SyncError> {
        let url = self.url_with_folder("/rest/db/ignores", folder_id)?;
        let body = IgnoreRequest { ignore: rules };
        let response = self
            .http
            .post(url)
            .header(API_KEY_HEADER, self.api_key.expose_for_header())
            .json(&body)
            .send()
            .map_err(|error| map_request_error("write folder ignores", error))?;
        ensure_success("write folder ignores", response)?;
        let actual = self.read_ignores(folder_id)?;
        if actual != rules {
            return Err(SyncError::ConfigurationMismatch { operation: "verify folder ignores" });
        }
        Ok(actual)
    }

    pub fn events_since(
        &self,
        cursor: EventCursor,
        timeout_seconds: u16,
    ) -> Result<Vec<SyncEvent>, SyncError> {
        let mut url = self.endpoint.join("/rest/events")?;
        url.query_pairs_mut()
            .append_pair("since", &cursor.0.to_string())
            .append_pair("timeout", &timeout_seconds.min(MAX_EVENT_TIMEOUT_SECONDS).to_string());
        let response: Vec<EventResponse> = self.get_json_url("events", url)?;
        let remote_events = response
            .into_iter()
            .map(|event| RemoteEvent { id: event.id, kind: event_kind(&event.event_type) })
            .collect();
        Ok(reduce_event_batch(cursor, remote_events).events)
    }

    pub fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus, SyncError> {
        let url = self.url_with_folder("/rest/db/status", folder)?;
        let response: FolderStatusResponse = self.get_json_url("folder status", url)?;
        let completion_percent = if response.global_bytes == 0 {
            if response.need_bytes == 0 { 100.0 } else { 0.0 }
        } else {
            let completed_bytes = response.global_bytes.saturating_sub(response.need_bytes);
            (completed_bytes as f64 / response.global_bytes as f64 * 100.0).clamp(0.0, 100.0)
        };
        Ok(FolderStatus {
            phase: folder_phase(&response.state),
            need_bytes: response.need_bytes,
            need_items: response.need_items(),
            completion_percent,
            errors: response.error_count(),
        })
    }

    pub fn folder_errors(&self, folder: &FolderId) -> Result<Vec<String>, SyncError> {
        let url = self.url_with_folder("/rest/folder/errors", folder)?;
        let response: FolderErrorsResponse = self.get_json_url("folder errors", url)?;
        Ok(response
            .errors
            .into_iter()
            .map(|item| {
                if item.filename.is_empty() {
                    item.error
                } else {
                    format!("{}: {}", item.filename, item.error)
                }
            })
            .collect())
    }

    fn url_with_folder(&self, path: &str, folder: &FolderId) -> Result<reqwest::Url, SyncError> {
        let mut url = self.endpoint.join(path)?;
        url.query_pairs_mut().append_pair("folder", folder.as_str());
        Ok(url)
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        path: &str,
    ) -> Result<T, SyncError> {
        let url = self.endpoint.join(path)?;
        self.get_json_url(operation, url)
    }

    fn get_json_optional<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        path: &str,
    ) -> Result<Option<T>, SyncError> {
        let url = self.endpoint.join(path)?;
        let response = self
            .http
            .get(url)
            .header(API_KEY_HEADER, self.api_key.expose_for_header())
            .send()
            .map_err(|error| map_request_error(operation, error))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = ensure_success(operation, response)?;
        response
            .json::<T>()
            .map(Some)
            .map_err(|error| SyncError::InvalidResponse { operation, message: error.to_string() })
    }

    fn get_json_value_optional(
        &self,
        operation: &'static str,
        path: &str,
    ) -> Result<Option<Value>, SyncError> {
        self.get_json_optional(operation, path)
    }

    fn get_json_value(&self, operation: &'static str, path: &str) -> Result<Value, SyncError> {
        self.get_json(operation, path)
    }

    fn put_json(&self, operation: &'static str, path: &str, body: Value) -> Result<(), SyncError> {
        let response = self.send_request(Method::PUT, operation, path, Some(body))?;
        ensure_success(operation, response).map(|_| ())
    }

    fn send_request(
        &self,
        method: Method,
        operation: &'static str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Response, SyncError> {
        let url = self.endpoint.join(path)?;
        let mut request =
            self.http.request(method, url).header(API_KEY_HEADER, self.api_key.expose_for_header());
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().map_err(|error| map_request_error(operation, error))
    }

    fn set_device_paused(
        &self,
        device_id: &DeviceId,
        paused: bool,
    ) -> Result<DeviceConfig, SyncError> {
        let action = if paused { "pause" } else { "resume" };
        let path = format!("/rest/system/{action}?device={}", device_id.as_str());
        let response = self.send_request(Method::POST, "set device pause state", &path, None)?;
        ensure_success("set device pause state", response)?;
        let actual = self.device_config(device_id)?.ok_or_else(|| {
            SyncError::ConfigurationMismatch { operation: "verify device pause state" }
        })?;
        if actual.paused != paused {
            return Err(SyncError::ConfigurationMismatch {
                operation: "verify device pause state",
            });
        }
        Ok(actual)
    }

    fn get_json_url<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        url: reqwest::Url,
    ) -> Result<T, SyncError> {
        let response = self
            .http
            .get(url)
            .header(API_KEY_HEADER, self.api_key.expose_for_header())
            .send()
            .map_err(|error| map_request_error(operation, error))?;
        let response = ensure_success(operation, response)?;
        response
            .json::<T>()
            .map_err(|error| SyncError::InvalidResponse { operation, message: error.to_string() })
    }
}

fn parse_device_config(
    response: DeviceConfigResponse,
    operation: &'static str,
    requested_device_id: Option<&DeviceId>,
) -> Result<DeviceConfig, SyncError> {
    let device_id =
        DeviceId::parse(response.device_id).map_err(|_| SyncError::InvalidResponse {
            operation,
            message: "response contains an invalid device ID".to_owned(),
        })?;
    if requested_device_id.is_some_and(|requested| requested != &device_id) {
        return Err(SyncError::InvalidResponse {
            operation,
            message: "response device ID does not match the requested device".to_owned(),
        });
    }
    Ok(DeviceConfig::new(device_id, response.name, response.addresses, response.paused))
}

fn device_membership_payload(object: &Map<String, Value>, expected: &FolderConfig) -> Value {
    let existing = object.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut devices = Vec::with_capacity(expected.devices.len());
    for device_id in &expected.devices {
        let mut device = existing
            .iter()
            .find(|candidate| {
                candidate
                    .get("deviceID")
                    .and_then(Value::as_str)
                    .is_some_and(|candidate_id| candidate_id == device_id.as_str())
            })
            .cloned()
            .unwrap_or_else(|| json!({"deviceID": device_id.as_str()}));
        if let Some(device_object) = device.as_object_mut() {
            device_object
                .insert("deviceID".to_owned(), Value::String(device_id.as_str().to_owned()));
        }
        devices.push(device);
    }
    Value::Array(devices)
}

fn verify_device_configuration(
    expected: &DeviceConfig,
    actual: &DeviceConfig,
) -> Result<(), SyncError> {
    if expected.device_id != actual.device_id
        || expected.name != actual.name
        || expected.addresses != actual.addresses
        || expected.paused != actual.paused
    {
        return Err(SyncError::ConfigurationMismatch { operation: "verify device config" });
    }
    Ok(())
}

fn verify_folder_configuration(
    expected: &FolderConfig,
    actual: &FolderConfig,
) -> Result<(), SyncError> {
    if expected.folder_id != actual.folder_id
        || expected.label != actual.label
        || expected.path != actual.path
        || expected.paused != actual.paused
        || expected.devices != actual.devices
    {
        return Err(SyncError::ConfigurationMismatch { operation: "verify folder config" });
    }
    Ok(())
}

fn parse_folder_config(
    response: FolderConfigResponse,
    operation: &'static str,
    requested_folder_id: Option<&FolderId>,
) -> Result<FolderConfig, SyncError> {
    let folder_id = FolderId::new(response.folder_id).map_err(|_| SyncError::InvalidResponse {
        operation,
        message: "response contains an invalid folder ID".to_owned(),
    })?;
    if requested_folder_id.is_some_and(|requested| requested != &folder_id) {
        return Err(SyncError::InvalidResponse {
            operation,
            message: "response folder ID does not match the requested folder".to_owned(),
        });
    }
    let devices = response
        .devices
        .into_iter()
        .map(|device| {
            DeviceId::parse(device.device_id).map_err(|_| SyncError::InvalidResponse {
                operation,
                message: "response contains an invalid folder device ID".to_owned(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FolderConfig::new(folder_id, response.label, response.path, response.paused, devices))
}

fn parse_version(raw: &str) -> Result<semver::Version, SyncError> {
    let normalized = raw.strip_prefix('v').unwrap_or(raw);
    semver::Version::parse(normalized).map_err(|error| SyncError::InvalidResponse {
        operation: "system version",
        message: format!("invalid version: {error}"),
    })
}

fn ensure_supported_version(version: &semver::Version) -> Result<(), SyncError> {
    let minimum = semver::Version::new(
        MIN_SUPPORTED_VERSION.0,
        MIN_SUPPORTED_VERSION.1,
        MIN_SUPPORTED_VERSION.2,
    );
    let maximum = semver::Version::new(
        MAX_SUPPORTED_VERSION.0,
        MAX_SUPPORTED_VERSION.1,
        MAX_SUPPORTED_VERSION.2,
    );
    if version < &minimum || version >= &maximum {
        return Err(SyncError::IncompatibleVersion { found: version.clone() });
    }
    Ok(())
}

fn folder_phase(raw: &str) -> FolderPhase {
    match raw {
        "idle" => FolderPhase::Idle,
        "scanning" => FolderPhase::Scanning,
        "syncing" => FolderPhase::Syncing,
        "paused" => FolderPhase::Paused,
        "error" => FolderPhase::Error,
        _ => FolderPhase::Unknown,
    }
}

fn event_kind(raw: &str) -> Option<SyncEventKind> {
    match raw {
        "DeviceConnected" => Some(SyncEventKind::DeviceConnected),
        "DeviceDisconnected" => Some(SyncEventKind::DeviceDisconnected),
        "FolderStateChanged" => Some(SyncEventKind::FolderStateChanged),
        "ItemFinished" => Some(SyncEventKind::ItemFinished),
        "ConfigSaved" | "ConfigLoaded" => Some(SyncEventKind::ConfigurationChanged),
        "FolderErrors" | "RemoteError" => Some(SyncEventKind::RemoteError),
        _ => None,
    }
}

fn ensure_success(operation: &'static str, response: Response) -> Result<Response, SyncError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SyncError::Authentication),
        status if !status.is_success() => {
            Err(SyncError::Remote { operation, status: status.as_u16() })
        }
        _ => Ok(response),
    }
}

fn map_request_error(operation: &'static str, error: reqwest::Error) -> SyncError {
    if error.is_timeout() {
        SyncError::RequestTimeout { operation }
    } else if error.is_connect() {
        SyncError::ConnectionRefused
    } else {
        SyncError::InvalidResponse { operation, message: "request failed".to_owned() }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread::{self, JoinHandle};

    use super::SyncthingClient;
    use crate::{
        ApiKey, DeviceId, EventCursor, FolderPhase, LoopbackEndpoint, SyncError, SyncEvent,
        SyncEventKind,
    };

    const DEVICE_ID: &str = "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG";

    struct MockServer {
        endpoint: LoopbackEndpoint,
        thread: JoinHandle<Vec<String>>,
    }

    impl MockServer {
        fn start(responses: Vec<(u16, String)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
            let address = listener.local_addr().expect("mock server should expose address");
            let thread = thread::spawn(move || {
                let mut requests = Vec::new();
                for (status, body) in responses {
                    let (mut stream, _) = listener.accept().expect("mock server should accept");
                    requests.push(read_request(&mut stream));
                    write_response(&mut stream, status, &body);
                }
                requests
            });
            let endpoint_url = format!("http://{address}");
            let endpoint =
                LoopbackEndpoint::parse(&endpoint_url).expect("mock server endpoint should parse");
            Self { endpoint, thread }
        }

        fn join(self) -> Vec<String> {
            self.thread.join().expect("mock server should stop cleanly")
        }
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = stream.read(&mut buffer).expect("mock server should read");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let content_length = String::from_utf8_lossy(&request[..header_end])
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                let request_length = header_end + 4 + content_length;
                if request.len() >= request_length {
                    break;
                }
            }
        }
        String::from_utf8(request).expect("mock request should be UTF-8")
    }

    fn request_body(request: &str) -> &str {
        request.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or("")
    }

    fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
        let reason = if status == 200 { "OK" } else { "Error" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("mock server should write");
    }

    #[test]
    fn probe_reads_version_and_device_id_with_api_key_header() {
        let server = MockServer::start(vec![
            (200, r#"{"version":"v2.1.1"}"#.to_owned()),
            (200, format!(r#"{{"myID":"{DEVICE_ID}"}}"#)),
        ]);
        let secret = "test-api-key";
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new(secret.to_owned()).expect("test key should parse"),
        )
        .expect("client should build");

        let instance = client.probe().expect("probe should succeed");
        assert_eq!(instance.version.to_string(), "2.1.1");
        assert_eq!(instance.device_id.as_str(), DEVICE_ID);

        let requests = server.join();
        assert!(requests[0].starts_with("GET /rest/system/version HTTP/1.1"));
        assert!(requests[0].to_ascii_lowercase().contains("x-api-key: test-api-key"));
        assert!(requests[1].starts_with("GET /rest/system/status HTTP/1.1"));
    }

    #[test]
    fn maps_authentication_failure_without_exposing_api_key() {
        let server =
            MockServer::start(vec![(401, r#"{"error":"secret must not leak"}"#.to_owned())]);
        let secret = "do-not-leak-this-key";
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new(secret.to_owned()).expect("test key should parse"),
        )
        .expect("client should build");

        let error = client.probe().expect_err("401 should fail");
        assert!(matches!(error, SyncError::Authentication));
        assert!(!error.to_string().contains(secret));
        let _ = server.join();
    }

    #[test]
    fn rejects_invalid_json_as_a_typed_response_error() {
        let server = MockServer::start(vec![(200, "{\"version\":".to_owned())]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");

        let error = client.probe().expect_err("invalid JSON should fail");
        assert!(matches!(error, SyncError::InvalidResponse { operation: "system version", .. }));
        let _ = server.join();
    }

    #[test]
    fn rejects_versions_outside_the_supported_minor_range() {
        let server = MockServer::start(vec![(200, r#"{"version":"v2.2.0"}"#.to_owned())]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");

        let error = client.probe().expect_err("2.2.0 should be rejected");
        assert!(matches!(error, SyncError::IncompatibleVersion { .. }));
        let _ = server.join();
    }

    #[test]
    fn maps_folder_state_to_stable_phase_and_completion() {
        let server = MockServer::start(vec![(
            200,
            r#"{"state":"syncing","needBytes":25,"needItems":2,"globalBytes":100,"inSyncBytes":75,"errors":1}"#.to_owned(),
        )]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let folder = crate::FolderId::new("notes".to_owned()).expect("folder should parse");

        let status = client.folder_status(&folder).expect("folder status should parse");
        assert!(matches!(status.phase, FolderPhase::Syncing));
        assert_eq!(status.need_bytes, 25);
        assert_eq!(status.need_items, 2);
        assert_eq!(status.completion_percent, 75.0);
        assert_eq!(status.errors, 1);
        let _ = server.join();
    }

    #[test]
    fn events_since_preserves_cursor_query_and_ignores_unknown_events() {
        let server = MockServer::start(vec![(
            200,
            r#"[{"id":5,"type":"FolderStateChanged","data":{}},{"id":6,"type":"NewFutureEvent","data":{}}]"#.to_owned(),
        )]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");

        let events = client.events_since(EventCursor(4), 30).expect("event request should succeed");
        assert_eq!(
            events,
            vec![SyncEvent::Remote { id: 5, kind: SyncEventKind::FolderStateChanged }]
        );
        let requests = server.join();
        assert!(requests[0].starts_with("GET /rest/events?since=4&timeout=30 HTTP/1.1"));
    }

    #[test]
    fn reads_device_folder_and_default_configuration_without_leaking_api_key() {
        let server = MockServer::start(vec![
            (
                200,
                format!(
                    r#"{{"deviceID":"{DEVICE_ID}","name":"Remote","addresses":["tcp://sync.example.com:22000"],"paused":false,"extra":"preserved"}}"#
                ),
            ),
            (
                200,
                format!(
                    r#"{{"id":"notes","label":"Notes","path":"/tmp/notes","paused":true,"devices":[{{"deviceID":"{DEVICE_ID}"}}],"rescanIntervalS":60}}"#
                ),
            ),
            (
                200,
                format!(
                    r#"{{"deviceID":"{DEVICE_ID}","name":"Default","addresses":["dynamic"],"paused":false}}"#
                ),
            ),
            (
                200,
                format!(
                    r#"{{"id":"default-folder","label":"Default","path":"","paused":false,"devices":[{{"deviceID":"{DEVICE_ID}"}}]}}"#
                ),
            ),
        ]);
        let api_key = "test-api-key";
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new(api_key.to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let device_id = DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse");
        let folder_id = crate::FolderId::new("notes".to_owned()).expect("folder ID should parse");

        let device = client
            .device_config(&device_id)
            .expect("device config should parse")
            .expect("device config should exist");
        assert_eq!(device.name, "Remote");
        assert_eq!(device.addresses, vec!["tcp://sync.example.com:22000"]);
        let folder = client
            .folder_config(&folder_id)
            .expect("folder config should parse")
            .expect("folder config should exist");
        assert_eq!(folder.path, std::path::PathBuf::from("/tmp/notes"));
        assert!(folder.paused);
        assert_eq!(folder.devices, vec![device_id.clone()]);
        assert_eq!(client.default_device().expect("default device should parse").name, "Default");
        assert_eq!(
            client.default_folder().expect("default folder should parse").folder_id.as_str(),
            "default-folder"
        );

        let requests = server.join();
        assert!(requests[0].starts_with("GET /rest/config/devices/"));
        assert!(requests[1].starts_with("GET /rest/config/folders/notes"));
        assert!(requests[2].starts_with("GET /rest/config/defaults/device"));
        assert!(requests[3].starts_with("GET /rest/config/defaults/folder"));
        assert!(
            requests.iter().all(|request| {
                request.to_ascii_lowercase().contains("x-api-key: test-api-key")
            })
        );
        assert!(requests.iter().all(|request| !request.contains("extra")));
    }

    #[test]
    fn treats_missing_configuration_resource_as_none_and_rejects_mismatched_ids() {
        let server = MockServer::start(vec![
            (404, r#"{"error":"not found"}"#.to_owned()),
            (200, r#"{"id":"other","label":"Notes","path":"/tmp/notes","devices":[]}"#.to_owned()),
        ]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let device_id = DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse");
        let folder_id = crate::FolderId::new("notes".to_owned()).expect("folder ID should parse");

        assert!(
            client.device_config(&device_id).expect("missing device should be handled").is_none()
        );
        let error = client.folder_config(&folder_id).expect_err("mismatched ID should fail");
        assert!(matches!(error, SyncError::InvalidResponse { operation: "folder config", .. }));
        let _ = server.join();
    }

    #[test]
    fn writes_only_owned_device_fields_and_preserves_unknown_fields() {
        let server = MockServer::start(vec![
            (
                200,
                format!(
                    r#"{{"deviceID":"{DEVICE_ID}","name":"Old","addresses":["dynamic"],"paused":true,"customArray":[1,2]}}"#
                ),
            ),
            (200, "{}".to_owned()),
            (
                200,
                format!(
                    r#"{{"deviceID":"{DEVICE_ID}","name":"Remote","addresses":["tcp://sync.example.com:22000"],"paused":false,"customArray":[1,2]}}"#
                ),
            ),
        ]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let device_id = DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse");
        let expected = crate::DeviceConfig::new(
            device_id,
            "Remote".to_owned(),
            vec!["tcp://sync.example.com:22000".to_owned()],
            false,
        );

        let actual = client.put_device(&expected).expect("device config should be written");
        assert_eq!(actual, expected);
        let requests = server.join();
        assert!(requests[1].starts_with("PUT /rest/config/devices/"));
        let body = request_body(&requests[1]);
        assert!(body.contains(r#""customArray":[1,2]"#));
        assert!(body.contains(r#""name":"Remote"#));
        assert!(body.contains(r#""addresses":["tcp://sync.example.com:22000"]"#));
    }

    #[test]
    fn writes_folder_membership_while_preserving_unknown_nested_fields() {
        let server = MockServer::start(vec![
            (
                200,
                format!(
                    r#"{{"id":"notes","label":"Old","path":"/tmp/old","paused":true,"customArray":["keep"],"devices":[{{"deviceID":"{DEVICE_ID}","introducedBy":"manual"}}]}}"#
                ),
            ),
            (200, "{}".to_owned()),
            (
                200,
                format!(
                    r#"{{"id":"notes","label":"Notes","path":"/tmp/notes","paused":false,"customArray":["keep"],"devices":[{{"deviceID":"{DEVICE_ID}","introducedBy":"manual"}}]}}"#
                ),
            ),
        ]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let device_id = DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse");
        let expected = crate::FolderConfig::new(
            crate::FolderId::new("notes".to_owned()).expect("folder ID should parse"),
            "Notes".to_owned(),
            std::path::PathBuf::from("/tmp/notes"),
            false,
            vec![device_id],
        );

        let actual = client.put_folder(&expected).expect("folder config should be written");
        assert_eq!(actual, expected);
        let requests = server.join();
        assert!(requests[1].starts_with("PUT /rest/config/folders/notes"));
        let body = request_body(&requests[1]);
        assert!(body.contains(r#""customArray":["keep"]"#));
        assert!(body.contains(r#""introducedBy":"manual"#));
    }

    #[test]
    fn supports_idempotent_removal_pause_and_scan_commands() {
        let folder_server = MockServer::start(vec![
            (204, String::new()),
            (404, r#"{"error":"not found"}"#.to_owned()),
        ]);
        let client = SyncthingClient::new(
            folder_server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let folder = crate::FolderId::new("notes".to_owned()).expect("folder ID should parse");
        client.remove_folder(&folder).expect("missing folder removal should be idempotent");
        let requests = folder_server.join();
        assert!(requests[0].starts_with("DELETE /rest/config/folders/notes"));

        let device_server = MockServer::start(vec![
            (200, String::new()),
            (
                200,
                format!(
                    r#"{{"deviceID":"{DEVICE_ID}","name":"Remote","addresses":[],"paused":true}}"#
                ),
            ),
        ]);
        let client = SyncthingClient::new(
            device_server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let device = DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse");
        client.pause_device(&device).expect("device should pause");
        let requests = device_server.join();
        assert!(requests[0].starts_with("POST /rest/system/pause?device="));

        let scan_server = MockServer::start(vec![(200, String::new())]);
        let client = SyncthingClient::new(
            scan_server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        client.scan_folder(&folder).expect("folder should scan");
        let requests = scan_server.join();
        assert!(requests[0].starts_with("POST /rest/db/scan?folder=notes"));
    }

    #[test]
    fn ensures_managed_ignores_preserves_user_rules_and_verifies_the_write() {
        let managed_rules = vec![
            "# user rule".to_owned(),
            "".to_owned(),
            crate::TEXTORA_MANAGED_BEGIN.to_owned(),
            crate::TEXTORA_MANAGED_RULE.to_owned(),
            crate::TEXTORA_MANAGED_END.to_owned(),
        ];
        let server = MockServer::start(vec![
            (200, "{\"ignore\":[\"# user rule\"]}".to_owned()),
            (200, String::new()),
            (200, serde_json::json!({"ignore": managed_rules}).to_string()),
        ]);
        let client = SyncthingClient::new(
            server.endpoint.clone(),
            ApiKey::new("test-api-key".to_owned()).expect("test key should parse"),
        )
        .expect("client should build");
        let folder = crate::FolderId::new("notes".to_owned()).expect("folder ID should parse");

        let actual =
            client.ensure_managed_ignores(&folder).expect("managed ignore block should be written");
        assert_eq!(actual, managed_rules);
        let requests = server.join();
        assert!(requests[0].starts_with("GET /rest/db/ignores?folder=notes"));
        assert!(requests[1].starts_with("POST /rest/db/ignores?folder=notes"));
        assert!(request_body(&requests[1]).contains(crate::TEXTORA_MANAGED_RULE));
        assert!(requests[2].starts_with("GET /rest/db/ignores?folder=notes"));
    }

    #[allow(dead_code)]
    fn _keep_types_referenced(_: DeviceId) {}
}
