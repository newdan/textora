//! Everything related to Unicode lives here.

mod cursor_nav;
mod tables;

pub use cursor_nav::*;
pub use tables::{
    ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done, ucd_grapheme_cluster_lookup,
};

/// Yield the character index and UTF-8 byte offset at the start of each extended grapheme.
pub fn grapheme_start_indices(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text.char_indices().enumerate().peekable();

    std::iter::from_fn(move || {
        let (char_index, (byte_offset, ch)) = chars.next()?;
        let mut state = 0;
        let mut prev_props = ucd_grapheme_cluster_lookup(ch);

        while let Some(&(_, (_, next_ch))) = chars.peek() {
            let next_props = ucd_grapheme_cluster_lookup(next_ch);
            state = ucd_grapheme_cluster_joins(state, prev_props, next_props);
            if ucd_grapheme_cluster_joins_done(state) {
                break;
            }
            prev_props = next_props;
            chars.next();
        }

        Some((char_index, byte_offset))
    })
}

/// Return the UTF-8 byte offsets of the grapheme starts followed by `text.len()`.
///
/// For empty input, the returned vector contains only the end sentinel `0`.
pub fn grapheme_byte_boundaries(text: &str) -> Vec<usize> {
    grapheme_start_indices(text)
        .map(|(_, byte_offset)| byte_offset)
        .chain(std::iter::once(text.len()))
        .collect()
}
