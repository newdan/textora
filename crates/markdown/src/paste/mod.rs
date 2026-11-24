pub mod model;

#[cfg(feature = "rich-markdown")]
mod html;
#[cfg(feature = "rich-markdown")]
mod html_dom;
#[cfg(feature = "rich-markdown")]
mod html_entities;
mod rtf;
mod selection;
mod writer;

pub use model::*;
pub use selection::prepare_paste;
