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
pub mod file_name;
pub mod markdown_links;
pub mod note_command;
pub mod reconciliation;
pub mod scan;
pub mod summary_parser;
pub mod trash;
pub mod workspace;
pub mod workspace_directory;
pub mod workspace_tree;

pub use backup::{
    BackupRetention, CatalogBackupError, create_catalog_backup, create_catalog_backup_from_path,
    latest_valid_catalog_backup, restore_catalog_backup,
};
pub use catalog::{
    Catalog, CatalogCard, CatalogCardCursor, CatalogCardPage, CatalogError, CatalogNavigationTree,
    CatalogNote, CatalogOpenOutcome, CatalogRecoveryError, NotePathOperation,
    NotePathOperationKind, NotePathOperationState, TagWithActiveNoteCount, TrashEntry,
};
pub use domain::{
    DocumentIdentity, DocumentKind, DocumentOrigin, EXTERNAL_TEXT_FILE_EXTENSIONS, ExternalFileId,
    NavigationScope, NoteEditorMetadata, NoteEncryption, NoteFileNameBinding, NoteFileNameMetadata,
    NoteId, NoteLifecycle, NoteSummary, TagId, TagSummary, TitleInitialization, WorkspaceId,
};
pub use file_monitor::{
    WorkspaceFileBatch, WorkspaceFileChange, WorkspaceFileMonitor, WorkspaceFileMonitorError,
};
pub use file_name::{
    AllocatedTitleBoundFileName, DEFAULT_NOTE_TITLE, MAX_AUTOMATIC_NAME_DISAMBIGUATOR,
    MAX_NOTE_FILE_STEM_GRAPHEMES, allocate_title_bound_file_name, document_file_extension,
    file_name_collision_key, normalize_title_file_stem, title_bound_file_name,
};
pub use markdown_links::{
    MarkdownPathReference, MarkdownPathReferenceKind, extract_markdown_path_references,
};
pub use note_command::{
    ConfiguredCreateNoteRequest, CreateNoteResult, CreateNoteStorage, CreatedNoteAccess,
    NoteCommand, NoteCommandError, NoteCommandOutcome, NotePathRecoveryError,
    NotePathRecoveryReport, UpdateNoteTitleRequest, execute_note_command,
    recover_note_path_operations,
};
pub use reconciliation::{
    DiscoveredNote, ReconciliationChange, ReconciliationError, ReconciliationPlan,
    reconcile_catalog,
};
pub use scan::{
    ScanCompletion, ScanError, ScanFailure, scan_workspace, scan_workspace_file_batch,
    scan_workspace_paths,
};
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
pub use workspace_directory::{
    WorkspaceDirectoryCommand, WorkspaceDirectoryCommandError, WorkspaceDirectoryCommandResult,
    execute_workspace_directory_command, validate_workspace_directory_name,
};
pub use workspace_tree::{WorkspaceDirectoryScanError, scan_workspace_directories};
