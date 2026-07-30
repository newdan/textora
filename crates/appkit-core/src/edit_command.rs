/// Editor commands derived from keyboard input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditCommand {
    // ── Cursor movement ──
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveToLineStart,
    MoveToLineEnd,
    MoveToDocStart,
    MoveToDocEnd,
    PageUp,
    PageDown,

    // ── Editing ──
    InsertChar(String),
    InsertText(String),
    InsertNewline,
    Backspace,
    DeleteForward,
    DeleteRange(std::ops::Range<usize>),
    /// 原子替换：删除 range，然后在 range.start 处插入 text。
    /// 用于 WYSIWYG augment 一次性 outcome（方案 2026-07-06 阶段 3b）。
    /// 与 `DeleteRange + InsertText` 组合的差别：**只产生一份 outcome**
    /// （dirty_lines / new_line_count 合并计算），selection anchor 也
    /// 只在开始时展开一次，避免中间态被 sync_plugin_state 观察到。
    ReplaceRange {
        range: std::ops::Range<usize>,
        text: String,
    },

    // ── Clipboard (basic — full impl in stage 7) ──
    Cut,
    Copy,
    Paste,

    // ── Undo/Redo ──
    Undo,
    Redo,

    // ── Selection ──
    SelectAll,

    // ── Selection extension (Shift+Arrow) ──
    ExtendLeft,
    ExtendRight,
    ExtendUp,
    ExtendDown,
    ExtendWordLeft,
    ExtendWordRight,
    ExtendToLineStart,
    ExtendToLineEnd,
    ExtendToDocStart,
    ExtendToDocEnd,

    // ── File IO ──
    Save,
    SaveAs,
    OpenFile,
    OpenFolder,

    // ── Tab management ──
    NewTab,
    CloseTab,
    ToggleSidebarPin,
    ToggleView,
    ToggleToc,
    ReopenTab,
    NextTab,
    PrevTab,
    SwitchTab(usize),

    // ── Navigation history ──
    NavigateBack,
    NavigateForward,

    // ── Chapter navigation ──
    NextChapter,
    PrevChapter,

    // ── Misc ──
    Tab,
    Escape,

    // ── Search ──
    Find,
    FindReplace,
    FindNext,
    FindPrev,
}
