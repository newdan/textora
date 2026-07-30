use crate::library_registry::{LibraryRegistrationState, ProvisioningStage};
use crate::sync_controller::{SyncConnectionState, SyncControllerSnapshot, SyncNotice};
use crate::sync_settings_types::{
    LibrarySyncState, LibraryView, PendingFolderView, SyncConnectionView, SyncNoticeSeverity,
    SyncNoticeView, SyncSettingsInput,
};

pub(crate) fn empty_sync_settings_input() -> SyncSettingsInput {
    SyncSettingsInput::default()
}

pub(crate) fn build_sync_settings_input(
    snapshot: &SyncControllerSnapshot,
    notices: &[SyncNotice],
) -> SyncSettingsInput {
    SyncSettingsInput {
        endpoint: snapshot.endpoint.clone().unwrap_or_default(),
        has_api_key: snapshot.has_api_key,
        connection: map_connection(&snapshot.connection),
        libraries: snapshot.libraries.iter().map(map_library).collect(),
        pending_folders: snapshot
            .pending_folders
            .iter()
            .map(|pending| PendingFolderView {
                folder_id: pending.folder_id.as_str().to_owned(),
                offered_by: pending.offered_by.as_str().to_owned(),
            })
            .collect(),
        notices: notices.iter().map(map_notice).collect(),
    }
}

fn map_connection(state: &SyncConnectionState) -> SyncConnectionView {
    match state {
        SyncConnectionState::NotConfigured => SyncConnectionView::NotConfigured,
        SyncConnectionState::Connecting => SyncConnectionView::Connecting,
        SyncConnectionState::Connected { instance } => SyncConnectionView::Connected {
            device_id: instance.device_id.as_str().to_owned(),
            version: instance.version.to_string(),
        },
        SyncConnectionState::AuthenticationRequired => SyncConnectionView::AuthenticationRequired,
        SyncConnectionState::Incompatible { found } => {
            SyncConnectionView::Incompatible { found: found.to_string() }
        }
        SyncConnectionState::Unavailable { message } => {
            SyncConnectionView::Unavailable { message: format!("Syncthing 不可用：{message}") }
        }
    }
}

fn map_library(record: &crate::library_registry::LibraryRecord) -> LibraryView {
    let state = match &record.state {
        LibraryRegistrationState::Provisioning { stage } => match stage {
            ProvisioningStage::Scanning => LibrarySyncState::Scanning,
            ProvisioningStage::AwaitingRemoteAcceptance => {
                LibrarySyncState::AwaitingRemoteAcceptance
            }
            ProvisioningStage::RegisteringDevice
            | ProvisioningStage::RegisteringFolder
            | ProvisioningStage::ConfiguringIgnores => LibrarySyncState::Pending,
        },
        LibraryRegistrationState::Active => LibrarySyncState::UpToDate,
        LibraryRegistrationState::Paused => LibrarySyncState::Paused,
        LibraryRegistrationState::ConfigurationMismatch => LibrarySyncState::ConfigurationMismatch,
        LibraryRegistrationState::Error { message } => {
            LibrarySyncState::Error { message: stable_library_error(message) }
        }
    };
    let can_repair = matches!(record.state, LibraryRegistrationState::ConfigurationMismatch);
    LibraryView {
        name: record.remote.name.clone(),
        root_display: record.root.display().to_string(),
        state,
        can_repair,
        can_remove_mapping: true,
        can_unregister: record.folder_created_by_textora,
    }
}

fn stable_library_error(message: &str) -> String {
    format!("同步资料库失败：{message}")
}

fn map_notice(notice: &SyncNotice) -> SyncNoticeView {
    let (severity, message) = match notice {
        SyncNotice::LibraryError { message, .. } => {
            (SyncNoticeSeverity::Error, format!("同步操作失败：{message}"))
        }
        SyncNotice::LocalError { message } => {
            (SyncNoticeSeverity::Error, format!("同步操作失败：{message}"))
        }
        SyncNotice::RemoteEvent(textora_sync::SyncEvent::FullRefreshRequired) => {
            (SyncNoticeSeverity::Warning, "同步状态需要刷新".to_owned())
        }
        SyncNotice::RemoteEvent(textora_sync::SyncEvent::Remote { kind, .. }) => {
            (SyncNoticeSeverity::Info, format!("同步状态已更新：{}", event_kind_label(kind)))
        }
    };
    SyncNoticeView { severity, message }
}

fn event_kind_label(kind: &textora_sync::SyncEventKind) -> &'static str {
    match kind {
        textora_sync::SyncEventKind::DeviceConnected => "设备已连接",
        textora_sync::SyncEventKind::DeviceDisconnected => "设备已断开",
        textora_sync::SyncEventKind::FolderStateChanged => "资料库状态已更新",
        textora_sync::SyncEventKind::ItemFinished => "文件同步完成",
        textora_sync::SyncEventKind::ConfigurationChanged => "配置已变化",
        textora_sync::SyncEventKind::RemoteError => "远端报告错误",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_registry::{
        LibraryOrigin, LibraryRecord, LibraryRegistrationState, ProvisioningStage,
        RegisteredRemoteDevice,
    };
    use crate::sync_controller::{
        RequestId, SyncConnectionState, SyncControllerSnapshot, SyncNotice,
    };
    use crate::sync_settings_types::{LibrarySyncState, SyncConnectionView, SyncNoticeSeverity};
    use semver::Version;
    use std::path::PathBuf;
    use textora_sync::{DeviceId, EventCursor, FolderId, InstanceInfo, PendingFolder, SyncEvent};

    const LOCAL_DEVICE_ID: &str = "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG";
    const REMOTE_DEVICE_ID: &str =
        "BCDEFGH-BCDEFGH-BCDEFGH-BCDEFGH-BCDEFGH-BCDEFGH-BCDEFGH-BCDEFGH";

    fn device_id(value: &str) -> DeviceId {
        DeviceId::parse(value.to_owned()).expect("test device ID should be valid")
    }

    fn folder_id(value: &str) -> FolderId {
        FolderId::new(value.to_owned()).expect("test folder ID should be valid")
    }

    fn snapshot(connection: SyncConnectionState) -> SyncControllerSnapshot {
        SyncControllerSnapshot {
            connection,
            endpoint: Some("http://127.0.0.1:8384".to_owned()),
            has_api_key: true,
            last_request_id: Some(RequestId(7)),
            event_cursor: EventCursor(3),
            libraries: vec![LibraryRecord {
                library_id: "library-1".to_owned(),
                root: PathBuf::from("/tmp/notes"),
                folder_id: folder_id("notes"),
                remote: RegisteredRemoteDevice {
                    device_id: device_id(REMOTE_DEVICE_ID),
                    name: "Remote".to_owned(),
                    addresses: vec!["dynamic".to_owned()],
                },
                origin: LibraryOrigin::Published,
                state: LibraryRegistrationState::Provisioning {
                    stage: ProvisioningStage::AwaitingRemoteAcceptance,
                },
                device_created_by_textora: true,
                folder_created_by_textora: true,
            }],
            pending_folders: vec![PendingFolder {
                folder_id: folder_id("incoming"),
                label: Some("Incoming".to_owned()),
                offered_by: device_id(REMOTE_DEVICE_ID),
            }],
        }
    }

    #[test]
    fn sync_settings_input_maps_snapshot_without_api_key_value() {
        let input = build_sync_settings_input(&snapshot(SyncConnectionState::NotConfigured), &[]);

        assert_eq!(input.endpoint, "http://127.0.0.1:8384");
        assert!(input.has_api_key);
        assert_eq!(input.connection, SyncConnectionView::NotConfigured);
        assert!(!format!("{input:?}").contains("secret"));
    }

    #[test]
    fn maps_connection_and_library_snapshot_to_settings_input_without_secret_material() {
        let snapshot = snapshot(SyncConnectionState::Connected {
            instance: InstanceInfo {
                version: Version::parse("2.1.1").expect("test version should parse"),
                device_id: device_id(LOCAL_DEVICE_ID),
            },
        });

        let input = build_sync_settings_input(&snapshot, &[]);

        assert_eq!(input.endpoint, "http://127.0.0.1:8384");
        assert!(input.has_api_key);
        assert_eq!(
            input.connection,
            SyncConnectionView::Connected {
                device_id: LOCAL_DEVICE_ID.to_owned(),
                version: "2.1.1".to_owned(),
            }
        );
        assert_eq!(input.libraries[0].state, LibrarySyncState::AwaitingRemoteAcceptance);
        assert_eq!(input.pending_folders[0].offered_by, REMOTE_DEVICE_ID);
        assert!(!format!("{input:?}").contains("secret"));
    }

    #[test]
    fn maps_incompatible_connection_and_error_notices_to_stable_ui_text() {
        let snapshot = snapshot(SyncConnectionState::Incompatible {
            found: Version::parse("3.0.0").expect("test version should parse"),
        });
        let notices = vec![
            SyncNotice::LibraryError {
                request_id: RequestId(9),
                message: "network error".to_owned(),
            },
            SyncNotice::RemoteEvent(SyncEvent::FullRefreshRequired),
        ];

        let input = build_sync_settings_input(&snapshot, &notices);

        assert_eq!(
            input.connection,
            SyncConnectionView::Incompatible { found: "3.0.0".to_owned() }
        );
        assert_eq!(input.notices[0].severity, SyncNoticeSeverity::Error);
        assert_eq!(input.notices[0].message, "同步操作失败：network error");
        assert_eq!(input.notices[1].severity, SyncNoticeSeverity::Warning);
        assert_eq!(input.notices[1].message, "同步状态需要刷新");
    }

    #[test]
    fn maps_local_validation_error_to_stable_error_notice() {
        let snapshot = snapshot(SyncConnectionState::NotConfigured);
        let input = build_sync_settings_input(
            &snapshot,
            &[SyncNotice::LocalError { message: "API Key 无效".to_owned() }],
        );

        assert_eq!(input.notices[0].severity, SyncNoticeSeverity::Error);
        assert_eq!(input.notices[0].message, "同步操作失败：API Key 无效");
    }
}
