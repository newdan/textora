use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use textora_sync::LoopbackEndpoint;

const METADATA_FILE_NAME: &str = "sync.toml";
const TEMP_FILE_PREFIX: &str = ".sync.toml.tmp";
const KEYCHAIN_SERVICE: &str = "com.textora.syncthing-api-key";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) const SYNC_KEYCHAIN_SERVICE: &str = KEYCHAIN_SERVICE;

#[derive(Debug)]
pub(crate) struct StoredSyncConnection {
    pub(crate) endpoint: LoopbackEndpoint,
    pub(crate) keychain_account: String,
}

pub(crate) struct SyncConnectionStore {
    config_dir: PathBuf,
}

#[derive(Debug)]
pub(crate) enum SyncConnectionStoreError {
    Io { operation: &'static str, source: std::io::Error },
    InvalidMetadata { message: String },
    Serialization { message: String },
}

impl fmt::Display for SyncConnectionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => write!(
                formatter,
                "failed to access Syncthing connection metadata during {operation}: {source}"
            ),
            Self::InvalidMetadata { message } => {
                write!(formatter, "invalid Syncthing connection metadata: {message}")
            }
            Self::Serialization { message } => {
                write!(formatter, "failed to serialize Syncthing connection metadata: {message}")
            }
        }
    }
}

impl std::error::Error for SyncConnectionStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidMetadata { .. } | Self::Serialization { .. } => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedSyncConnection {
    endpoint: String,
    keychain_account: String,
}

impl SyncConnectionStore {
    pub(crate) fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    pub(crate) fn default() -> Self {
        let home =
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        Self::new(home.join(".edit+"))
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.config_dir.join(METADATA_FILE_NAME)
    }

    pub(crate) fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    pub(crate) fn load(&self) -> Result<Option<StoredSyncConnection>, SyncConnectionStoreError> {
        let path = self.path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(SyncConnectionStoreError::Io { operation: "read", source });
            }
        };
        let persisted: PersistedSyncConnection = toml::from_str(&contents).map_err(|error| {
            SyncConnectionStoreError::InvalidMetadata { message: error.to_string() }
        })?;
        let endpoint = LoopbackEndpoint::parse(&persisted.endpoint).map_err(|error| {
            SyncConnectionStoreError::InvalidMetadata { message: error.to_string() }
        })?;
        validate_keychain_account(&persisted.keychain_account)?;
        Ok(Some(StoredSyncConnection { endpoint, keychain_account: persisted.keychain_account }))
    }

    pub(crate) fn save(
        &self,
        connection: &StoredSyncConnection,
    ) -> Result<(), SyncConnectionStoreError> {
        validate_keychain_account(&connection.keychain_account)?;
        let persisted = PersistedSyncConnection {
            endpoint: connection.endpoint.as_str().to_owned(),
            keychain_account: connection.keychain_account.clone(),
        };
        let contents = toml::to_string_pretty(&persisted).map_err(|error| {
            SyncConnectionStoreError::Serialization { message: error.to_string() }
        })?;
        fs::create_dir_all(&self.config_dir).map_err(|source| SyncConnectionStoreError::Io {
            operation: "create metadata directory",
            source,
        })?;

        let path = self.path();
        let temporary_path = temporary_path(&path);
        let write_result = write_temporary_file(&temporary_path, contents.as_bytes());
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }

        if let Err(source) = fs::rename(&temporary_path, &path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(SyncConnectionStoreError::Io { operation: "replace metadata", source });
        }
        Ok(())
    }

    pub(crate) fn remove(&self) -> Result<(), SyncConnectionStoreError> {
        match fs::remove_file(self.path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SyncConnectionStoreError::Io { operation: "remove", source }),
        }
    }
}

fn validate_keychain_account(account: &str) -> Result<(), SyncConnectionStoreError> {
    if account.trim().is_empty()
        || account.contains(['/', '\\'])
        || account.chars().any(char::is_control)
    {
        return Err(SyncConnectionStoreError::InvalidMetadata {
            message: "invalid keychain account".to_owned(),
        });
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let filename = format!("{TEMP_FILE_PREFIX}-{}-{sequence}", std::process::id());
    path.with_file_name(filename)
}

fn write_temporary_file(path: &Path, contents: &[u8]) -> Result<(), SyncConnectionStoreError> {
    let mut file =
        OpenOptions::new().write(true).create_new(true).open(path).map_err(|source| {
            SyncConnectionStoreError::Io { operation: "create temporary metadata", source }
        })?;
    file.write_all(contents).map_err(|source| SyncConnectionStoreError::Io {
        operation: "write temporary metadata",
        source,
    })?;
    file.sync_all().map_err(|source| SyncConnectionStoreError::Io {
        operation: "sync temporary metadata",
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use textora_sync::LoopbackEndpoint;

    use super::{StoredSyncConnection, SyncConnectionStore};

    fn store() -> (TempDir, SyncConnectionStore) {
        let directory = tempfile::tempdir().expect("temporary config directory should exist");
        let store = SyncConnectionStore::new(directory.path().to_owned());
        (directory, store)
    }

    fn connection() -> StoredSyncConnection {
        StoredSyncConnection {
            endpoint: LoopbackEndpoint::parse("http://127.0.0.1:8384")
                .expect("endpoint should parse"),
            keychain_account: "default-account".to_owned(),
        }
    }

    #[test]
    fn missing_metadata_is_unconfigured() {
        let (_directory, store) = store();
        assert!(store.load().expect("missing file should load").is_none());
    }

    #[test]
    fn save_and_load_persist_only_endpoint_and_keychain_account() {
        let (_directory, store) = store();
        store.save(&connection()).expect("metadata should save");

        let loaded = store.load().expect("metadata should load").expect("metadata should exist");
        assert_eq!(loaded.endpoint.as_str(), "http://127.0.0.1:8384");
        assert_eq!(loaded.keychain_account, "default-account");
        let serialized =
            fs::read_to_string(store.path()).expect("metadata file should be readable");
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn rejects_unknown_sensitive_fields_and_invalid_accounts() {
        let (_directory, store) = store();
        fs::write(
            store.path(),
            "endpoint = \"http://127.0.0.1:8384\"\nkeychain_account = \"account\"\napi_key = \"secret\"\n",
        )
        .expect("invalid metadata should be written");
        assert!(store.load().is_err());

        let invalid = StoredSyncConnection {
            endpoint: LoopbackEndpoint::parse("http://127.0.0.1:8384")
                .expect("endpoint should parse"),
            keychain_account: "account/with/path".to_owned(),
        };
        assert!(store.save(&invalid).is_err());
    }

    #[test]
    fn replacement_is_atomic_and_updates_only_after_success() {
        let (_directory, store) = store();
        store.save(&connection()).expect("initial metadata should save");
        let old_contents = fs::read_to_string(store.path()).expect("old metadata should exist");

        let replacement = StoredSyncConnection {
            endpoint: LoopbackEndpoint::parse("http://localhost:9999")
                .expect("endpoint should parse"),
            keychain_account: "replacement".to_owned(),
        };
        store.save(&replacement).expect("replacement should save atomically");
        let new_contents = fs::read_to_string(store.path()).expect("new metadata should exist");
        assert_ne!(old_contents, new_contents);
        assert_eq!(
            store
                .load()
                .expect("metadata should load")
                .expect("metadata should exist")
                .keychain_account,
            "replacement"
        );
    }
}
