use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use notora_core::{DocumentKind, NavigationScope, NoteId, TagId, WorkspaceId};
use serde::{Deserialize, Serialize};

const SESSION_SCHEMA_VERSION: u32 = 1;
const LEGACY_SESSION_SCHEMA_VERSION: u32 = 0;
const MINIMUM_WINDOW_WIDTH_PX: f32 = 320.0;
const MINIMUM_WINDOW_HEIGHT_PX: f32 = 240.0;
const MAXIMUM_WINDOW_COORDINATE_PX: f32 = 32_768.0;
static SESSION_TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 可持久化的导航范围；不包含 runtime 或 catalog handle。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedNavigationScope {
    WorkspaceRoot,
    Directory { relative_path: PathBuf },
    Starred,
    Trash,
    Tag { tag_id: TagId },
    ExternalFiles,
}

impl From<&NavigationScope> for SavedNavigationScope {
    fn from(scope: &NavigationScope) -> Self {
        match scope {
            NavigationScope::Directory { relative_path } => {
                Self::Directory { relative_path: relative_path.clone() }
            }
            NavigationScope::Starred => Self::Starred,
            NavigationScope::Trash => Self::Trash,
            NavigationScope::Tag { tag_id } => Self::Tag { tag_id: *tag_id },
            NavigationScope::ExternalFiles => Self::ExternalFiles,
            NavigationScope::Search { .. } | NavigationScope::WorkspaceRoot => Self::WorkspaceRoot,
        }
    }
}

impl From<SavedNavigationScope> for NavigationScope {
    fn from(scope: SavedNavigationScope) -> Self {
        match scope {
            SavedNavigationScope::WorkspaceRoot => Self::WorkspaceRoot,
            SavedNavigationScope::Directory { relative_path } => Self::Directory { relative_path },
            SavedNavigationScope::Starred => Self::Starred,
            SavedNavigationScope::Trash => Self::Trash,
            SavedNavigationScope::Tag { tag_id } => Self::Tag { tag_id },
            SavedNavigationScope::ExternalFiles => Self::ExternalFiles,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedDocument {
    Note { note_id: NoteId },
    ExternalPath { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowGeometry {
    pub x_px: f32,
    pub y_px: f32,
    pub width_px: f32,
    pub height_px: f32,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self { x_px: 80.0, y_px: 80.0, width_px: 1_200.0, height_px: 800.0 }
    }
}

impl WindowGeometry {
    pub fn sanitized(self) -> Self {
        let default = Self::default();
        Self {
            x_px: finite_coordinate(self.x_px).unwrap_or(default.x_px),
            y_px: finite_coordinate(self.y_px).unwrap_or(default.y_px),
            width_px: finite_size(self.width_px, MINIMUM_WINDOW_WIDTH_PX, default.width_px),
            height_px: finite_size(self.height_px, MINIMUM_WINDOW_HEIGHT_PX, default.height_px),
        }
    }
}

/// session 只保存恢复所需的位置与选择；从不保存正文、SQLite connection 或 TabId。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProductSession {
    pub schema_version: u32,
    pub workspace_root: Option<PathBuf>,
    pub workspace_id: Option<WorkspaceId>,
    pub external_paths: Vec<PathBuf>,
    pub last_navigation_scope: SavedNavigationScope,
    pub expanded_directories: Vec<PathBuf>,
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
    pub last_document: Option<SavedDocument>,
    pub window_geometry: WindowGeometry,
}

impl Default for ProductSession {
    fn default() -> Self {
        Self {
            schema_version: SESSION_SCHEMA_VERSION,
            workspace_root: None,
            workspace_id: None,
            external_paths: Vec::new(),
            last_navigation_scope: SavedNavigationScope::WorkspaceRoot,
            expanded_directories: Vec::new(),
            navigation_width_logical: 220.0,
            card_list_width_logical: 340.0,
            last_document: None,
            window_geometry: WindowGeometry::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedProductSession {
    pub session: ProductSession,
    pub diagnostic: Option<String>,
}

pub fn load_product_session(path: &Path) -> LoadedProductSession {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedProductSession { session: ProductSession::default(), diagnostic: None };
        }
        Err(error) => {
            return LoadedProductSession {
                session: ProductSession::default(),
                diagnostic: Some(format!("could not read session: {error}")),
            };
        }
    };
    match toml::from_str::<ProductSession>(&contents) {
        Ok(session) if session.schema_version == SESSION_SCHEMA_VERSION => {
            LoadedProductSession { session: normalize_session(session), diagnostic: None }
        }
        Ok(mut session) if session.schema_version == LEGACY_SESSION_SCHEMA_VERSION => {
            session.schema_version = SESSION_SCHEMA_VERSION;
            LoadedProductSession { session: normalize_session(session), diagnostic: None }
        }
        Ok(session) => LoadedProductSession {
            session: ProductSession::default(),
            diagnostic: Some(format!(
                "unsupported session schema version: {}",
                session.schema_version
            )),
        },
        Err(error) => LoadedProductSession {
            session: ProductSession::default(),
            diagnostic: Some(format!("could not parse session: {error}")),
        },
    }
}

pub fn save_product_session(path: &Path, session: &ProductSession) -> Result<(), SessionError> {
    let contents = toml::to_string_pretty(session).map_err(SessionError::Serialize)?;
    let parent =
        path.parent().ok_or_else(|| SessionError::MissingParent { path: path.to_path_buf() })?;
    fs::create_dir_all(parent)
        .map_err(|source| SessionError::Io { path: parent.to_path_buf(), source })?;
    let temporary_path = parent.join(format!(
        ".session.{}.{}.tmp",
        std::process::id(),
        SESSION_TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut temporary_guard = TemporarySessionPath::new(temporary_path.clone());
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|source| SessionError::Io { path: temporary_path.clone(), source })?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| SessionError::Io { path: temporary_path.clone(), source })?;
    fs::rename(&temporary_path, path)
        .map_err(|source| SessionError::Io { path: path.to_path_buf(), source })?;
    temporary_guard.keep();
    Ok(())
}

fn is_restorable_external_path(path: &Path) -> bool {
    path.is_file() && DocumentKind::from_path(path).is_some()
}

fn normalize_session(mut session: ProductSession) -> ProductSession {
    session.window_geometry = session.window_geometry.sanitized();
    session.navigation_width_logical = finite_logical_width(
        session.navigation_width_logical,
        crate::shell::layout::MINIMUM_NAVIGATION_WIDTH_LOGICAL,
        crate::shell::layout::MAXIMUM_NAVIGATION_WIDTH_LOGICAL,
        crate::shell::layout::DEFAULT_NAVIGATION_WIDTH_LOGICAL,
    );
    session.card_list_width_logical = finite_logical_width(
        session.card_list_width_logical,
        crate::shell::layout::MINIMUM_CARD_LIST_WIDTH_LOGICAL,
        crate::shell::layout::MAXIMUM_CARD_LIST_WIDTH_LOGICAL,
        crate::shell::layout::DEFAULT_CARD_LIST_WIDTH_LOGICAL,
    );
    session.external_paths.retain(|path| is_restorable_external_path(path));
    session
}

fn finite_coordinate(value: f32) -> Option<f32> {
    value
        .is_finite()
        .then_some(value.clamp(-MAXIMUM_WINDOW_COORDINATE_PX, MAXIMUM_WINDOW_COORDINATE_PX))
}

fn finite_size(value: f32, minimum: f32, fallback: f32) -> f32 {
    if value.is_finite() { value.max(minimum) } else { fallback }
}

fn finite_logical_width(value: f32, minimum: f32, maximum: f32, fallback: f32) -> f32 {
    if value.is_finite() { value.clamp(minimum, maximum) } else { fallback }
}

#[derive(Debug)]
pub enum SessionError {
    MissingParent { path: PathBuf },
    Serialize(toml::ser::Error),
    Io { path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingParent { path } => {
                write!(f, "session path has no parent: {}", path.display())
            }
            Self::Serialize(e) => write!(f, "could not serialize session: {e}"),
            Self::Io { path, source } => {
                write!(f, "session I/O failed for {}: {source}", path.display())
            }
        }
    }
}
impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            Self::MissingParent { .. } => None,
        }
    }
}

struct TemporarySessionPath {
    path: PathBuf,
    should_remove: bool,
}
impl TemporarySessionPath {
    fn new(path: PathBuf) -> Self {
        Self { path, should_remove: true }
    }
    fn keep(&mut self) {
        self.should_remove = false;
    }
}
impl Drop for TemporarySessionPath {
    fn drop(&mut self) {
        if self.should_remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ProductSession, SavedDocument, SavedNavigationScope, WindowGeometry, load_product_session,
        save_product_session,
    };

    #[test]
    fn session_round_trip_filters_missing_external_files_and_sanitizes_geometry() {
        let directory = tempfile::tempdir().expect("session test directory should exist");
        let external_path = directory.path().join("external.md");
        std::fs::write(&external_path, "# External").expect("fixture external file should write");
        let path = directory.path().join("session.toml");
        let note_id = notora_core::NoteId::generate();
        let session = ProductSession {
            external_paths: vec![external_path.clone(), directory.path().join("missing.md")],
            last_navigation_scope: SavedNavigationScope::Starred,
            last_document: Some(SavedDocument::Note { note_id }),
            expanded_directories: vec![PathBuf::from("plans"), PathBuf::from("plans/q3")],
            window_geometry: WindowGeometry {
                x_px: f32::NAN,
                y_px: 0.0,
                width_px: 1.0,
                height_px: f32::INFINITY,
            },
            ..ProductSession::default()
        };
        save_product_session(&path, &session).expect("session should save atomically");
        let loaded = load_product_session(&path);
        assert_eq!(loaded.diagnostic, None);
        assert_eq!(loaded.session.external_paths, vec![external_path]);
        assert_eq!(loaded.session.last_navigation_scope, SavedNavigationScope::Starred);
        assert_eq!(loaded.session.last_document, Some(SavedDocument::Note { note_id }));
        assert_eq!(
            loaded.session.expanded_directories,
            vec![PathBuf::from("plans"), PathBuf::from("plans/q3")]
        );
        assert_eq!(loaded.session.window_geometry.width_px, 320.0);
        assert_eq!(loaded.session.window_geometry.height_px, 800.0);
        assert_eq!(
            loaded.session.navigation_width_logical,
            crate::shell::layout::DEFAULT_NAVIGATION_WIDTH_LOGICAL
        );
        assert_eq!(
            loaded.session.card_list_width_logical,
            crate::shell::layout::DEFAULT_CARD_LIST_WIDTH_LOGICAL
        );
    }

    #[test]
    fn malformed_session_falls_back_without_preventing_startup() {
        let directory = tempfile::tempdir().expect("session test directory should exist");
        let path = directory.path().join("session.toml");
        std::fs::write(&path, "invalid = true").expect("fixture should write");
        assert!(load_product_session(&path).diagnostic.is_some());
    }

    #[test]
    fn legacy_session_version_is_migrated_and_widths_are_clamped() {
        let directory = tempfile::tempdir().expect("session test directory should exist");
        let path = directory.path().join("session.toml");
        std::fs::write(
            &path,
            "schema_version = 0\nnavigation_width_logical = 9999\ncard_list_width_logical = -1\n",
        )
        .expect("legacy fixture should write");

        let loaded = load_product_session(&path);
        assert_eq!(loaded.diagnostic, None);
        assert_eq!(loaded.session.schema_version, super::SESSION_SCHEMA_VERSION);
        assert_eq!(
            loaded.session.navigation_width_logical,
            crate::shell::layout::MAXIMUM_NAVIGATION_WIDTH_LOGICAL
        );
        assert_eq!(
            loaded.session.card_list_width_logical,
            crate::shell::layout::MINIMUM_CARD_LIST_WIDTH_LOGICAL
        );
    }
}
