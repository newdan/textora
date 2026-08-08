//! Headless domain and persistence primitives for notora.
//!
//! This crate deliberately does not depend on UI, windowing, rendering, editor-runtime, or
//! Markdown-plugin crates. Product code is responsible for mapping these domain values to UI
//! input and editor sessions. Shared editor crates must likewise remain unaware of notora domain
//! types.

#![forbid(unsafe_code)]

pub mod backup;
pub mod catalog;
pub mod domain;
pub mod file_monitor;
pub mod note_command;
pub mod reconciliation;
pub mod scan;
pub mod summary_parser;
pub mod trash;
pub mod workspace;

pub use backup::{
    BackupRetention, CatalogBackupError, create_catalog_backup, create_catalog_backup_from_path,
    latest_valid_catalog_backup, restore_catalog_backup,
};
pub use catalog::{
    Catalog, CatalogCard, CatalogCardCursor, CatalogCardPage, CatalogError, CatalogNavigationTree,
    CatalogNote, CatalogOpenOutcome, CatalogRecoveryError, TagWithActiveNoteCount, TrashEntry,
};
pub use domain::{
    DocumentIdentity, DocumentKind, DocumentOrigin, ExternalFileId, NavigationScope,
    NoteEditorMetadata, NoteEncryption, NoteId, NoteLifecycle, NoteSummary, TagId, TagSummary,
    TitleInitialization, WorkspaceId,
};
pub use file_monitor::{WorkspaceFileBatch, WorkspaceFileMonitor, WorkspaceFileMonitorError};
pub use note_command::{
    ConfiguredCreateNoteRequest, CreateNoteResult, NoteCommand, NoteCommandError,
    execute_note_command,
};
pub use reconciliation::{
    DiscoveredNote, ReconciliationChange, ReconciliationError, ReconciliationPlan,
    reconcile_catalog,
};
pub use scan::{ScanCompletion, ScanError, ScanFailure, scan_workspace, scan_workspace_paths};
pub use summary_parser::{
    DocumentTitleProjection, MAX_EXCERPT_GRAPHEMES, NoteTextSummary, document_title_projection,
    parse_note_text_summary, replace_document_title,
};
pub use trash::{
    TrashError, empty_trash, move_to_trash, permanently_delete_trashed_note, restore_from_trash,
    restore_from_trash_with_renamed_path,
};
pub use workspace::{
    WORKSPACE_METADATA_DIRECTORY_NAME, Workspace, WorkspaceDescriptor, WorkspaceError,
    WorkspaceManifest,
};
