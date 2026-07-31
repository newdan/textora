//! Headless domain and persistence primitives for notora.
//!
//! This crate deliberately does not depend on UI, windowing, rendering, editor-runtime, or
//! Markdown-plugin crates. Product code is responsible for mapping these domain values to UI
//! input and editor sessions.

#![forbid(unsafe_code)]

pub mod domain;

pub use domain::{
    DocumentIdentity, DocumentKind, DocumentOrigin, ExternalFileId, NavigationScope, NoteId,
    NoteLifecycle, NoteSummary, TagId, TagSummary, WorkspaceId,
};
