//! Plugin infrastructure for view-mode rendering.
//!
//! Provides [`ViewPlugin`] trait for pluggable renderers,
//! [`PluginFactory`] for deferred creation, and [`PluginRegistry`] for dispatch.

use std::path::Path;

use core::buffer::EditHistoryKind;
use core::document::{DocView, DocViewMut};
use shaping::Shaper;

use crate::canvas::{CanvasPoint, CanvasViewportSnapshot};
use crate::core::geom::Rect;
use crate::core::paint::DrawList;
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Messages & Queries
// ---------------------------------------------------------------------------

/// Commands sent from the host to a plugin.
#[derive(Debug)]
pub enum PluginMessage {
    /// Scroll by a delta (pixels), clamped by viewport height.
    Scroll { delta: f32, viewport_h: f32 },
    /// Jump to the heading at the given index in the TOC.
    ScrollToHeading(usize),
    /// Jump to the N-th search match.
    ScrollToSearchMatch { query: String, match_case: bool, active_idx: usize },
    /// Advance to the next chapter marker.
    ScrollToNextChapter,
    /// Retreat to the previous chapter marker.
    ScrollToPrevChapter,
    /// Replace the cached source text (e.g. after an edit).
    UpdateSource { text: String, generation: u32 },
    /// Set the selection cursor position (flat_line_idx, grapheme_pos).
    SetSelCursor(Option<(usize, usize)>),
    /// Set the selection anchor position (flat_line_idx, grapheme_pos).
    SetSelAnchor(Option<(usize, usize)>),
    /// Set the selection cursor position by source byte offset.
    SetSelCursorByte(Option<usize>),
    /// Set the selection anchor position by source byte offset.
    SetSelAnchorByte(Option<usize>),
    /// Clear the current selection.
    ClearSelection,
    /// 请求插件拦截处理结构编辑按键。
    /// 插件可调用 doc 的方法直接修改源码。返回 true 表示已消费。
    InterceptKey { key: crate::core::widget::KeyCode, modifiers: crate::core::widget::Modifiers },
    /// Select all text in the preview.
    SelectAll,
    /// Update host-controlled render settings.
    SetRenderSettings {
        font_size: f32,
        line_height: f32,
        toc_max_depth: u8,
        markdown_first_line_indent: bool,
    },
    /// Restore absolute scroll position (for switching back from edit mode).
    SetScrollY(f32),
    /// Restore scroll position as a ratio (0.0~1.0) of content height.
    SetScrollRatio(f32),
    /// Restore scroll position by content anchor (text snippet + pixel offset).
    RestoreScrollAnchor { text: String, offset: f32 },
    /// Notify plugin that the cursor byte offset in source has changed.
    SetCursorByte(usize),
    /// Notify plugin whether the cursor blink phase is currently visible.
    SetCursorVisible(bool),
    /// Leave the plugin's object-specific editing focus without changing source text.
    ClearEditFocus,
    /// Notify WYSIWYG plugins about the current IME composing text.
    SetPreedit { text: String, cursor: Option<(usize, usize)> },
    /// Notify canvas plugins about the latest pointer position in screen coordinates.
    SetCanvasPointer(Option<CanvasPoint>),
}

// ---------------------------------------------------------------------------
// Directional / augmentation types (WYSIWYG)
// ---------------------------------------------------------------------------

/// Visual navigation direction for WYSIWYG cursor movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

/// Semantic destination for editing interactions in a custom-rendered view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditHitTarget {
    /// Place a collapsed caret at a source byte offset.
    ///
    /// `selection_scope` bounds a drag that starts at this caret. `None`
    /// preserves unconstrained source-text selection for generic plugins.
    TextCaret { byte_offset: usize, selection_scope: Option<std::ops::Range<usize>> },
    /// Select the exact source range represented by a rendered object.
    SourceObject { source_range: std::ops::Range<usize> },
    /// Activate a rendered canvas control backed by a source range.
    CanvasControl { source_range: std::ops::Range<usize> },
    /// Leave object-specific editing and place the caret after the source.
    ClearFocus,
}

/// Edit-augmentation kind — which key the host is about to process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AugmentKind {
    Enter,
    LineBreak,
    Backspace,
    Tab,
    InsertText(String),
}

/// Suggested augmentation for a pending edit action.
///
/// When a WYSIWYG plugin wants to override default edit behavior (e.g. continue
/// a list on Enter, delete a paired delimiter on Backspace), it returns this
/// struct from [`PluginQuery::AugmentEdit`].
#[derive(Debug, Clone, Default)]
pub struct EditAugmentation {
    /// Replacement delete range (overrides default single-char backspace). `None` = no change.
    pub replace_range: Option<std::ops::Range<usize>>,
    /// Replacement insert text (overrides default `"\n"` or `""`). `None` = use default.
    pub insert_text: Option<String>,
    /// Cursor byte offset after the augmented edit.
    pub cursor_byte_after: usize,
}

/// Context passed to `EditAugmenter` for processing an edit.
pub struct AugmentContext<'a> {
    pub current_byte: usize,
    pub kind: AugmentKind,
    pub doc: &'a dyn DocView,
}

pub trait EditAugmenter {
    fn augment(&self, ctx: &AugmentContext) -> Option<EditAugmentation>;
}

pub trait KeyInterceptor {
    /// Give the plugin a chance to intercept and handle raw key events.
    /// Returns true if the key event was consumed.
    fn intercept_key(
        &self,
        key: &crate::core::widget::KeyCode,
        modifiers: &crate::core::widget::Modifiers,
        doc: &mut dyn DocViewMut,
    ) -> bool;
}

/// Maps raw keys to declarative editing intents without mutating the document.
pub trait KeyIntentMapper {
    fn map_key(
        &self,
        key: &crate::core::widget::KeyCode,
        modifiers: &crate::core::widget::Modifiers,
    ) -> Option<EditIntent>;
}

pub struct NoopAugmenter;
impl EditAugmenter for NoopAugmenter {
    fn augment(&self, _ctx: &AugmentContext) -> Option<EditAugmentation> {
        None
    }
}

/// Synchronous queries from the host into a plugin.
#[derive(Debug)]
pub enum PluginQuery {
    /// Current vertical scroll position (pixels).
    ScrollY,
    /// Total content height (pixels).
    ContentHeight,
    /// Whether the source cache is stale for the given generation.
    NeedsSourceUpdate(u32),
    /// List all TOC headings.
    TOCHeadings,
    /// Which heading index is closest to the given scroll-y.
    CurrentHeadingIndex(f32),
    /// Whether the plugin has an active text selection.
    HasSelection,
    /// The currently selected text, if any.
    SelectedText,
    /// (line, column) of the selection cursor, if any.
    SelCursor,
    /// Byte range of the selection, if any.
    SelectionRange,
    /// Hit-test at screen coordinates.
    HitTest {
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    },
    /// Collect search highlight quads.
    SearchHighlights {
        query: String,
        match_case: bool,
        use_regex: bool,
        active_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    },
    /// Collect selection highlight quads in the given color.
    SelectionHighlights([f32; 4]),
    /// Flatten visible lines into text for copy/search.
    FlatLines,
    /// Word boundaries at a position (flat_line_idx, grapheme_pos).
    WordAtPos(usize, usize),
    /// Line range at a position (flat_line_idx, grapheme_pos).
    LineRangeAtPos(usize, usize),
    /// Get content-anchored scroll position (text_snippet, pixel_offset).
    ScrollAnchor,
    /// 画布坐标 → 源码字节偏移。用于点击画布节点定位光标。
    HitTestCanvas {
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    },
    /// 源码字节偏移 → 画布像素坐标 (x, y, height)。用于光标绘制和 IME 定位。
    CursorRect(usize),
    CanvasMove {
        from_byte: usize,
        direction: Direction,
    },
    /// Source byte offset → screen pixel rect (x, y, w, h). For IME positioning.
    CursorScreenPos(usize),
    /// Screen pixel → source byte offset. For mouse-click byte positioning.
    HitTestByte {
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    },
    /// Screen pixel → semantic target for editing interactions.
    HitTestEditTarget {
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    },
    /// Plan an edit for a canvas control represented by a source range.
    PlanCanvasControl {
        source_range: std::ops::Range<usize>,
        source_generation: u32,
    },
    /// Report the file-level mindmap theme selection for the current document.
    MindmapThemeSelection,
    /// Plan an edit that sets the file-level mindmap theme.
    PlanMindmapTheme {
        theme_id: String,
        source_generation: u32,
    },
    /// Visual direction navigation from `current_byte` (considering folded/invisible markers).
    VisualMove {
        current_byte: usize,
        direction: MoveDirection,
        /// Preferred X pixel position when moving up/down (sticky column).
        target_x: Option<f32>,
    },
    /// Semantic direction navigation from the current source byte.
    MoveEditTarget {
        current_byte: usize,
        direction: MoveDirection,
        /// Preferred X pixel position when moving up/down (sticky column).
        target_x: Option<f32>,
    },
    /// Plan a product semantic editing command without mutating the document.
    PlanSemanticEdit {
        command: SemanticEditCommand,
        source_generation: u32,
        cursor_byte: usize,
        selection: Option<std::ops::Range<usize>>,
    },
}

/// Responses returned from [`PluginQuery`].
#[derive(Debug)]
pub enum PluginResponse {
    /// No meaningful result (e.g. query not applicable).
    None,
    Float(f32),
    Bool(bool),
    String(String),
    Headings(Vec<HeadingEntry>),
    Position(Option<(usize, usize)>),
    DrawList(DrawList),
    FlatLines(Vec<FlatLine>),
    /// A pair of positions (start, end).
    PositionPair(Option<((usize, usize), (usize, usize))>),
    /// Content-anchored scroll position (text_snippet, pixel_offset).
    ScrollAnchor(String, f32),
    HitResult(Option<HitResult>),
    /// 光标的画布物理坐标 (x, y, height)。None = 字节偏移不在任何节点内。
    CursorRect(Option<(f32, f32, f32)>),
    /// Cursor rect in document coordinates (x, y, w, h).
    CursorScreenRect(Option<(f32, f32, f32, f32)>),
    /// Source byte offset.
    BytePosition(Option<usize>),
    /// Semantic target for an editing interaction.
    EditHitTarget(Option<EditHitTarget>),
    /// Declarative edit plan for a canvas control.
    EditPlan(EditPlan),
    /// Edit augmentation suggestion.
    Augmentation(Option<EditAugmentation>),
    /// The file-level mindmap theme selection reported by the plugin.
    MindmapThemeSelection(crate::theme::MindmapThemeSelection),
    /// Result of planning a product semantic editing command.
    SemanticEdit(SemanticEditPlan),
}

// ---------------------------------------------------------------------------
// Shared data types
// ---------------------------------------------------------------------------

/// A single entry in the table-of-contents.
#[derive(Debug, Clone)]
pub struct HeadingEntry {
    pub title: String,
    pub y: f32,
    pub level: u8,
}

/// A flattened visual line used for search-indexing, clipboard operations, and geometry queries.
#[derive(Debug, Clone)]
pub struct FlatLine {
    pub text: String,
    /// Number of UAX#29 grapheme clusters on this line.
    pub grapheme_count: usize,
    /// Absolute document geometry for this visual line.
    pub rect: Rect,
    /// X advance at every visual grapheme boundary, including the sentinel.
    pub grapheme_x: Vec<f32>,
}

/// Geometry reported by plugins that opt into canvas rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasContentMetrics {
    /// Bounds of all canvas content in content coordinates.
    pub content_bounds: Rect,
    /// Optional content-space point the host can use as an initial focus target.
    pub focus_anchor: Option<CanvasPoint>,
}

/// Hit-test 结果：画布坐标 → 源码信息。
#[derive(Debug, Clone)]
pub struct HitResult {
    pub byte_offset: usize, // 在源码中的精确字节位置
    pub node_idx: usize,    // 命中的节点 DFS 索引
}

/// 画布方向导航。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

// ---------------------------------------------------------------------------
// Edit Transaction Protocol
// ---------------------------------------------------------------------------

/// Product-level commands whose document-specific edit semantics belong to a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticEditCommand {
    Undo,
    Redo,
    SetHeadingLevel(u8),
    ToggleBold,
    ToggleItalic,
    ToggleStrikethrough,
    ToggleInlineCode,
    UnorderedList,
    OrderedList,
    TaskList,
    Quote,
    CodeBlock,
    InsertLink,
    PromoteObject,
    DemoteObject,
}

/// Typed result returned by a plugin when planning a semantic command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticEditPlan {
    Unsupported,
    NoChange,
    Apply(EditTransaction),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditIntent {
    InsertText(String),
    InsertParagraphBreak,
    InsertLineBreak,
    DeleteBackward,
    DeleteForward,
    Indent,
    Outdent,
    PromoteObject,
    DemoteObject,
    SelectObject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub source_generation: u32,
    pub cursor_byte: usize,
    pub selection: Option<std::ops::Range<usize>>,
    pub intent: EditIntent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextReplacement {
    pub range: std::ops::Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditSelection {
    Caret(usize),
    Range { anchor: usize, cursor: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTransaction {
    pub source_generation: u32,
    pub replacements: Vec<TextReplacement>,
    pub selection_after: EditSelection,
}

impl EditTransaction {
    pub fn replace(
        source_generation: u32,
        range: std::ops::Range<usize>,
        text: String,
        cursor_after: usize,
    ) -> Self {
        Self {
            source_generation,
            replacements: vec![TextReplacement { range, text }],
            selection_after: EditSelection::Caret(cursor_after),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorUpdate {
    pub cursor_after: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditPlan {
    UseDefault,
    Apply(EditTransaction),
    /// Resolved default-plan transaction. Unlike plugin-provided [`EditPlan::Apply`],
    /// it may coalesce with adjacent default edits per its [`EditHistoryKind`]
    /// (continuous typing / backspace runs share one undo entry).
    ApplyDefault(EditTransaction, EditHistoryKind),
    SetSelection(EditSelection),
    MoveCursor(CursorUpdate),
    Consume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasDragPhase {
    Start,
    Update,
    Drop,
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasDragRequest {
    pub phase: CanvasDragPhase,
    pub source_range: std::ops::Range<usize>,
    pub pointer_x: f32,
    pub pointer_y: f32,
    pub pressed_x: f32,
    pub pressed_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub source_generation: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasDragPreview {
    /// 预览卡片内显示的纯文本标签；由具体画布插件生成。
    pub label: String,
    pub source_rect: Rect,
    pub source_subtree_rects: Vec<Rect>,
    pub preview_rect: Rect,
    pub guide_from: (f32, f32),
    pub guide_to: Option<(f32, f32)>,
    pub insertion_line: Option<((f32, f32), (f32, f32))>,
    pub target_rect: Option<Rect>,
    pub is_valid: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasDragResponse {
    Ignore,
    Preview(CanvasDragPreview),
    Apply(EditTransaction),
}

pub trait EditPolicy {
    fn plan_edit(&self, request: &EditRequest) -> EditPlan;
}

pub struct NoopEditPolicy;

impl EditPolicy for NoopEditPolicy {
    fn plan_edit(&self, _request: &EditRequest) -> EditPlan {
        EditPlan::UseDefault
    }
}

// ---------------------------------------------------------------------------
// ViewPlugin trait
// ---------------------------------------------------------------------------

/// Clipboard representation a view can safely accept from a smart paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastePreference {
    /// Insert the clipboard's plain text without interpreting semantic styling.
    PlainText,
    /// Convert semantic clipboard markup into Markdown before insertion.
    SemanticMarkdown,
}

/// Trait implemented by pluggable view renderers.
///
/// A plugin is responsible for rendering a document region and optionally
/// handling messages / queries from the host editor.
pub trait ViewPlugin {
    /// Human-readable name (e.g. `"reader"`, `"editor"`).
    fn name(&self) -> &str;

    /// Clipboard representation this view prefers for regular paste.
    fn paste_preference(&self) -> PastePreference {
        PastePreference::PlainText
    }

    /// Render the visible portion of the document and return a draw list.
    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: Rect,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList;

    /// Prepare optional canvas content metrics before the host resolves a viewport.
    ///
    /// Non-canvas plugins return `None` and retain their existing render path.
    fn prepare_canvas(
        &mut self,
        _doc: &dyn DocView,
        _theme: &Theme,
        _shaper: &mut Shaper,
        _dpi_scale: f32,
    ) -> Option<CanvasContentMetrics> {
        None
    }

    /// Render through a resolved canvas viewport.
    ///
    /// The default preserves the established plugin render behavior by passing
    /// the viewport's screen bounds to [`ViewPlugin::render`].
    fn render_canvas(
        &mut self,
        doc: &dyn DocView,
        viewport: &CanvasViewportSnapshot,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        self.render(doc, viewport.viewport, theme, shaper, dpi_scale)
    }

    /// Handle a command message. Returns `true` if the message was consumed.
    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        let _ = (msg, doc);
        false
    }

    /// Answer a synchronous query.
    fn query(&self, query: PluginQuery, doc: &dyn DocView) -> PluginResponse {
        let _ = (query, doc);
        PluginResponse::None
    }

    /// Handle a canvas drag request using plugin-specific document semantics.
    fn handle_canvas_drag(
        &mut self,
        _request: CanvasDragRequest,
        _doc: &dyn DocView,
    ) -> CanvasDragResponse {
        CanvasDragResponse::Ignore
    }

    /// Whether this plugin renders its own text cursor.
    fn shows_cursor(&self) -> bool {
        true
    }

    /// Whether the app should still handle cursor blink wakeups for this plugin.
    ///
    /// Returns `shows_cursor()` by default. Override when a plugin draws its own
    /// cursor (e.g. WYSIWYG) but needs the app to compute and forward blink phase.
    fn needs_cursor_blink_wakeup(&self) -> bool {
        self.shows_cursor()
    }

    /// Whether this plugin renders its own gutter.
    fn shows_gutter(&self) -> bool {
        true
    }

    /// Whether the host should allow text editing when this plugin is active.
    fn allows_editing(&self) -> bool {
        true
    }

    /// Whether this plugin handles its own rendering via [`render()`].
    ///
    /// When `true`, the host delegates content rendering to [`render()`];
    /// when `false`, the host uses the standard text-shaping pipeline.
    fn handles_own_rendering(&self) -> bool {
        !self.allows_editing()
    }

    /// Retrieve the edit policy for this plugin.
    fn edit_policy(&self) -> &dyn EditPolicy {
        &NoopEditPolicy
    }

    /// Retrieve the edit augmenter for this plugin.
    fn augmenter(&self) -> &dyn EditAugmenter {
        &NoopAugmenter
    }

    /// Retrieve the declarative key-to-edit-intent mapper for this plugin, if any.
    fn key_intent_mapper(&self) -> Option<&dyn KeyIntentMapper> {
        None
    }

    /// Retrieve the key interceptor for this plugin, if any.
    fn key_interceptor(&self) -> Option<&dyn KeyInterceptor> {
        None
    }

    /// Whether this plugin acts as an infinite canvas and should occupy the full
    /// available bounds (minus TOC), ignoring reading-mode max-width constraints.
    fn is_canvas(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// PluginFactory & PluginRegistry
// ---------------------------------------------------------------------------

/// Factory trait: create [`ViewPlugin`] instances on demand.
///
/// Factories are `Send + Sync` so they can live in a global registry.
pub trait PluginFactory: Send + Sync {
    /// Human-readable name of the plugin this factory produces.
    fn name(&self) -> &str;

    /// Return `true` if this factory can produce a plugin for the given file.
    fn can_handle(&self, path: Option<&Path>) -> bool;

    /// Create a fresh plugin instance.
    fn create(&self) -> Box<dyn ViewPlugin>;
}

// ── 插件名常量 ──
/// 基础文本编辑器插件。
pub const PLUGIN_EDITOR: &str = "editor";
/// Markdown WYSIWYG 编辑器插件。
pub const PLUGIN_MARKDOWN_EDITOR: &str = "markdown_editor";
/// 小说阅读视图插件。
pub const PLUGIN_NOVEL_VIEW: &str = "novel_view";
/// 思维导图插件。
pub const PLUGIN_MINDMAP: &str = "mindmap";

/// Registry of [`PluginFactory`] instances.
///
/// The host queries the registry to resolve which plugin should handle a file.
pub struct PluginRegistry {
    factories: Vec<Box<dyn PluginFactory>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self { factories: Vec::new() }
    }

    /// Register a new factory. Factories are tried in insertion order.
    pub fn register(&mut self, factory: Box<dyn PluginFactory>) {
        self.factories.push(factory);
    }

    /// Find the first factory whose [`PluginFactory::can_handle`] returns `true`
    /// for `path`, create a plugin from it, and return it.
    ///
    /// If no factory matches, the provided `editor_fallback` is returned.
    pub fn create_for_file(
        &self,
        path: Option<&Path>,
        editor_fallback: Box<dyn ViewPlugin>,
    ) -> Box<dyn ViewPlugin> {
        for factory in &self.factories {
            if factory.can_handle(path) {
                return factory.create();
            }
        }
        editor_fallback
    }

    /// Like [`create_for_file`] but returns the **last** matching factory
    /// (reverse insertion order).  When the preview factory is registered
    /// before the editor factory, this naturally picks the editor variant.
    ///
    /// Falls back to `editor_fallback` when no factory matches.
    pub fn create_editor_for_file(
        &self,
        path: Option<&Path>,
        editor_fallback: Box<dyn ViewPlugin>,
    ) -> Box<dyn ViewPlugin> {
        for factory in self.factories.iter().rev() {
            if factory.can_handle(path) {
                return factory.create();
            }
        }
        editor_fallback
    }

    /// 按插件名查找工厂并创建插件。找不到则返回 fallback。
    pub fn create_by_name(&self, name: &str, fallback: Box<dyn ViewPlugin>) -> Box<dyn ViewPlugin> {
        self.factories.iter().find(|f| f.name() == name).map(|f| f.create()).unwrap_or(fallback)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct StructuralKeyMapper;

    impl KeyIntentMapper for StructuralKeyMapper {
        fn map_key(
            &self,
            key: &crate::core::widget::KeyCode,
            modifiers: &crate::core::widget::Modifiers,
        ) -> Option<EditIntent> {
            match (key, modifiers.cmd || modifiers.ctrl) {
                (crate::core::widget::KeyCode::Char('['), true) => Some(EditIntent::PromoteObject),
                (crate::core::widget::KeyCode::Char(']'), true) => Some(EditIntent::DemoteObject),
                _ => None,
            }
        }
    }

    struct StubPlugin {
        name: &'static str,
    }

    struct TestDoc {
        source: String,
    }

    impl TestDoc {
        fn new(source: impl Into<String>) -> Self {
            Self { source: source.into() }
        }
    }

    impl DocView for TestDoc {
        fn line_count(&self) -> usize {
            self.source.lines().count().max(1)
        }

        fn doc_line_text(&self, line: usize) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed(self.source.lines().nth(line).unwrap_or_default())
        }

        fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> std::borrow::Cow<'_, str> {
            let start = range.start.min(self.source.len());
            let end = range.end.min(self.source.len());
            std::borrow::Cow::Borrowed(&self.source[start..end])
        }

        fn line_byte_offset(&self, line: usize) -> usize {
            self.source.lines().take(line).map(|text| text.len() + 1).sum()
        }

        fn line_byte_length(&self, line: usize) -> usize {
            self.source.lines().nth(line).map_or(0, str::len)
        }

        fn scroll_y(&self) -> f32 {
            0.0
        }

        fn viewport_height(&self) -> f32 {
            0.0
        }
    }

    impl ViewPlugin for StubPlugin {
        fn name(&self) -> &str {
            self.name
        }
        fn render(
            &mut self,
            _: &dyn DocView,
            _: Rect,
            _: &Theme,
            _: &mut Shaper,
            _: f32,
        ) -> DrawList {
            DrawList::new()
        }
    }

    #[test]
    fn view_plugin_defaults_to_plain_text_paste() {
        let plugin = StubPlugin { name: "plain-text" };

        assert_eq!(plugin.paste_preference(), PastePreference::PlainText);
    }

    struct CanvasProtocolPlugin {
        rendered_bounds: Option<Rect>,
    }

    impl ViewPlugin for CanvasProtocolPlugin {
        fn name(&self) -> &str {
            "canvas-protocol"
        }

        fn render(
            &mut self,
            _: &dyn DocView,
            bounds: Rect,
            _: &Theme,
            _: &mut Shaper,
            _: f32,
        ) -> DrawList {
            self.rendered_bounds = Some(bounds);
            DrawList::new()
        }
    }

    struct StubFactory {
        factory_name: &'static str,
        ext: &'static str,
    }

    impl PluginFactory for StubFactory {
        fn name(&self) -> &str {
            self.factory_name
        }
        fn can_handle(&self, path: Option<&Path>) -> bool {
            path.and_then(|p| p.extension()).is_some_and(|e| e == self.ext)
        }
        fn create(&self) -> Box<dyn ViewPlugin> {
            Box::new(StubPlugin { name: self.factory_name })
        }
    }

    #[test]
    fn registry_returns_fallback_when_no_match() {
        let registry = PluginRegistry::new();
        let fallback = Box::new(StubPlugin { name: "fallback" });
        let result = registry.create_for_file(Some(Path::new("foo.xyz")), fallback);
        assert_eq!(result.name(), "fallback");
    }

    #[test]
    fn registry_matches_first_factory() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(StubFactory { factory_name: "md", ext: "md" }));
        registry.register(Box::new(StubFactory { factory_name: "txt", ext: "txt" }));
        let result = registry
            .create_for_file(Some(Path::new("readme.md")), Box::new(StubPlugin { name: "fb" }));
        assert_eq!(result.name(), "md");
    }

    #[test]
    fn registry_matches_second_factory() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(StubFactory { factory_name: "md", ext: "md" }));
        registry.register(Box::new(StubFactory { factory_name: "txt", ext: "txt" }));
        let result = registry
            .create_for_file(Some(Path::new("novel.txt")), Box::new(StubPlugin { name: "fb" }));
        assert_eq!(result.name(), "txt");
    }

    #[test]
    fn handles_own_rendering_defaults_to_false() {
        let plugin = StubPlugin { name: "test" };
        assert!(!plugin.handles_own_rendering());
    }

    #[test]
    fn view_plugin_ignores_canvas_drag_by_default() {
        let mut plugin = StubPlugin { name: "stub" };
        let response = plugin.handle_canvas_drag(
            CanvasDragRequest {
                phase: CanvasDragPhase::Start,
                source_range: 3..8,
                pointer_x: 40.0,
                pointer_y: 60.0,
                pressed_x: 30.0,
                pressed_y: 50.0,
                offset_x: 0.0,
                offset_y: 0.0,
                source_generation: 7,
            },
            &TestDoc::new("abcdefghi"),
        );
        assert!(matches!(response, CanvasDragResponse::Ignore));
    }

    mod canvas_protocol {
        use super::*;
        use crate::canvas::{
            CanvasPoint, CanvasViewportConfig, CanvasViewportInput, resolve_viewport,
        };
        use crate::theme::ThemeDefinition;

        const DPI_SCALE: f32 = 2.0;

        fn viewport_bounds() -> Rect {
            Rect::new(10.0, 20.0, 300.0, 400.0)
        }

        fn content_bounds() -> Rect {
            Rect::new(-50.0, -60.0, 100.0, 120.0)
        }

        #[test]
        fn canvas_content_metrics_only_exposes_geometry() {
            let metrics = CanvasContentMetrics {
                content_bounds: content_bounds(),
                focus_anchor: Some(CanvasPoint::new(12.0, 34.0)),
            };

            assert_eq!(metrics.content_bounds, content_bounds());
            assert_eq!(metrics.focus_anchor, Some(CanvasPoint::new(12.0, 34.0)));
        }

        #[test]
        fn normal_plugin_prepare_canvas_defaults_to_none() {
            let mut plugin = CanvasProtocolPlugin { rendered_bounds: None };
            let document = TestDoc::new("canvas");
            let theme = Theme::from_definition(&ThemeDefinition::default_dark());
            let mut shaper =
                Shaper::new().expect("test shaper should initialize with system fonts");

            assert_eq!(plugin.prepare_canvas(&document, &theme, &mut shaper, DPI_SCALE), None);
        }

        #[test]
        fn normal_plugin_render_canvas_delegates_to_render_with_viewport_bounds() {
            let mut plugin = CanvasProtocolPlugin { rendered_bounds: None };
            let document = TestDoc::new("canvas");
            let theme = Theme::from_definition(&ThemeDefinition::default_dark());
            let mut shaper =
                Shaper::new().expect("test shaper should initialize with system fonts");
            let viewport = resolve_viewport(CanvasViewportInput::initial(
                viewport_bounds(),
                content_bounds(),
                CanvasViewportConfig::DEFAULT,
            ));

            plugin.render_canvas(&document, &viewport, &theme, &mut shaper, DPI_SCALE);

            assert_eq!(plugin.rendered_bounds, Some(viewport_bounds()));
        }
    }

    #[test]
    fn edit_augmentation_default_is_empty() {
        let aug = EditAugmentation::default();
        assert!(aug.replace_range.is_none());
        assert!(aug.insert_text.is_none());
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn registry_none_path_returns_fallback() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(StubFactory { factory_name: "md", ext: "md" }));
        let result = registry.create_for_file(None, Box::new(StubPlugin { name: "fb" }));
        assert_eq!(result.name(), "fb");
    }

    #[test]
    fn create_by_name_returns_matching_factory() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(StubFactory { factory_name: "alpha", ext: "a" }));
        registry.register(Box::new(StubFactory { factory_name: "beta", ext: "b" }));
        let result = registry.create_by_name("beta", Box::new(StubPlugin { name: "fb" }));
        assert_eq!(result.name(), "beta");
    }

    #[test]
    fn create_by_name_returns_fallback_when_no_match() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(StubFactory { factory_name: "alpha", ext: "a" }));
        let result = registry.create_by_name("nonexistent", Box::new(StubPlugin { name: "fb" }));
        assert_eq!(result.name(), "fb");
    }

    #[test]
    fn create_by_name_returns_fallback_for_empty_registry() {
        let registry = PluginRegistry::new();
        let result = registry.create_by_name("anything", Box::new(StubPlugin { name: "fb" }));
        assert_eq!(result.name(), "fb");
    }

    #[test]
    fn noop_edit_policy_requests_default_transaction() {
        let request = EditRequest {
            source_generation: 7,
            cursor_byte: 4,
            selection: Some(1..4),
            intent: EditIntent::DeleteBackward,
        };

        assert_eq!(NoopEditPolicy.plan_edit(&request), EditPlan::UseDefault);
    }

    #[test]
    fn transaction_state_is_expressed_by_enum_variants() {
        let transaction = EditTransaction::replace(7, 4..4, "\n\n".into(), 6);

        assert!(matches!(EditPlan::Apply(transaction), EditPlan::Apply(_)));
        assert!(matches!(
            EditPlan::SetSelection(EditSelection::Range { anchor: 3, cursor: 9 }),
            EditPlan::SetSelection(_)
        ));
        assert!(matches!(
            EditPlan::MoveCursor(CursorUpdate { cursor_after: 9 }),
            EditPlan::MoveCursor(_)
        ));
    }

    #[test]
    fn structural_key_mapper_returns_transactional_intent() {
        let modifiers =
            crate::core::widget::Modifiers { cmd: true, ..crate::core::widget::Modifiers::NONE };

        assert_eq!(
            StructuralKeyMapper.map_key(&crate::core::widget::KeyCode::Char(']'), &modifiers),
            Some(EditIntent::DemoteObject)
        );
    }
}
