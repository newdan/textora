use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use textora_sync::{DeviceId, FolderId, RemoteDeviceSpec};

const LIBRARY_METADATA_FILE_NAME: &str = "libraries.toml";
const LIBRARY_METADATA_TEMP_PREFIX: &str = ".libraries.toml.tmp";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum LibraryOrigin {
    Published,
    AcceptedRemote,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ProvisioningStage {
    RegisteringDevice,
    RegisteringFolder,
    ConfiguringIgnores,
    Scanning,
    AwaitingRemoteAcceptance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum LibraryRegistrationState {
    Provisioning { stage: ProvisioningStage },
    Active,
    Paused,
    ConfigurationMismatch,
    Error { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RegisteredRemoteDevice {
    pub(crate) device_id: DeviceId,
    pub(crate) name: String,
    pub(crate) addresses: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LibraryRecord {
    pub(crate) library_id: String,
    pub(crate) root: PathBuf,
    pub(crate) folder_id: FolderId,
    pub(crate) remote: RegisteredRemoteDevice,
    pub(crate) origin: LibraryOrigin,
    pub(crate) state: LibraryRegistrationState,
    pub(crate) device_created_by_textora: bool,
    pub(crate) folder_created_by_textora: bool,
}

#[derive(Debug)]
pub(crate) enum LibraryRegistryError {
    Io { operation: &'static str, source: std::io::Error },
    InvalidMetadata { message: String },
    RootMissing { path: PathBuf },
    RootNotDirectory { path: PathBuf },
    RootNotEmpty { path: PathBuf },
    NestedRoot { root: PathBuf, existing: PathBuf },
    DuplicateLibrary { root: PathBuf },
    InvalidLibraryId,
}

impl fmt::Display for LibraryRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => {
                write!(formatter, "library registry {operation} failed: {source}")
            }
            Self::InvalidMetadata { message } => {
                write!(formatter, "invalid library registry metadata: {message}")
            }
            Self::RootMissing { path } => {
                write!(formatter, "library root does not exist: {}", path.display())
            }
            Self::RootNotDirectory { path } => {
                write!(formatter, "library root is not a directory: {}", path.display())
            }
            Self::RootNotEmpty { path } => {
                write!(formatter, "remote library root is not empty: {}", path.display())
            }
            Self::NestedRoot { root, existing } => write!(
                formatter,
                "library root {} overlaps registered root {}",
                root.display(),
                existing.display()
            ),
            Self::DuplicateLibrary { root } => {
                write!(formatter, "library root is already registered: {}", root.display())
            }
            Self::InvalidLibraryId => formatter.write_str("invalid stable library ID"),
        }
    }
}

impl std::error::Error for LibraryRegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidMetadata { .. }
            | Self::RootMissing { .. }
            | Self::RootNotDirectory { .. }
            | Self::RootNotEmpty { .. }
            | Self::NestedRoot { .. }
            | Self::DuplicateLibrary { .. }
            | Self::InvalidLibraryId => None,
        }
    }
}

pub(crate) struct LibraryRegistry {
    config_dir: PathBuf,
    libraries: BTreeMap<String, LibraryRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedLibraryRegistry {
    libraries: Vec<PersistedLibraryRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedLibraryRecord {
    library_id: String,
    root: String,
    folder_id: String,
    remote_device_id: String,
    remote_name: String,
    remote_addresses: Vec<String>,
    origin: String,
    state: String,
    provisioning_stage: Option<String>,
    error_message: Option<String>,
    device_created_by_textora: bool,
    folder_created_by_textora: bool,
}

impl LibraryRegistry {
    pub(crate) fn new(config_dir: PathBuf) -> Self {
        Self { config_dir, libraries: BTreeMap::new() }
    }

    pub(crate) fn load(config_dir: PathBuf) -> Result<Self, LibraryRegistryError> {
        let path = config_dir.join(LIBRARY_METADATA_FILE_NAME);
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::new(config_dir));
            }
            Err(source) => {
                return Err(LibraryRegistryError::Io { operation: "read metadata", source });
            }
        };
        let persisted: PersistedLibraryRegistry = toml::from_str(&contents).map_err(|error| {
            LibraryRegistryError::InvalidMetadata { message: error.to_string() }
        })?;
        let libraries = persisted
            .libraries
            .into_iter()
            .map(|record| {
                deserialize_record(record).map(|record| (record.library_id.clone(), record))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(Self { config_dir, libraries })
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.config_dir.join(LIBRARY_METADATA_FILE_NAME)
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &LibraryRecord> {
        self.libraries.values()
    }

    pub(crate) fn get(&self, library_id: &str) -> Option<&LibraryRecord> {
        self.libraries.get(library_id)
    }

    pub(crate) fn get_mut(&mut self, library_id: &str) -> Option<&mut LibraryRecord> {
        self.libraries.get_mut(library_id)
    }

    pub(crate) fn register_published(
        &mut self,
        root: PathBuf,
        remote: &RemoteDeviceSpec,
    ) -> Result<LibraryRecord, LibraryRegistryError> {
        let root = canonical_library_root(&root)?;
        let folder_id = FolderId::new(stable_folder_id(&root))
            .map_err(|_| LibraryRegistryError::InvalidLibraryId)?;
        self.register(root, folder_id, remote, LibraryOrigin::Published)
    }

    pub(crate) fn register_accepted_remote(
        &mut self,
        root: PathBuf,
        folder_id: FolderId,
        remote: &RemoteDeviceSpec,
    ) -> Result<LibraryRecord, LibraryRegistryError> {
        let root = canonical_library_root(&root)?;
        self.register(root, folder_id, remote, LibraryOrigin::AcceptedRemote)
    }

    pub(crate) fn canonical_empty_root(
        &self,
        root: &Path,
    ) -> Result<PathBuf, LibraryRegistryError> {
        let root = canonical_library_root(root)?;
        let mut entries = fs::read_dir(&root).map_err(|source| LibraryRegistryError::Io {
            operation: "inspect remote library root",
            source,
        })?;
        if entries.next().is_some() {
            return Err(LibraryRegistryError::RootNotEmpty { path: root });
        }
        Ok(root)
    }

    pub(crate) fn register(
        &mut self,
        root: PathBuf,
        folder_id: FolderId,
        remote: &RemoteDeviceSpec,
        origin: LibraryOrigin,
    ) -> Result<LibraryRecord, LibraryRegistryError> {
        let root = canonical_library_root(&root)?;
        if let Some(existing) =
            self.libraries.values().find(|existing| roots_overlap(&root, &existing.root))
        {
            if existing.root == root {
                return Err(LibraryRegistryError::DuplicateLibrary { root });
            }
            return Err(LibraryRegistryError::NestedRoot { root, existing: existing.root.clone() });
        }
        let library_id = stable_library_id(&root);
        let record = LibraryRecord {
            library_id: library_id.clone(),
            root,
            folder_id,
            remote: RegisteredRemoteDevice {
                device_id: remote.device_id.clone(),
                name: remote.name.clone(),
                addresses: remote
                    .addresses
                    .iter()
                    .map(|address| address.as_str().to_owned())
                    .collect(),
            },
            origin,
            state: LibraryRegistrationState::Provisioning {
                stage: ProvisioningStage::RegisteringDevice,
            },
            device_created_by_textora: false,
            folder_created_by_textora: false,
        };
        self.libraries.insert(library_id, record.clone());
        Ok(record)
    }

    pub(crate) fn remove(&mut self, library_id: &str) -> Option<LibraryRecord> {
        self.libraries.remove(library_id)
    }

    pub(crate) fn owner_of(&self, path: &Path) -> Option<&LibraryRecord> {
        let normalized = lookup_path(path)?;
        self.libraries
            .values()
            .filter(|record| normalized.starts_with(&record.root))
            .max_by_key(|record| record.root.components().count())
    }

    pub(crate) fn save(&self) -> Result<(), LibraryRegistryError> {
        let persisted = PersistedLibraryRegistry {
            libraries: self
                .libraries
                .values()
                .map(serialize_record)
                .collect::<Result<Vec<_>, _>>()?,
        };
        let contents = toml::to_string_pretty(&persisted).map_err(|error| {
            LibraryRegistryError::InvalidMetadata { message: error.to_string() }
        })?;
        fs::create_dir_all(&self.config_dir).map_err(|source| LibraryRegistryError::Io {
            operation: "create metadata directory",
            source,
        })?;
        let temporary_path = temporary_path(&self.path());
        write_temporary_file(&temporary_path, contents.as_bytes())?;
        if let Err(source) = fs::rename(&temporary_path, self.path()) {
            let _ = fs::remove_file(&temporary_path);
            return Err(LibraryRegistryError::Io { operation: "replace metadata", source });
        }
        Ok(())
    }
}

fn serialize_record(
    record: &LibraryRecord,
) -> Result<PersistedLibraryRecord, LibraryRegistryError> {
    let root = record.root.to_str().ok_or_else(|| LibraryRegistryError::InvalidMetadata {
        message: "library root is not valid UTF-8".to_owned(),
    })?;
    let (state, provisioning_stage, error_message) = match &record.state {
        LibraryRegistrationState::Provisioning { stage } => {
            ("provisioning".to_owned(), Some(serialize_stage(stage)), None)
        }
        LibraryRegistrationState::Active => ("active".to_owned(), None, None),
        LibraryRegistrationState::Paused => ("paused".to_owned(), None, None),
        LibraryRegistrationState::ConfigurationMismatch => {
            ("configuration_mismatch".to_owned(), None, None)
        }
        LibraryRegistrationState::Error { message } => {
            ("error".to_owned(), None, Some(message.clone()))
        }
    };
    Ok(PersistedLibraryRecord {
        library_id: record.library_id.clone(),
        root: root.to_owned(),
        folder_id: record.folder_id.as_str().to_owned(),
        remote_device_id: record.remote.device_id.as_str().to_owned(),
        remote_name: record.remote.name.clone(),
        remote_addresses: record.remote.addresses.clone(),
        origin: serialize_origin(&record.origin),
        state,
        provisioning_stage,
        error_message,
        device_created_by_textora: record.device_created_by_textora,
        folder_created_by_textora: record.folder_created_by_textora,
    })
}

fn deserialize_record(
    record: PersistedLibraryRecord,
) -> Result<LibraryRecord, LibraryRegistryError> {
    let device_id = DeviceId::parse(record.remote_device_id).map_err(|_| {
        LibraryRegistryError::InvalidMetadata { message: "invalid remote device ID".to_owned() }
    })?;
    let folder_id = FolderId::new(record.folder_id).map_err(|_| {
        LibraryRegistryError::InvalidMetadata { message: "invalid folder ID".to_owned() }
    })?;
    if record.library_id.trim().is_empty() {
        return Err(LibraryRegistryError::InvalidMetadata {
            message: "empty library ID".to_owned(),
        });
    }
    let origin = deserialize_origin(&record.origin)?;
    let state = deserialize_state(
        &record.state,
        record.provisioning_stage.as_deref(),
        record.error_message,
    )?;
    Ok(LibraryRecord {
        library_id: record.library_id,
        root: PathBuf::from(record.root),
        folder_id,
        remote: RegisteredRemoteDevice {
            device_id,
            name: record.remote_name,
            addresses: record.remote_addresses,
        },
        origin,
        state,
        device_created_by_textora: record.device_created_by_textora,
        folder_created_by_textora: record.folder_created_by_textora,
    })
}

fn serialize_origin(origin: &LibraryOrigin) -> String {
    match origin {
        LibraryOrigin::Published => "published".to_owned(),
        LibraryOrigin::AcceptedRemote => "accepted_remote".to_owned(),
    }
}

fn deserialize_origin(origin: &str) -> Result<LibraryOrigin, LibraryRegistryError> {
    match origin {
        "published" => Ok(LibraryOrigin::Published),
        "accepted_remote" => Ok(LibraryOrigin::AcceptedRemote),
        _ => Err(LibraryRegistryError::InvalidMetadata {
            message: "invalid library origin".to_owned(),
        }),
    }
}

fn serialize_stage(stage: &ProvisioningStage) -> String {
    match stage {
        ProvisioningStage::RegisteringDevice => "registering_device",
        ProvisioningStage::RegisteringFolder => "registering_folder",
        ProvisioningStage::ConfiguringIgnores => "configuring_ignores",
        ProvisioningStage::Scanning => "scanning",
        ProvisioningStage::AwaitingRemoteAcceptance => "awaiting_remote_acceptance",
    }
    .to_owned()
}

fn deserialize_stage(stage: &str) -> Result<ProvisioningStage, LibraryRegistryError> {
    match stage {
        "registering_device" => Ok(ProvisioningStage::RegisteringDevice),
        "registering_folder" => Ok(ProvisioningStage::RegisteringFolder),
        "configuring_ignores" => Ok(ProvisioningStage::ConfiguringIgnores),
        "scanning" => Ok(ProvisioningStage::Scanning),
        "awaiting_remote_acceptance" => Ok(ProvisioningStage::AwaitingRemoteAcceptance),
        _ => Err(LibraryRegistryError::InvalidMetadata {
            message: "invalid provisioning stage".to_owned(),
        }),
    }
}

fn deserialize_state(
    state: &str,
    provisioning_stage: Option<&str>,
    error_message: Option<String>,
) -> Result<LibraryRegistrationState, LibraryRegistryError> {
    match state {
        "provisioning" => Ok(LibraryRegistrationState::Provisioning {
            stage: deserialize_stage(provisioning_stage.ok_or_else(|| {
                LibraryRegistryError::InvalidMetadata {
                    message: "provisioning state is missing its stage".to_owned(),
                }
            })?)?,
        }),
        "active" if provisioning_stage.is_none() && error_message.is_none() => {
            Ok(LibraryRegistrationState::Active)
        }
        "paused" if provisioning_stage.is_none() && error_message.is_none() => {
            Ok(LibraryRegistrationState::Paused)
        }
        "configuration_mismatch" if provisioning_stage.is_none() && error_message.is_none() => {
            Ok(LibraryRegistrationState::ConfigurationMismatch)
        }
        "error" if provisioning_stage.is_none() => Ok(LibraryRegistrationState::Error {
            message: error_message.ok_or_else(|| LibraryRegistryError::InvalidMetadata {
                message: "error state is missing its message".to_owned(),
            })?,
        }),
        _ => Err(LibraryRegistryError::InvalidMetadata {
            message: "invalid library registration state".to_owned(),
        }),
    }
}

fn canonical_library_root(path: &Path) -> Result<PathBuf, LibraryRegistryError> {
    let metadata = fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LibraryRegistryError::RootMissing { path: path.to_owned() }
        } else {
            LibraryRegistryError::Io { operation: "inspect library root", source: error }
        }
    })?;
    if !metadata.is_dir() {
        return Err(LibraryRegistryError::RootNotDirectory { path: path.to_owned() });
    }
    fs::canonicalize(path).map_err(|source| LibraryRegistryError::Io {
        operation: "canonicalize library root",
        source,
    })
}

fn lookup_path(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path).ok();
    }
    let mut missing_components = Vec::new();
    let mut current = path;
    while !current.exists() {
        missing_components.push(current.file_name()?.to_owned());
        current = current.parent()?;
    }
    let mut normalized = fs::canonicalize(current).ok()?;
    for component in missing_components.iter().rev() {
        normalized.push(component);
    }
    Some(normalized)
}

fn roots_overlap(first: &Path, second: &Path) -> bool {
    first == second || first.starts_with(second) || second.starts_with(first)
}

fn stable_library_id(root: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3_u64);
    }
    format!("library-{hash:016x}")
}

fn stable_folder_id(root: &Path) -> String {
    let library_id = stable_library_id(root);
    format!("textora-{}", &library_id["library-".len()..])
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!("{LIBRARY_METADATA_TEMP_PREFIX}-{}-{sequence}", std::process::id()))
}

fn write_temporary_file(path: &Path, contents: &[u8]) -> Result<(), LibraryRegistryError> {
    let mut file =
        OpenOptions::new().write(true).create_new(true).open(path).map_err(|source| {
            LibraryRegistryError::Io { operation: "create temporary metadata", source }
        })?;
    file.write_all(contents).map_err(|source| LibraryRegistryError::Io {
        operation: "write temporary metadata",
        source,
    })?;
    file.sync_all()
        .map_err(|source| LibraryRegistryError::Io { operation: "sync temporary metadata", source })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use textora_sync::{DeviceId, RemoteDeviceSpec, StaticSyncAddress};

    use super::{
        LibraryOrigin, LibraryRegistrationState, LibraryRegistry, LibraryRegistryError,
        ProvisioningStage,
    };

    const DEVICE_ID: &str = "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG";

    fn remote() -> RemoteDeviceSpec {
        RemoteDeviceSpec {
            device_id: DeviceId::parse(DEVICE_ID.to_owned()).expect("device ID should parse"),
            name: "Remote".to_owned(),
            addresses: vec![
                StaticSyncAddress::parse("tcp://sync.example.com:22000".to_owned())
                    .expect("address should parse"),
            ],
        }
    }

    #[test]
    fn registers_stable_mapping_and_round_trips_atomically() {
        let directory = tempfile::tempdir().expect("config directory should exist");
        let root = directory.path().join("notes");
        fs::create_dir(&root).expect("library root should exist");
        let mut registry = LibraryRegistry::new(directory.path().join("config"));
        let record =
            registry.register_published(root.clone(), &remote()).expect("library should register");
        assert!(record.library_id.starts_with("library-"));
        assert!(record.folder_id.as_str().starts_with("textora-"));
        assert_eq!(record.origin, LibraryOrigin::Published);
        assert!(matches!(
            record.state,
            LibraryRegistrationState::Provisioning { stage: ProvisioningStage::RegisteringDevice }
        ));
        registry.save().expect("registry should save");

        let loaded =
            LibraryRegistry::load(directory.path().join("config")).expect("registry should load");
        let loaded_record = loaded.get(&record.library_id).expect("record should exist");
        assert_eq!(loaded_record.root, fs::canonicalize(root).expect("root should canonicalize"));
        assert_eq!(loaded_record.remote.name, "Remote");
        let serialized = fs::read_to_string(loaded.path()).expect("metadata should be readable");
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn rejects_duplicate_and_nested_roots_with_component_boundaries() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let parent = directory.path().join("library");
        let child = parent.join("child");
        let sibling_prefix = directory.path().join("library-copy");
        fs::create_dir_all(&child).expect("nested roots should exist");
        fs::create_dir(&sibling_prefix).expect("sibling root should exist");
        let mut registry = LibraryRegistry::new(directory.path().join("config"));
        let first = registry
            .register_published(parent.clone(), &remote())
            .expect("first root should register");
        assert!(matches!(
            registry.register_published(parent, &remote()),
            Err(LibraryRegistryError::DuplicateLibrary { .. })
        ));
        assert!(matches!(
            registry.register_published(child, &remote()),
            Err(LibraryRegistryError::NestedRoot { .. })
        ));
        let sibling = registry
            .register_published(sibling_prefix.clone(), &remote())
            .expect("component-prefix sibling should register");
        assert_ne!(first.library_id, sibling.library_id);
    }

    #[test]
    fn owner_lookup_uses_canonical_paths_and_longest_component_prefix() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let first_root = directory.path().join("a");
        let second_root = directory.path().join("ab");
        fs::create_dir_all(first_root.join("docs")).expect("first root should exist");
        fs::create_dir_all(second_root.join("docs")).expect("second root should exist");
        let mut registry = LibraryRegistry::new(directory.path().join("config"));
        let first = registry
            .register_published(first_root.clone(), &remote())
            .expect("first root should register");
        let second = registry
            .register_published(second_root.clone(), &remote())
            .expect("second root should register");
        let owner = registry
            .owner_of(&first_root.join("docs/file.md"))
            .expect("first path should have an owner");
        assert_eq!(owner.library_id, first.library_id);
        let owner = registry
            .owner_of(&second_root.join("docs/file.md"))
            .expect("second path should have an owner");
        assert_eq!(owner.library_id, second.library_id);
        assert!(registry.owner_of(&PathBuf::from("/unrelated/path")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_alias_of_an_existing_root() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let root = directory.path().join("real");
        let alias = directory.path().join("alias");
        fs::create_dir(&root).expect("real root should exist");
        symlink(&root, &alias).expect("symlink should be created");
        let mut registry = LibraryRegistry::new(directory.path().join("config"));
        registry.register_published(root, &remote()).expect("real root should register");
        assert!(matches!(
            registry.register_published(alias, &remote()),
            Err(LibraryRegistryError::DuplicateLibrary { .. })
        ));
    }
}
