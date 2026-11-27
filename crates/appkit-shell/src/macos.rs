//! Native macOS window and application delegate integration.

mod display_link;
mod open_documents;
mod qos;
mod titlebar;

pub(crate) use display_link::WindowDisplayLink;
pub use open_documents::install_open_document_handler;
pub use qos::LatencyCriticalActivity;
pub use titlebar::{align_traffic_lights, native_window};
