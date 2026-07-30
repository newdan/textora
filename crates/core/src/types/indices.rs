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

/// Unichar offset within a logical line (UAX #29 grapheme cluster index).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UniCharOffset(pub usize);

impl UniCharOffset {
    pub const ZERO: Self = UniCharOffset(0);

    #[inline]
    pub fn to_usize(self) -> usize {
        self.0
    }

    #[inline]
    pub fn saturating_add_signed(self, rhs: isize) -> UniCharOffset {
        UniCharOffset(self.0.saturating_add_signed(rhs))
    }
}

impl Add<usize> for UniCharOffset {
    type Output = UniCharOffset;
    #[inline]
    fn add(self, rhs: usize) -> UniCharOffset {
        UniCharOffset(self.0 + rhs)
    }
}

impl AddAssign<usize> for UniCharOffset {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl Sub<usize> for UniCharOffset {
    type Output = UniCharOffset;
    #[inline]
    fn sub(self, rhs: usize) -> UniCharOffset {
        UniCharOffset(self.0.saturating_sub(rhs))
    }
}

impl SubAssign<usize> for UniCharOffset {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 = self.0.saturating_sub(rhs);
    }
}

impl Sub for UniCharOffset {
    type Output = usize;
    #[inline]
    fn sub(self, rhs: UniCharOffset) -> usize {
        self.0.saturating_sub(rhs.0)
    }
}

impl From<usize> for UniCharOffset {
    #[inline]
    fn from(v: usize) -> Self {
        UniCharOffset(v)
    }
}

impl From<UniCharOffset> for usize {
    #[inline]
    fn from(v: UniCharOffset) -> Self {
        v.0
    }
}

/// Logical cursor position — (line number, grapheme cluster index).
///
/// Line wrapping has NO influence on this coordinate.
/// Field ordering: line first (y-axis), then unichar (x-axis).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogicalPoint {
    pub line: usize,
    pub unichar: usize,
}

impl LogicalPoint {
    pub const ZERO: Self = LogicalPoint { line: 0, unichar: 0 };
    pub const MAX: Self = LogicalPoint { line: usize::MAX, unichar: usize::MAX };
}

impl PartialOrd for LogicalPoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogicalPoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.line.cmp(&other.line).then(self.unichar.cmp(&other.unichar))
    }
}

impl From<Point> for LogicalPoint {
    fn from(p: Point) -> Self {
        LogicalPoint { line: p.y.max(0) as usize, unichar: p.x.max(0) as usize }
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

/// Resolves a document-level `UniCharOffset` to a `(line, line_local_unichar)` pair.
///
/// This trait decouples `TextBuffer` (in `crates/core`) from concrete line-index
/// types in higher-level crates (e.g., `LineIndex` in `crates/app`), allowing
/// `cursor_move_to_unichar` to accept any implementation.
pub trait UnicharLineLookup {
    fn line_at_unichar(&self, offset: UniCharOffset) -> (usize, usize);
}
