use ui::core::widget::SensitiveText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncConnectionView {
    NotConfigured,
    Connecting,
    Connected { device_id: String, version: String },
    AuthenticationRequired,
    Incompatible { found: String },
    Unavailable { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySyncState {
    Pending,
    Scanning,
    Syncing,
    UpToDate,
    Paused,
    AwaitingRemoteAcceptance,
    ConfigurationMismatch,
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryView {
    pub name: String,
    pub root_display: String,
    pub state: LibrarySyncState,
    pub can_repair: bool,
    pub can_remove_mapping: bool,
    pub can_unregister: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFolderView {
    pub folder_id: String,
    pub offered_by: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncNoticeSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncNoticeView {
    pub severity: SyncNoticeSeverity,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncSettingsInput {
    pub endpoint: String,
    pub has_api_key: bool,
    pub connection: SyncConnectionView,
    pub libraries: Vec<LibraryView>,
    pub pending_folders: Vec<PendingFolderView>,
    pub notices: Vec<SyncNoticeView>,
}

impl Default for SyncSettingsInput {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            has_api_key: false,
            connection: SyncConnectionView::NotConfigured,
            libraries: Vec::new(),
            pending_folders: Vec::new(),
            notices: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncSettingsAction {
    TestConnection { endpoint: String, api_key: SensitiveText },
    ConfigureConnection { endpoint: String, api_key: SensitiveText },
    PublishLibrary { remote_device_id: String, remote_name: String, remote_addresses: Vec<String> },
    AcceptRemoteLibrary { pending_index: usize },
    ScanLibrary { library_index: usize },
    SetLibraryPaused { library_index: usize, paused: bool },
    RepairLibrary { library_index: usize },
    RemoveLibraryMapping { library_index: usize },
    UnregisterLibrary { library_index: usize },
}

#[cfg(test)]
mod tests {
    use ui::core::widget::SensitiveText;

    use super::SyncSettingsAction;

    #[test]
    fn sync_action_debug_redacts_api_key() {
        let action = SyncSettingsAction::ConfigureConnection {
            endpoint: "http://127.0.0.1:8384".to_owned(),
            api_key: SensitiveText::new("never-print-me".to_owned()),
        };
        let debug = format!("{action:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("never-print-me"));
    }

    #[test]
    fn sync_settings_input_has_no_api_key_value_field() {
        let source =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sync_settings_types.rs"));
        let prohibited_field = concat!("pub api_", "key:");
        assert!(!source.contains(prohibited_field));
    }
}
