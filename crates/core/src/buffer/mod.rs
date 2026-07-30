//! Buffer module: gap buffer, navigation, and text buffer.

pub mod edit;
mod gap_buffer;
pub mod history;
pub mod io;
mod navigation;
pub mod search;
pub mod selection;
pub mod simd_search;
pub mod text_buffer;

pub use gap_buffer::GapBuffer;
pub use navigation::{word_backward, word_forward, word_select};
pub use text_buffer::TextBuffer;
