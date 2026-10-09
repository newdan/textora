pub mod model;

#[cfg(feature = "rich-markdown")]
mod html;
mod rtf;
mod selection;
mod writer;

pub use model::*;
pub use selection::prepare_paste;
