//! Tab bar hit-test result shared by the state machine and widget.

/// Result of tab bar hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    NewTab,
    ScrollLeft,
    ScrollRight,
    /// Open the dropdown menu listing all open tabs
    Dropdown,
}
