//! Native macOS window and application delegate integration.

mod open_documents;
mod titlebar;

pub use open_documents::install_open_document_handler;
pub use titlebar::{align_traffic_lights, native_window};
