//! Everything related to Unicode lives here.

mod cursor_nav;
mod tables;

pub use cursor_nav::*;
pub use tables::{
    ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done, ucd_grapheme_cluster_lookup,
};
