//! edit-plus core: gap buffer, unicode measurement, simd, icu, fuzzy.
//!
//! The full TextBuffer (cursor, selection, history, render) is deferred
//! to stage 6. See `buffer/text_buffer.rs.deferred` for the original.

pub use document::{DocView, DocViewMut};

pub mod base64;
pub mod buffer;
#[allow(unused)]
mod cell;
pub mod disk_revision;
pub mod document;
pub mod file;
pub mod fuzzy;
pub mod hash;
pub mod helpers;
pub mod highlight;
pub mod icu;
pub mod json;
pub mod oklab;
pub mod path;
pub mod simd;
pub mod text;
pub mod types;
pub mod unicode;
