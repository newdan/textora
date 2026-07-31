//! Headless domain and persistence primitives for notora.
//!
//! This crate deliberately does not depend on UI, windowing, rendering, editor-runtime, or
//! Markdown-plugin crates. Product code is responsible for mapping these domain values to UI
//! input and editor sessions. Shared editor crates must likewise remain unaware of notora domain
//! types.

#![forbid(unsafe_code)]

pub mod domain;
pub mod summary_parser;
pub mod workspace;

pub use domain::{
    DocumentIdentity, DocumentKind, DocumentOrigin, ExternalFileId, NavigationScope, NoteId,
    NoteLifecycle, NoteSummary, TagId, TagSummary, WorkspaceId,
};
pub use summary_parser::{MAX_EXCERPT_GRAPHEMES, NoteTextSummary, parse_note_text_summary};
pub use workspace::{Workspace, WorkspaceDescriptor, WorkspaceError, WorkspaceManifest};
