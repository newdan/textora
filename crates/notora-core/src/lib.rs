//! Headless domain and persistence primitives for notora.
//!
//! This crate deliberately does not depend on UI, windowing, rendering, editor-runtime, or
//! Markdown-plugin crates. Product code is responsible for mapping these domain values to UI
//! input and editor sessions. Shared editor crates must likewise remain unaware of notora domain
//! types.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod domain;
pub mod file_monitor;
pub mod note_command;
pub mod reconciliation;
pub mod scan;
pub mod summary_parser;
pub mod workspace;

pub use catalog::{Catalog, CatalogError, CatalogNote};
pub use domain::{
    DocumentIdentity, DocumentKind, DocumentOrigin, ExternalFileId, NavigationScope, NoteId,
    NoteLifecycle, NoteSummary, TagId, TagSummary, WorkspaceId,
};
pub use file_monitor::{WorkspaceFileBatch, WorkspaceFileMonitor, WorkspaceFileMonitorError};
pub use note_command::{
    CreateNoteRequest, CreateNoteResult, NoteCommand, NoteCommandError, execute_note_command,
};
pub use reconciliation::{
    DiscoveredNote, ReconciliationChange, ReconciliationError, ReconciliationPlan,
    reconcile_catalog,
};
pub use scan::{ScanCompletion, ScanError, ScanFailure, scan_workspace};
pub use summary_parser::{MAX_EXCERPT_GRAPHEMES, NoteTextSummary, parse_note_text_summary};
pub use workspace::{
    WORKSPACE_METADATA_DIRECTORY_NAME, Workspace, WorkspaceDescriptor, WorkspaceError,
    WorkspaceManifest,
};
