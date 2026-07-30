use stdext::unicode::Utf8Chars;

use super::tables::*;
use crate::document::ReadableDocument;
use crate::helpers::CoordType;
use crate::types::{ByteIndex, LogicalPoint, VisualPoint};

/// Stores a position inside a [`ReadableDocument`].
///
/// The cursor tracks both the absolute byte-offset,
/// as well as the position in terminal-related coordinates.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Offset in bytes within the buffer.
    pub offset: ByteIndex,
    /// Position in the buffer in lines (`.line`) and unichar offsets (`.unichar`).
    ///
    /// Line wrapping has NO influence on this.
    pub logical_pos: LogicalPoint,
    /// Position in the buffer in laid out rows (`.row`) and columns (`.column`).
    ///
    /// Line wrapping has an influence on this.
    pub visual_pos: VisualPoint,
    /// Horizontal position in visual columns.
    ///
    /// Identical to `visual_pos.column`. This is useful for calculating tab widths.
    pub column: CoordType,
}

/// Your entrypoint to navigating inside a [`ReadableDocument`].
#[derive(Clone)]
pub struct CursorNav<'doc, A: GraphemeAdvance = TerminalAdvance> {
    cursor: Cursor,
    tab_size: CoordType,
    buffer: &'doc dyn ReadableDocument,
    advance: A,
}

/// Trait for computing the visual advance of a grapheme cluster.
///
/// Implementations determine the coordinate system:
/// - `TerminalAdvance`: returns column widths (1 or 2)
/// - `PixelAdvance`: returns pixel-based advances (for GUI rendering)
pub trait GraphemeAdvance: Clone {
    /// Returns the visual advance for a grapheme cluster.
    /// `cluster` is the UTF-8 bytes of the grapheme cluster.
    fn advance(&self, cluster: &[u8]) -> CoordType;

    /// Returns the visual advance for a tab at the given column position.
    fn tab_advance(&self, current_column: CoordType, tab_size: CoordType) -> CoordType;

    /// Clamps the total accumulated width of a grapheme cluster.
    ///
    /// In terminal mode, the max width of a cell is 2 (for wide characters).
    /// Multi-codepoint clusters (e.g., emoji + skin tone) may accumulate
    /// individual character widths exceeding 2, so the total must be clamped.
    /// In pixel mode, no clamping is needed.
    fn clamp_cluster_width(&self, width: CoordType) -> CoordType;
}

/// Terminal-style advance: returns column widths (max 2 per grapheme).
#[derive(Clone)]
pub struct TerminalAdvance;

impl GraphemeAdvance for TerminalAdvance {
    #[inline]
    fn advance(&self, cluster: &[u8]) -> CoordType {
        // Extract Unicode properties from the first byte
        let first_char = if cluster.is_empty() {
            '\0'
        } else {
            // Decode first char from UTF-8 bytes
            let b = cluster[0];
            if b < 0x80 {
                b as char
            } else if b < 0xE0 {
                // 2-byte sequence
                let b2 = if cluster.len() > 1 { cluster[1] } else { 0 };
                char::from_u32(((b as u32 & 0x1F) << 6) | (b2 as u32 & 0x3F)).unwrap_or('\0')
            } else if b < 0xF0 {
                // 3-byte sequence
                let b2 = if cluster.len() > 1 { cluster[1] } else { 0 };
                let b3 = if cluster.len() > 2 { cluster[2] } else { 0 };
                char::from_u32(
                    ((b as u32 & 0x0F) << 12) | ((b2 as u32 & 0x3F) << 6) | (b3 as u32 & 0x3F),
                )
                .unwrap_or('\0')
            } else {
                // 4-byte sequence
                let b2 = if cluster.len() > 1 { cluster[1] } else { 0 };
                let b3 = if cluster.len() > 2 { cluster[2] } else { 0 };
                let b4 = if cluster.len() > 3 { cluster[3] } else { 0 };
                char::from_u32(
                    ((b as u32 & 0x07) << 18)
                        | ((b2 as u32 & 0x3F) << 12)
                        | ((b3 as u32 & 0x3F) << 6)
                        | (b4 as u32 & 0x3F),
                )
                .unwrap_or('\0')
            }
        };
        let props = ucd_grapheme_cluster_lookup(first_char);
        ucd_grapheme_cluster_character_width(props, 1) as CoordType
    }

    #[inline]
    fn tab_advance(&self, current_column: CoordType, tab_size: CoordType) -> CoordType {
        tab_size - (current_column % tab_size)
    }

    #[inline]
    fn clamp_cluster_width(&self, width: CoordType) -> CoordType {
        width.min(2)
    }
}

/// Pixel-based advance: returns pixel widths for GUI rendering.
///
/// Uses UCD character widths to determine pixel advances:
/// - Narrow (ASCII, etc.): em_width pixels (default 8)
/// - Wide (CJK): 2 * em_width pixels (default 16)
/// - Zero-width (combining marks): 0 pixels
/// - Tab: tab_size * em_width pixels
///   Pixel-based advance with real font metrics via a lookup closure.
///
/// The closure takes a grapheme cluster as `&str` and returns its pixel advance.
/// Typically backed by `Shaper::grapheme_advance` with LRU cache.
pub struct PixelAdvance {
    /// Lookup function for grapheme cluster pixel advances.
    pub lookup: std::sync::Arc<dyn Fn(&str) -> f32 + Send + Sync>,
    /// Fallback width for characters that fail lookup.
    pub fallback_width: CoordType,
}

impl Clone for PixelAdvance {
    fn clone(&self) -> Self {
        Self { lookup: self.lookup.clone(), fallback_width: self.fallback_width }
    }
}

impl Default for PixelAdvance {
    fn default() -> Self {
        // Default: use Unicode table × 8 (same as before, for backward compat)
        Self {
            lookup: std::sync::Arc::new(|cluster: &str| {
                let first_char = cluster.chars().next().unwrap_or('\0');
                let props = ucd_grapheme_cluster_lookup(first_char);
                ucd_grapheme_cluster_character_width(props, 1) as f32 * 8.0
            }),
            fallback_width: 8,
        }
    }
}

impl PixelAdvance {
    /// Create a PixelAdvance backed by a Shaper's grapheme_advance.
    pub fn with_shaper<F>(lookup_fn: F, fallback_width: CoordType) -> Self
    where
        F: Fn(&str) -> f32 + Send + Sync + 'static,
    {
        Self { lookup: std::sync::Arc::new(lookup_fn), fallback_width }
    }
}

impl GraphemeAdvance for PixelAdvance {
    #[inline]
    fn advance(&self, cluster: &[u8]) -> CoordType {
        let grapheme = std::str::from_utf8(cluster).unwrap_or("");
        (self.lookup)(grapheme) as CoordType
    }

    #[inline]
    fn tab_advance(&self, current_column: CoordType, tab_size: CoordType) -> CoordType {
        let tab_width = tab_size * self.fallback_width;
        tab_width - (current_column % tab_width)
    }

    #[inline]
    fn clamp_cluster_width(&self, width: CoordType) -> CoordType {
        // No clamping for pixel-based rendering; widths are exact.
        width
    }
}

impl<'doc> CursorNav<'doc, TerminalAdvance> {
    /// Creates a new [`CursorNav`] for the given document.
    pub fn new(buffer: &'doc dyn ReadableDocument) -> Self {
        Self { cursor: Default::default(), tab_size: 8, buffer, advance: TerminalAdvance }
    }
}

impl<'doc, A: GraphemeAdvance> CursorNav<'doc, A> {
    /// Creates a new [`CursorNav`] with a custom [`GraphemeAdvance`] implementation.
    pub fn with_advance(buffer: &'doc dyn ReadableDocument, advance: A) -> Self {
        Self { cursor: Default::default(), tab_size: 8, buffer, advance }
    }

    /// Sets the initial cursor to the given position.
    ///
    /// WARNING: While the code doesn't panic if the cursor is invalid,
    /// the results will obviously be complete garbage.
    pub fn with_cursor(mut self, cursor: Cursor) -> Self {
        self.cursor = cursor;
        self
    }

    /// Sets the tab size.
    ///
    /// Defaults to 8, because that's what a tab in terminals evaluates to.
    pub fn with_tab_size(mut self, tab_size: CoordType) -> Self {
        self.tab_size = tab_size.max(1);
        self
    }

    /// Navigates **forward** to the given absolute byte offset.
    ///
    /// # Returns
    ///
    /// The cursor position after the navigation.
    pub fn goto_byte(&mut self, offset: ByteIndex) -> Cursor {
        self.measure_forward(offset, LogicalPoint::MAX, VisualPoint::MAX)
    }

    /// Navigates **forward** to the given logical position.
    ///
    /// Logical positions are in lines and unichar offsets.
    ///
    /// # Returns
    ///
    /// The cursor position after the navigation.
    pub fn goto_logical(&mut self, logical_target: LogicalPoint) -> Cursor {
        self.measure_forward(ByteIndex::MAX, logical_target, VisualPoint::MAX)
    }

    /// Navigates **forward** to the given visual position.
    ///
    /// Visual positions are in laid out rows and columns.
    ///
    /// # Returns
    ///
    /// The cursor position after the navigation.
    pub fn goto_visual(&mut self, visual_target: VisualPoint) -> Cursor {
        self.measure_forward(ByteIndex::MAX, LogicalPoint::MAX, visual_target)
    }

    /// Returns the current cursor position.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn measure_forward(
        &mut self,
        offset_target: ByteIndex,
        logical_target: LogicalPoint,
        visual_target: VisualPoint,
    ) -> Cursor {
        if self.cursor.offset >= offset_target
            || self.cursor.logical_pos >= logical_target
            || self.cursor.visual_pos >= visual_target
        {
            return self.cursor;
        }

        let mut offset = self.cursor.offset.to_usize();
        let mut logical_pos_unichar = self.cursor.logical_pos.unichar;
        let mut logical_pos_line = self.cursor.logical_pos.line;
        let mut visual_pos_column = self.cursor.visual_pos.column;
        let mut visual_pos_row = self.cursor.visual_pos.row;
        let mut column = self.cursor.column;

        let mut logical_target_unichar =
            Self::calc_logical_target_x(logical_target, logical_pos_line);
        let mut visual_target_column = Self::calc_visual_target_x(visual_target, visual_pos_row);

        let mut chunk_iter = Utf8Chars::new(b"", 0);
        let mut chunk_range = offset..offset;
        let mut props_next_cluster = ucd_start_of_text_properties();

        loop {
            if offset >= offset_target.to_usize()
                || logical_pos_unichar >= logical_target_unichar
                || visual_pos_column >= visual_target_column
            {
                break;
            }

            let props_current_cluster = props_next_cluster;
            let mut props_last_char;
            let mut offset_next_cluster;
            let mut state = 0;
            let cluster_start = offset;

            loop {
                if !chunk_iter.has_next() {
                    chunk_iter = Utf8Chars::new(self.buffer.read_forward(chunk_range.end), 0);
                    chunk_range = chunk_range.end..chunk_range.end + chunk_iter.len();
                }

                props_last_char = props_next_cluster;
                offset_next_cluster = chunk_range.start + chunk_iter.offset();

                let ch = match chunk_iter.next() {
                    Some(ch) => ch,
                    None => break,
                };

                props_next_cluster = ucd_grapheme_cluster_lookup(ch);
                state = ucd_grapheme_cluster_joins(state, props_last_char, props_next_cluster);

                if ucd_grapheme_cluster_joins_done(state) {
                    break;
                }
            }

            let cluster_bytes = if cluster_start < offset_next_cluster {
                let chunk = self.buffer.read_forward(cluster_start);
                let len = offset_next_cluster - cluster_start;
                if chunk.len() >= len { &chunk[..len] } else { &chunk[..chunk.len().min(len)] }
            } else {
                b""
            };

            let mut width = if cluster_bytes.is_empty() {
                let w = ucd_grapheme_cluster_character_width(props_current_cluster, 1);
                w as CoordType
            } else {
                self.advance.advance(cluster_bytes)
            };

            if offset_next_cluster == offset {
                if chunk_iter.is_empty() {
                    break;
                }
                continue;
            }

            width = self.advance.clamp_cluster_width(width);

            // Hard wrap: Both the logical and visual position advance by one line.
            if props_last_char == ucd_linefeed_properties() {
                // Don't cross the newline if the target is on this line but we haven't reached it.
                if logical_pos_line >= logical_target.line || visual_pos_row >= visual_target.row {
                    break;
                }

                offset = offset_next_cluster;
                logical_pos_unichar = 0;
                logical_pos_line += 1;
                visual_pos_column = 0;
                visual_pos_row += 1;
                column = 0;

                logical_target_unichar =
                    Self::calc_logical_target_x(logical_target, logical_pos_line);
                visual_target_column = Self::calc_visual_target_x(visual_target, visual_pos_row);
                continue;
            }

            if visual_pos_column + width > visual_target_column {
                break;
            }

            if props_last_char == ucd_tab_properties() {
                unsafe { std::hint::assert_unchecked(self.tab_size >= 1) };
                width = self.advance.tab_advance(column, self.tab_size);
            }

            offset = offset_next_cluster;
            logical_pos_unichar += 1;
            visual_pos_column += width;
            column += width;
        }

        self.cursor.offset = ByteIndex(offset);
        self.cursor.logical_pos =
            LogicalPoint { unichar: logical_pos_unichar, line: logical_pos_line };
        self.cursor.visual_pos = VisualPoint { column: visual_pos_column, row: visual_pos_row };
        self.cursor.column = column;
        self.cursor
    }

    #[inline]
    fn calc_logical_target_x(target: LogicalPoint, pos_line: usize) -> usize {
        match pos_line.cmp(&target.line) {
            std::cmp::Ordering::Less => usize::MAX,
            std::cmp::Ordering::Equal => target.unichar,
            std::cmp::Ordering::Greater => 0,
        }
    }

    #[inline]
    fn calc_visual_target_x(target: VisualPoint, pos_row: usize) -> CoordType {
        match pos_row.cmp(&target.row) {
            std::cmp::Ordering::Less => CoordType::MAX,
            std::cmp::Ordering::Equal => target.column,
            std::cmp::Ordering::Greater => 0,
        }
    }
}

/// Returns an offset past a newline.
///
/// If `offset` is right in front of a newline,
/// this will return the offset past said newline.
/// Strips a trailing newline from the given text.
pub fn strip_newline(mut text: &[u8]) -> &[u8] {
    // Rust generates surprisingly tight assembly for this.
    if text.last() == Some(&b'\n') {
        text = &text[..text.len() - 1];
    }
    if text.last() == Some(&b'\r') {
        text = &text[..text.len() - 1];
    }
    text
}
#[cfg(test)]
mod test {
    use super::*;
    use crate::types::{ByteIndex, LogicalPoint, VisualPoint};

    struct ChunkedDoc<'a>(&'a [&'a [u8]]);

    impl ReadableDocument for ChunkedDoc<'_> {
        fn read_forward(&self, mut off: usize) -> &[u8] {
            for chunk in self.0 {
                if off < chunk.len() {
                    return &chunk[off..];
                }
                off -= chunk.len();
            }
            &[]
        }

        fn read_backward(&self, mut off: usize) -> &[u8] {
            for chunk in self.0.iter().rev() {
                if off < chunk.len() {
                    return &chunk[..chunk.len() - off];
                }
                off -= chunk.len();
            }
            &[]
        }
    }

    #[test]
    fn test_measure_forward_newline_start() {
        let cursor =
            CursorNav::new(&"foo\nbar".as_bytes()).goto_visual(VisualPoint { column: 0, row: 1 });
        assert_eq!(
            cursor,
            Cursor {
                offset: ByteIndex(4),
                logical_pos: LogicalPoint { unichar: 0, line: 1 },
                visual_pos: VisualPoint { column: 0, row: 1 },
                column: 0,
            }
        );
    }

    #[test]
    fn test_measure_forward_clipped_wide_char() {
        let cursor =
            CursorNav::new(&"a😶‍🌫️b".as_bytes()).goto_visual(VisualPoint { column: 2, row: 0 });
        assert_eq!(
            cursor,
            Cursor {
                offset: ByteIndex(1),
                logical_pos: LogicalPoint { unichar: 1, line: 0 },
                visual_pos: VisualPoint { column: 1, row: 0 },
                column: 1,
            }
        );
    }

    #[test]
    fn test_measure_forward_tabs() {
        let text = "a\tb\tc".as_bytes();
        let cursor =
            CursorNav::new(&text).with_tab_size(4).goto_visual(VisualPoint { column: 4, row: 0 });
        assert_eq!(
            cursor,
            Cursor {
                offset: ByteIndex(2),
                logical_pos: LogicalPoint { unichar: 2, line: 0 },
                visual_pos: VisualPoint { column: 4, row: 0 },
                column: 4,
            }
        );
    }

    #[test]
    fn test_measure_forward_chunk_boundaries() {
        let chunks = [
            "Hello".as_bytes(),
            "\u{1F469}\u{1F3FB}".as_bytes(), // 8 bytes, 2 columns
            "World".as_bytes(),
        ];
        let doc = ChunkedDoc(&chunks);
        let cursor = CursorNav::new(&doc).goto_visual(VisualPoint { column: 5 + 2 + 3, row: 0 });
        assert_eq!(cursor.offset, ByteIndex(5 + 8 + 3));
        assert_eq!(cursor.logical_pos, LogicalPoint { unichar: 5 + 1 + 3, line: 0 });
    }

    #[test]
    fn test_crlf() {
        let text = "a\r\nbcd\r\ne".as_bytes();
        let cursor =
            CursorNav::new(&text).goto_visual(VisualPoint { column: CoordType::MAX, row: 1 });
        assert_eq!(
            cursor,
            Cursor {
                offset: ByteIndex(6),
                logical_pos: LogicalPoint { unichar: 3, line: 1 },
                visual_pos: VisualPoint { column: 3, row: 1 },
                column: 3,
            }
        );
    }

    #[test]
    fn test_strip_newline() {
        assert_eq!(strip_newline(b"hello\n"), b"hello");
        assert_eq!(strip_newline(b"hello\r\n"), b"hello");
        assert_eq!(strip_newline(b"hello"), b"hello");
    }
    // PixelAdvance tests

    #[test]
    fn pixel_advance_basic() {
        let bytes = "abc".as_bytes();
        let advance = PixelAdvance::default();
        let cursor = CursorNav::with_advance(&bytes, advance)
            .goto_visual(VisualPoint { column: 24, row: 0 });
        assert_eq!(
            cursor,
            Cursor {
                offset: ByteIndex(3),
                logical_pos: LogicalPoint { unichar: 3, line: 0 },
                visual_pos: VisualPoint { column: 24, row: 0 },
                column: 24,
            }
        );
    }

    #[test]
    fn pixel_tab_stop_alignment() {
        let bytes = "a\tb".as_bytes();
        let advance = PixelAdvance::default();
        let mut cfg = CursorNav::with_advance(&bytes, advance).with_tab_size(4);

        let cursor = cfg.goto_visual(VisualPoint { column: 32, row: 0 });
        assert_eq!(
            cursor,
            Cursor {
                offset: ByteIndex(2),
                logical_pos: LogicalPoint { unichar: 2, line: 0 },
                visual_pos: VisualPoint { column: 32, row: 0 },
                column: 32,
            }
        );
    }

    #[test]
    fn pixel_advance_zero_width() {
        let text = format!("e{}", '\u{0301}');
        let bytes = text.as_bytes();
        let advance = PixelAdvance::default();
        let cursor =
            CursorNav::with_advance(&bytes, advance).goto_visual(VisualPoint { column: 8, row: 0 });
        assert_eq!(cursor.offset, ByteIndex(bytes.len()));
        assert_eq!(cursor.visual_pos.column, 8);
    }

    #[test]
    fn pixel_advance_extreme_long_line() {
        let text = "a".repeat(1000);
        let bytes = text.as_bytes();
        let advance = PixelAdvance::default();
        let cursor = CursorNav::with_advance(&bytes, advance)
            .goto_visual(VisualPoint { column: 8000, row: 0 });
        assert_eq!(cursor.offset, ByteIndex(1000));
        assert_eq!(cursor.visual_pos, VisualPoint { column: 8000, row: 0 });
        assert_eq!(cursor.column, 8000);
    }
}
