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

/// How a `replace_range` edit participates in undo history coalescing.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EditHistoryKind {
    /// Always forms its own undo entry and breaks any ongoing coalescing
    /// (plugin-augmented operations, find-and-replace, ...).
    Standalone,
    /// Text insertion that coalesces with an immediately adjacent insertion,
    /// matching source-mode continuous-typing undo granularity.
    Insert,
    /// Text deletion that coalesces with an immediately adjacent deletion,
    /// matching source-mode continuous-backspace undo granularity.
    Delete,
}

impl From<EditHistoryKind> for HistoryType {
    fn from(kind: EditHistoryKind) -> Self {
        match kind {
            EditHistoryKind::Standalone => HistoryType::Other,
            EditHistoryKind::Insert => HistoryType::Write,
            EditHistoryKind::Delete => HistoryType::Delete,
        }
    }
}

/// Tracks where the last coalescible edit joined the buffer, so a continuation
/// edit of the same kind can extend the same undo entry. Unlike
/// `last_history_type`, this survives caret-sync `set_cursor` calls and is
/// validated by byte adjacency, so unrelated cursor movement cannot corrupt a
/// merged entry.
#[derive(Copy, Clone, Eq, PartialEq)]
pub(crate) struct EditMergeAnchor {
    pub kind: EditHistoryKind,
    pub offset: usize,
}

impl EditMergeAnchor {
    /// The anchor left behind by an edit of `kind`, if follow-up edits may join it.
    pub(crate) fn after_edit(
        kind: EditHistoryKind,
        range: &std::ops::Range<usize>,
        replacement: &[u8],
    ) -> Option<Self> {
        match kind {
            EditHistoryKind::Insert => Some(Self { kind, offset: range.start + replacement.len() }),
            EditHistoryKind::Delete if replacement.is_empty() => {
                Some(Self { kind, offset: range.start })
            }
            _ => None,
        }
    }

    /// Whether an edit of `kind` continues the run this anchor tracks.
    pub(crate) fn continues(
        &self,
        kind: EditHistoryKind,
        range: &std::ops::Range<usize>,
        replacement: &[u8],
    ) -> bool {
        if self.kind != kind {
            return false;
        }
        match kind {
            EditHistoryKind::Insert => range.is_empty() && range.start == self.offset,
            EditHistoryKind::Delete => {
                replacement.is_empty() && (range.start == self.offset || range.end == self.offset)
            }
            EditHistoryKind::Standalone => false,
        }
    }
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
