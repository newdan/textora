//! edit-plus core: gap buffer, unicode measurement, simd, icu, fuzzy.
//!
//! The full TextBuffer (cursor, selection, history, render) is deferred
//! to stage 6. See `buffer/text_buffer.rs.deferred` for the original.

pub use document::{DocView, DocViewMut};

pub mod buffer;
#[allow(unused)]
pub mod disk_revision;
pub mod document;
pub mod file;
pub mod helpers;
pub mod highlight;
pub mod simd;
pub mod text;
pub mod types;
pub mod unicode;
