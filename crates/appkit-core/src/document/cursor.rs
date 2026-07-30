use core::types::ByteIndex;

/// Pure cursor and selection state that belongs to the headless document
/// model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorState {
    /// Current cursor byte offset in the document buffer.
    pub offset: ByteIndex,
    /// Selection anchor byte offset, if a selection is active.
    pub selection_anchor: Option<usize>,
    /// Whether the previous command was "move to line start".
    pub last_command_was_home: bool,
    /// Whether the previous command was "move to line end".
    pub last_command_was_end: bool,
    /// Cached `(byte_offset, line_index)` pair for line-navigation helpers.
    pub cached_line: Option<(ByteIndex, usize)>,
    /// Snapshot cursor byte offset captured for lazy workspace restore.
    pub snapshot_offset: Option<usize>,
    /// Snapshot selection anchor captured for lazy workspace restore.
    pub snapshot_selection_anchor: Option<Option<usize>>,
}

impl CursorState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::CursorState;
    use core::types::ByteIndex;

    #[test]
    fn default_cursor_state_starts_at_zero_without_selection() {
        let state = CursorState::default();

        assert_eq!(state.offset, ByteIndex::ZERO);
        assert_eq!(state.selection_anchor, None);
        assert!(!state.last_command_was_home);
        assert!(!state.last_command_was_end);
        assert_eq!(state.cached_line, None);
        assert_eq!(state.snapshot_offset, None);
        assert_eq!(state.snapshot_selection_anchor, None);
    }

    #[test]
    fn new_cursor_state_matches_default() {
        assert_eq!(CursorState::new(), CursorState::default());
    }
}
