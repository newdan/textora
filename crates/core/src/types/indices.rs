use std::ops::{Add, AddAssign, Sub, SubAssign};

use crate::helpers::{CoordType, Point};

/// Byte offset within a gap buffer.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteIndex(pub usize);

impl ByteIndex {
    pub const ZERO: Self = ByteIndex(0);
    pub const MAX: Self = ByteIndex(usize::MAX);

    #[inline]
    pub fn to_usize(self) -> usize {
        self.0
    }

    #[inline]
    pub fn saturating_add_signed(self, rhs: isize) -> ByteIndex {
        ByteIndex(self.0.saturating_add_signed(rhs))
    }
}

impl Add<usize> for ByteIndex {
    type Output = ByteIndex;
    #[inline]
    fn add(self, rhs: usize) -> ByteIndex {
        ByteIndex(self.0 + rhs)
    }
}

impl AddAssign<usize> for ByteIndex {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl Sub<usize> for ByteIndex {
    type Output = ByteIndex;
    #[inline]
    fn sub(self, rhs: usize) -> ByteIndex {
        ByteIndex(self.0.saturating_sub(rhs))
    }
}

impl SubAssign<usize> for ByteIndex {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 = self.0.saturating_sub(rhs);
    }
}

impl Sub for ByteIndex {
    type Output = usize;
    #[inline]
    fn sub(self, rhs: ByteIndex) -> usize {
        self.0.saturating_sub(rhs.0)
    }
}

impl From<usize> for ByteIndex {
    #[inline]
    fn from(v: usize) -> Self {
        ByteIndex(v)
    }
}

impl From<ByteIndex> for usize {
    #[inline]
    fn from(v: ByteIndex) -> Self {
        v.0
    }
}

/// Document-wide UAX #29 grapheme offset, excluding line endings.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphemeOffset(pub usize);

impl GraphemeOffset {
    pub const ZERO: Self = GraphemeOffset(0);

    #[inline]
    pub fn to_usize(self) -> usize {
        self.0
    }

    #[inline]
    pub fn saturating_add_signed(self, rhs: isize) -> GraphemeOffset {
        GraphemeOffset(self.0.saturating_add_signed(rhs))
    }
}

impl Add<usize> for GraphemeOffset {
    type Output = GraphemeOffset;
    #[inline]
    fn add(self, rhs: usize) -> GraphemeOffset {
        GraphemeOffset(self.0 + rhs)
    }
}

impl AddAssign<usize> for GraphemeOffset {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl Sub<usize> for GraphemeOffset {
    type Output = GraphemeOffset;
    #[inline]
    fn sub(self, rhs: usize) -> GraphemeOffset {
        GraphemeOffset(self.0.saturating_sub(rhs))
    }
}

impl SubAssign<usize> for GraphemeOffset {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 = self.0.saturating_sub(rhs);
    }
}

impl Sub for GraphemeOffset {
    type Output = usize;
    #[inline]
    fn sub(self, rhs: GraphemeOffset) -> usize {
        self.0.saturating_sub(rhs.0)
    }
}

impl From<usize> for GraphemeOffset {
    #[inline]
    fn from(v: usize) -> Self {
        GraphemeOffset(v)
    }
}

impl From<GraphemeOffset> for usize {
    #[inline]
    fn from(v: GraphemeOffset) -> Self {
        v.0
    }
}

/// Logical cursor position — (line number, grapheme cluster index).
///
/// Line wrapping has NO influence on this coordinate.
/// Field ordering: line first (y-axis), then grapheme_column (x-axis).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogicalPoint {
    pub line: usize,
    pub grapheme_column: usize,
}

impl LogicalPoint {
    pub const ZERO: Self = LogicalPoint { line: 0, grapheme_column: 0 };
    pub const MAX: Self = LogicalPoint { line: usize::MAX, grapheme_column: usize::MAX };
}

impl PartialOrd for LogicalPoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogicalPoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.line.cmp(&other.line).then(self.grapheme_column.cmp(&other.grapheme_column))
    }
}

impl From<Point> for LogicalPoint {
    fn from(p: Point) -> Self {
        LogicalPoint { line: p.y.max(0) as usize, grapheme_column: p.x.max(0) as usize }
    }
}

/// Visual cursor position — (visual row, column width accumulator).
///
/// Line wrapping DOES influence this coordinate.
/// Field ordering: row first (y-axis), then column (x-axis).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisualPoint {
    pub row: usize,
    pub column: CoordType,
}

impl VisualPoint {
    pub const ZERO: Self = VisualPoint { row: 0, column: 0 };
    pub const MAX: Self = VisualPoint { row: usize::MAX, column: CoordType::MAX };
}

impl PartialOrd for VisualPoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VisualPoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.row.cmp(&other.row).then(self.column.cmp(&other.column))
    }
}

impl From<Point> for VisualPoint {
    fn from(p: Point) -> Self {
        VisualPoint { row: p.y.max(0) as usize, column: p.x }
    }
}

/// Resolves a document-level `GraphemeOffset` to a `(line, grapheme_column)` pair.
///
/// This trait decouples `TextBuffer` (in `crates/core`) from concrete line-index
/// types in higher-level crates (e.g., `LineIndex` in `crates/appkit-core`), allowing
/// `cursor_move_to_grapheme_offset` to accept any implementation.
pub trait GraphemeLineLookup {
    fn line_at_grapheme_offset(&self, offset: GraphemeOffset) -> (usize, usize);
}
