use std::path::PathBuf;

use serde::Deserialize;

use crate::{DeviceId, FolderId};

pub const SYNCTHING_DYNAMIC_ADDRESS: &str = "dynamic";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticSyncAddress(String);

impl StaticSyncAddress {
    pub fn parse(candidate: String) -> Result<Self, crate::SyncError> {
        let url =
            reqwest::Url::parse(&candidate).map_err(|_| crate::SyncError::InvalidEndpoint {
                reason: "static sync address must be a valid URL".to_owned(),
            })?;
        if !matches!(url.scheme(), "tcp" | "quic" | "relay") || url.host_str().is_none() {
            return Err(crate::SyncError::InvalidEndpoint {
                reason: "static sync address must use tcp, quic, or relay with a host".to_owned(),
            });
        }
        Ok(Self(candidate))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteDeviceSpec {
    pub device_id: DeviceId,
    pub name: String,
    pub addresses: Vec<StaticSyncAddress>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceConfig {
    pub device_id: DeviceId,
    pub name: String,
    pub addresses: Vec<String>,
    pub paused: bool,
}

impl DeviceConfig {
    pub fn new(device_id: DeviceId, name: String, addresses: Vec<String>, paused: bool) -> Self {
        Self { device_id, name, addresses, paused }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderConfig {
    pub folder_id: FolderId,
    pub label: String,
    pub path: PathBuf,
    pub paused: bool,
    pub devices: Vec<DeviceId>,
}

impl FolderConfig {
    pub fn new(
        folder_id: FolderId,
        label: String,
        path: PathBuf,
        paused: bool,
        devices: Vec<DeviceId>,
    ) -> Self {
        Self { folder_id, label, path, paused, devices }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationDifference {
    MissingDevice,
    MissingFolder,
    DeviceAddressChanged,
    PathChanged,
    DeviceMembershipChanged,
    ManagedIgnoreChanged,
    PauseStateChanged,
}

pub fn compare_device_configuration(
    expected: &DeviceConfig,
    actual: Option<&DeviceConfig>,
) -> Vec<ConfigurationDifference> {
    let Some(actual) = actual else {
        return vec![ConfigurationDifference::MissingDevice];
    };
    let mut differences = Vec::new();
    if expected.addresses != actual.addresses {
        differences.push(ConfigurationDifference::DeviceAddressChanged);
    }
    if expected.paused != actual.paused {
        differences.push(ConfigurationDifference::PauseStateChanged);
    }
    differences
}

pub fn compare_folder_configuration(
    expected: &FolderConfig,
    actual: Option<&FolderConfig>,
) -> Vec<ConfigurationDifference> {
    let Some(actual) = actual else {
        return vec![ConfigurationDifference::MissingFolder];
    };
    let mut differences = Vec::new();
    if expected.path != actual.path {
        differences.push(ConfigurationDifference::PathChanged);
    }
    if expected.devices != actual.devices {
        differences.push(ConfigurationDifference::DeviceMembershipChanged);
    }
    if expected.paused != actual.paused {
        differences.push(ConfigurationDifference::PauseStateChanged);
    }
    differences
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceInfo {
    pub version: semver::Version,
    pub device_id: DeviceId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FolderPhase {
    Idle,
    Scanning,
    Syncing,
    Paused,
    Error,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FolderStatus {
    pub phase: FolderPhase,
    pub need_bytes: u64,
    pub need_items: u64,
    pub completion_percent: f64,
    pub errors: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingDevice {
    pub device_id: DeviceId,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingFolder {
    pub folder_id: FolderId,
    pub label: Option<String>,
    pub offered_by: DeviceId,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SystemVersionResponse {
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SystemStatusResponse {
    #[serde(rename = "myID")]
    pub device_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConnectionsResponse {
    #[serde(default)]
    pub connections: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PendingDeviceResponse {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PendingFolderResponse {
    #[serde(rename = "offeredBy", default)]
    pub offered_by: std::collections::BTreeMap<String, PendingFolderOffer>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PendingFolderOffer {
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FolderStatusResponse {
    #[serde(default)]
    pub state: String,
    #[serde(rename = "needBytes", default)]
    pub need_bytes: u64,
    #[serde(rename = "needTotalItems", default)]
    pub need_total_items: u64,
    #[serde(rename = "needItems", default)]
    pub need_items: u64,
    #[serde(rename = "needFiles", default)]
    pub need_files: u64,
    #[serde(rename = "globalBytes", default)]
    pub global_bytes: u64,
    #[serde(rename = "errors", default)]
    pub errors: u64,
    #[serde(rename = "pullErrors", default)]
    pub pull_errors: u64,
}

impl FolderStatusResponse {
    pub(crate) fn need_items(&self) -> u64 {
        if self.need_total_items > 0 {
            self.need_total_items
        } else if self.need_items > 0 {
            self.need_items
        } else {
            self.need_files
        }
    }

    pub(crate) fn error_count(&self) -> u64 {
        self.errors.max(self.pull_errors)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct FolderErrorsResponse {
    #[serde(default)]
    pub errors: Vec<FolderErrorItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FolderErrorItem {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeviceConfigResponse {
    #[serde(rename = "deviceID")]
    pub device_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FolderConfigResponse {
    #[serde(rename = "id")]
    pub folder_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub devices: Vec<FolderDeviceResponse>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FolderDeviceResponse {
    #[serde(rename = "deviceID")]
    pub device_id: String,
}

#[cfg(test)]
mod config_tests {
    use std::path::PathBuf;

    use super::{
        ConfigurationDifference, DeviceConfig, FolderConfig, StaticSyncAddress,
        compare_device_configuration, compare_folder_configuration,
    };
    use crate::{DeviceId, FolderId};

    const DEVICE_ID: &str = "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG";

    #[test]
    fn validates_static_sync_addresses_and_compares_owned_fields() {
        assert!(StaticSyncAddress::parse("tcp://sync.example.com:22000".to_owned()).is_ok());
        assert!(StaticSyncAddress::parse("dynamic".to_owned()).is_err());
        assert!(StaticSyncAddress::parse("tcp:///22000".to_owned()).is_err());

        let device = DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse");
        let expected_device = DeviceConfig::new(
            device.clone(),
            "Remote".to_owned(),
            vec!["tcp://sync.example.com:22000".to_owned()],
            false,
        );
        let actual_device = DeviceConfig::new(device.clone(), "Remote".to_owned(), vec![], true);
        assert_eq!(
            compare_device_configuration(&expected_device, Some(&actual_device)),
            vec![
                ConfigurationDifference::DeviceAddressChanged,
                ConfigurationDifference::PauseStateChanged
            ]
        );

        let folder_id = FolderId::new("notes".to_owned()).expect("folder ID should parse");
        let expected_folder = FolderConfig::new(
            folder_id.clone(),
            "Notes".to_owned(),
            PathBuf::from("/tmp/notes"),
            false,
            vec![device.clone()],
        );
        let actual_folder = FolderConfig::new(
            folder_id,
            "Notes".to_owned(),
            PathBuf::from("/tmp/other"),
            true,
            Vec::new(),
        );
        assert_eq!(
            compare_folder_configuration(&expected_folder, Some(&actual_folder)),
            vec![
                ConfigurationDifference::PathChanged,
                ConfigurationDifference::DeviceMembershipChanged,
                ConfigurationDifference::PauseStateChanged
            ]
        );
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventResponse {
    pub id: u64,
    #[serde(rename = "type")]
    pub event_type: String,
}
