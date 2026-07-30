//! Headless application model and persistence for textora-based products.

#![forbid(unsafe_code)]

pub mod content_hash;
pub mod document;
pub mod edit;
pub mod edit_command;
pub mod external_document_change;
pub mod file_history;
pub mod file_safety;
pub mod line_index;
pub mod navigator;
pub mod persistence;
pub mod snapshot;
pub mod workspace;
