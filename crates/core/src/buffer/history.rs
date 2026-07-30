use crate::helpers::CoordType;
use crate::types::LogicalPoint;

/// Stores statistics about the whole document.
#[derive(Copy, Clone)]
pub struct TextBufferStatistics {
    pub logical_lines: CoordType,
    pub visual_lines: CoordType,
}

/// Stores the active text selection anchors.
///
/// The two points are not sorted. Instead, `beg` refers to where the selection
/// started being made and `end` refers to the currently being updated position.
#[derive(Copy, Clone)]
pub(crate) struct TextBufferSelection {
    pub beg: LogicalPoint,
    pub end: LogicalPoint,
}

/// In order to group actions into a single undo step,
/// we need to know the type of action that was performed.
/// This stores the action type.
#[derive(Copy, Clone, Eq, PartialEq)]
pub(crate) enum HistoryType {
    Other,
    Write,
    Delete,
}

/// An undo/redo entry.
pub(crate) struct HistoryEntry {
    /// cursor position before the change was made.
    pub cursor_before: LogicalPoint,
    /// selection before the change was made.
    pub selection_before: Option<TextBufferSelection>,
    /// stats before the change was made.
    pub stats_before: TextBufferStatistics,
    /// generation before the change was made.
    ///
    /// **NOTE:** Entries with the same generation are grouped together.
    pub generation_before: u32,
    /// Logical cursor position where the change took place.
    /// The position is at the start of the changed range.
    pub cursor: LogicalPoint,
    /// Text that was deleted from the buffer.
    pub deleted: Vec<u8>,
    /// Text that was added to the buffer.
    pub added: Vec<u8>,
}

/// Undo/redo grouping works by recording a set of "overrides",
/// which are then applied in `TextBuffer::edit_begin()`.
/// This allows us to create a group of edits that all share a
/// common `generation_before` and can be undone/redone together.
/// This struct stores those overrides.
pub(crate) struct ActiveEditGroupInfo {
    /// cursor position before the change was made.
    pub cursor_before: LogicalPoint,
    /// selection before the change was made.
    pub selection_before: Option<TextBufferSelection>,
    /// stats before the change was made.
    pub stats_before: TextBufferStatistics,
    /// generation before the change was made.
    ///
    /// **NOTE:** Entries with the same generation are grouped together.
    pub generation_before: u32,
}
