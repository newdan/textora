//! Plugin infrastructure for view-mode rendering.
//!
//! Provides [`ViewPlugin`] trait for pluggable renderers,
//! [`PluginFactory`] for deferred creation, and [`PluginRegistry`] for dispatch.

use std::path::Path;

use shaping::Shaper;
use textora_core::buffer::EditHistoryKind;
use textora_core::document::{DocView, DocViewMut};
pub use textora_core::types::CursorAffinity;

use crate::canvas::{CanvasPoint, CanvasViewportSnapshot};
use crate::core::geom::Rect;
use crate::core::paint::DrawList;
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Messages & Queries
// ---------------------------------------------------------------------------

/// Commands sent from the host to a plugin.
#[derive(Debug)]
pub enum PluginMessage {}

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

/// A source caret together with its unambiguous visual position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualCaretPosition {
    pub byte_offset: usize,
    pub affinity: CursorAffinity,
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
    ShiftTab,
    InsertText(String),
}

/// Suggested augmentation for a pending edit action.
///
/// When a WYSIWYG plugin wants to override default edit behavior (e.g. continue
/// a list on Enter, delete a paired delimiter on Backspace), it returns this
/// struct when planning the edit.
#[derive(Debug, Clone, Default)]
pub struct EditAugmentation {
    /// Replacement delete range (overrides default single-char backspace). `None` = no change.
    pub replace_range: Option<std::ops::Range<usize>>,
    /// Replacement insert text (overrides default `"\n"` or `""`). `None` = use default.
    pub insert_text: Option<String>,
    /// Cursor byte offset after the augmented edit.
    pub cursor_byte_after: usize,
}

/// Maps raw keys to declarative editing intents without mutating the document.
pub trait KeyIntentMapper {
    fn map_key(
        &self,
        key: &crate::core::widget::KeyCode,
        modifiers: &crate::core::widget::Modifiers,
    ) -> Option<EditIntent>;
}

/// Synchronous queries from the host into a plugin.
#[derive(Debug)]
pub enum PluginQuery {
    /// Report the file-level mindmap theme selection for the current document.
    MindmapThemeSelection,
    /// Plan an edit that sets the file-level mindmap theme.
    PlanMindmapTheme { theme_id: String, source_generation: u32 },
}

/// Responses returned from [`PluginQuery`].
#[derive(Debug)]
pub enum PluginResponse {
    /// No meaningful result (e.g. query not applicable).
    None,
    /// Declarative edit plan for a canvas control.
    EditPlan(EditPlan),
    /// The file-level mindmap theme selection reported by the plugin.
    MindmapThemeSelection(crate::theme::MindmapThemeSelection),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChapterDirection {
    Previous,
    Next,
}

pub trait OutlineProvider {
    fn headings(&self) -> Vec<HeadingEntry>;
    fn current_heading_index(&self, scroll_y: f32) -> Option<usize>;
    fn scroll_to_heading(&mut self, index: usize);

    fn scroll_to_chapter(&mut self, _direction: ChapterDirection) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadingPosition {
    pub line_index: usize,
    pub grapheme_index: usize,
}

impl ReadingPosition {
    pub const fn new(line_index: usize, grapheme_index: usize) -> Self {
        Self { line_index, grapheme_index }
    }

    pub const fn indices(self) -> (usize, usize) {
        (self.line_index, self.grapheme_index)
    }
}

pub trait SelectionView {
    fn has_selection(&self) -> bool {
        false
    }

    fn selected_text(&self) -> Option<String> {
        None
    }

    fn selection_cursor(&self) -> Option<ReadingPosition> {
        None
    }

    fn selection_source_range(&self) -> Option<std::ops::Range<usize>> {
        None
    }

    fn hit_test_position(
        &self,
        _x: f32,
        _y: f32,
        _offset_x: f32,
        _offset_y: f32,
    ) -> Option<ReadingPosition> {
        None
    }

    fn word_range(&self, _position: ReadingPosition) -> Option<std::ops::Range<ReadingPosition>> {
        None
    }

    fn line_range(&self, _position: ReadingPosition) -> Option<std::ops::Range<ReadingPosition>> {
        None
    }

    fn flat_lines(&self) -> Vec<FlatLine> {
        Vec::new()
    }

    fn selection_highlights(&self, _color: [f32; 4]) -> DrawList {
        DrawList::new()
    }

    fn set_selection_cursor(
        &mut self,
        _position: Option<ReadingPosition>,
        _document: &dyn DocView,
    ) -> bool {
        false
    }

    fn set_selection_anchor(
        &mut self,
        _position: Option<ReadingPosition>,
        _document: &dyn DocView,
    ) -> bool {
        false
    }

    fn set_selection_cursor_byte(&mut self, _byte: Option<usize>) -> bool {
        false
    }

    fn set_selection_anchor_byte(&mut self, _byte: Option<usize>) -> bool {
        false
    }

    fn clear_selection(&mut self) -> bool {
        false
    }

    fn select_all(&mut self) -> bool {
        false
    }
}

pub struct SearchHighlightsRequest<'a> {
    pub query: &'a str,
    pub match_case: bool,
    pub use_regex: bool,
    pub active_idx: usize,
    pub match_color: [f32; 4],
    pub inactive_color: [f32; 4],
}

pub struct SourceSearchHighlightsRequest<'a> {
    pub matches: &'a [std::ops::Range<usize>],
    pub source_generation: u32,
    pub active_idx: usize,
    pub match_color: [f32; 4],
    pub inactive_color: [f32; 4],
}

pub trait SearchView {
    fn search_highlights(&self, request: SearchHighlightsRequest<'_>) -> DrawList;

    fn source_search_highlights(
        &self,
        _request: SourceSearchHighlightsRequest<'_>,
    ) -> Option<DrawList> {
        None
    }

    fn scroll_to_search_match(&mut self, _query: &str, _match_case: bool, _active_idx: usize) {}
}

pub struct ViewRenderSettings {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub toc_max_depth: u8,
    pub markdown_first_line_indent: bool,
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
    InsertTable { columns: usize, rows: usize },
    TableStructure(TableStructureCommand),
    PromoteObject,
    DemoteObject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableStructureCapabilities {
    pub can_delete_row: bool,
    pub can_delete_column: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableStructureCommand {
    InsertRowBefore,
    InsertRowAfter,
    DeleteRow,
    InsertColumnBefore,
    InsertColumnAfter,
    DeleteColumn,
    SetColumnAlignment(TableColumnAlignment),
    DeleteTable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableColumnAlignment {
    Default,
    Left,
    Center,
    Right,
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

/// Determines which rendering pipeline owns the document content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginMode {
    Source,
    Reading,
    RichText,
    Canvas,
}

impl PluginMode {
    pub fn renders_content(self) -> bool {
        self != Self::Source
    }

    pub fn uses_host_gutter(self) -> bool {
        self == Self::Source
    }

    pub fn uses_host_cursor(self) -> bool {
        self == Self::Source
    }

    pub fn needs_cursor_blink_wakeup(self) -> bool {
        self != Self::Reading
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncReason {
    Normal,
    Force,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    Unchanged,
    Updated,
}

/// Core interface shared by all view plugins.
pub trait ViewPlugin {
    fn name(&self) -> &str;

    fn mode(&self) -> PluginMode;

    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: Rect,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList;

    /// Request the exact source only when this view needs to refresh a local snapshot.
    fn sync_document(
        &mut self,
        _generation: u32,
        _reason: SyncReason,
        _load_source: &mut dyn FnMut() -> String,
    ) -> SyncOutcome {
        SyncOutcome::Unchanged
    }
}

/// Scroll state and restoration supported by a self-rendering view.
pub trait ScrollableView {
    fn scroll_y(&self, _document: &dyn DocView) -> f32 {
        0.0
    }

    fn content_height(&self) -> f32 {
        0.0
    }

    fn scroll(&mut self, _delta: f32, _viewport_height: f32) -> bool {
        false
    }

    fn restore_y(&mut self, _y: f32) -> bool {
        false
    }

    fn restore_ratio(&mut self, _ratio: f32) -> bool {
        false
    }

    fn scroll_anchor(&self) -> Option<(String, f32)> {
        None
    }

    fn restore_anchor(&mut self, _text: &str, _offset: f32) -> bool {
        false
    }
}

/// Discovers capabilities borrowed from the same instance that renders the view.
pub trait ViewPluginCapabilities: ViewPlugin {
    fn set_view_render_settings(&mut self, _settings: ViewRenderSettings) {}

    fn set_view_text_spacing_mode(&mut self, _mode: crate::typography::TextSpacingMode) {}

    fn scrollable(&self) -> Option<&dyn ScrollableView> {
        None
    }

    fn scrollable_mut(&mut self) -> Option<&mut dyn ScrollableView> {
        None
    }

    fn editable(&self) -> Option<&dyn EditableView> {
        None
    }

    fn editable_mut(&mut self) -> Option<&mut dyn EditableView> {
        None
    }

    fn canvas(&self) -> Option<&dyn CanvasView> {
        None
    }

    /// Source ranges whose glyph spacing must remain literal.
    fn source_spacing_protected_ranges(
        &self,
        _source_generation: u32,
    ) -> Option<std::sync::Arc<[std::ops::Range<usize>]>> {
        None
    }

    fn canvas_mut(&mut self) -> Option<&mut dyn CanvasView> {
        None
    }

    fn outline(&self) -> Option<&dyn OutlineProvider> {
        None
    }

    fn outline_mut(&mut self) -> Option<&mut dyn OutlineProvider> {
        None
    }

    fn selection(&self) -> Option<&dyn SelectionView> {
        None
    }

    fn selection_mut(&mut self) -> Option<&mut dyn SelectionView> {
        None
    }

    fn search(&self) -> Option<&dyn SearchView> {
        None
    }

    fn search_mut(&mut self) -> Option<&mut dyn SearchView> {
        None
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

    /// Whether the app should still handle cursor blink wakeups for this plugin.
    ///
    /// The mode supplies the default. A custom view can override its refresh policy.
    fn needs_cursor_blink_wakeup(&self) -> bool {
        self.mode().needs_cursor_blink_wakeup()
    }

    /// Whether the host should allow text editing when this plugin is active.
    fn allows_editing(&self) -> bool {
        self.mode() != PluginMode::Reading
    }
}

/// Editing and input capabilities of the active rendered view.
pub trait EditableView: EditPolicy {
    /// Plan a semantic command against the requested source generation.
    fn plan_semantic_edit(
        &self,
        _command: SemanticEditCommand,
        _source_generation: u32,
        _cursor_byte: usize,
        _selection: Option<std::ops::Range<usize>>,
    ) -> SemanticEditPlan {
        SemanticEditPlan::Unsupported
    }

    /// Resolve a screen point to a source caret, retaining soft-wrap affinity.
    fn hit_test_caret(
        &self,
        _x: f32,
        _y: f32,
        _offset_x: f32,
        _offset_y: f32,
    ) -> Option<VisualCaretPosition> {
        None
    }

    /// Move through visual text, using the preferred x position for vertical navigation.
    fn visual_move(
        &self,
        _current_byte: usize,
        _direction: MoveDirection,
        _target_x: Option<f32>,
    ) -> Option<VisualCaretPosition> {
        None
    }

    /// Outer `None` delegates to host caret hit-testing. `Some(None)` is a handled miss.
    fn hit_test_edit_target(
        &self,
        _x: f32,
        _y: f32,
        _offset_x: f32,
        _offset_y: f32,
    ) -> Option<Option<EditHitTarget>> {
        None
    }

    /// Find an object or scoped text target for directional editing.
    fn move_edit_target(
        &self,
        _current_byte: usize,
        _direction: MoveDirection,
        _target_x: Option<f32>,
    ) -> Option<EditHitTarget> {
        None
    }

    /// Return the caret rectangle (x, y, width, height) for IME placement.
    fn cursor_screen_rect(&self, _cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        None
    }

    fn set_cursor_byte(&mut self, _cursor_byte: usize) {}

    /// Place a source caret, retaining the side of a soft-wrap boundary.
    fn set_visual_caret(&mut self, _position: VisualCaretPosition) {}

    fn clear_edit_focus(&mut self) {}

    /// Schedule the caret to be revealed and return whether the view will handle it.
    fn reveal_cursor(&mut self, _cursor_byte: usize) -> bool {
        false
    }

    fn set_cursor_visible(&mut self, _visible: bool) {}

    /// Update IME composition. Empty text clears the composition.
    fn set_preedit(&mut self, _text: String, _cursor: Option<(usize, usize)>) {}

    fn paste_preference(&self) -> PastePreference {
        PastePreference::PlainText
    }

    fn key_intent_mapper(&self) -> Option<&dyn KeyIntentMapper> {
        None
    }

    /// Return available actions for the table at the specified source byte.
    fn table_structure_capabilities(
        &self,
        _source_generation: u32,
        _cursor_byte: usize,
    ) -> Option<TableStructureCapabilities> {
        None
    }
}

/// Interactive canvas behavior driven by the host's resolved viewport.
pub trait CanvasView: ViewPlugin {
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

    /// Find a canvas control at a screen-space point.
    fn hit_test_canvas_control(&self, _position: CanvasPoint) -> Option<std::ops::Range<usize>> {
        None
    }

    /// Plan activation of a canvas control identified by its source range.
    fn plan_canvas_control(
        &self,
        _source_range: std::ops::Range<usize>,
        _source_generation: u32,
    ) -> EditPlan {
        EditPlan::Consume
    }

    /// Update the canvas pointer and report whether to redraw.
    fn set_canvas_pointer(&mut self, _pointer: Option<CanvasPoint>) -> bool {
        false
    }

    /// Handle a canvas drag request using plugin-specific document semantics.
    fn handle_canvas_drag(
        &mut self,
        _request: CanvasDragRequest,
        _doc: &dyn DocView,
    ) -> CanvasDragResponse {
        CanvasDragResponse::Ignore
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
    fn create(&self) -> Box<dyn ViewPluginCapabilities>;
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
        editor_fallback: Box<dyn ViewPluginCapabilities>,
    ) -> Box<dyn ViewPluginCapabilities> {
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
        editor_fallback: Box<dyn ViewPluginCapabilities>,
    ) -> Box<dyn ViewPluginCapabilities> {
        for factory in self.factories.iter().rev() {
            if factory.can_handle(path) {
                return factory.create();
            }
        }
        editor_fallback
    }

    /// 按插件名查找工厂并创建插件。找不到则返回 fallback。
    pub fn create_by_name(
        &self,
        name: &str,
        fallback: Box<dyn ViewPluginCapabilities>,
    ) -> Box<dyn ViewPluginCapabilities> {
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

        fn mode(&self) -> super::PluginMode {
            super::PluginMode::Source
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

    impl super::ViewPluginCapabilities for StubPlugin {}

    impl CanvasView for StubPlugin {}

    #[test]
    fn plugin_modes_route_content_and_host_cursor_explicitly() {
        for (mode, renders_content, shows_gutter, wakes_cursor) in [
            (PluginMode::Source, false, true, true),
            (PluginMode::Reading, true, false, false),
            (PluginMode::RichText, true, false, true),
            (PluginMode::Canvas, true, false, true),
        ] {
            assert_eq!(mode.renders_content(), renders_content);
            assert_eq!(mode.uses_host_gutter(), shows_gutter);
            assert_eq!(mode.needs_cursor_blink_wakeup(), wakes_cursor);
        }
    }

    #[test]
    fn default_document_sync_does_not_load_source() {
        let mut plugin = StubPlugin { name: "document-view" };
        let mut load = || panic!("a plugin without a source cache must not load the document");
        for reason in [SyncReason::Normal, SyncReason::Force] {
            assert_eq!(plugin.sync_document(0, reason, &mut load), SyncOutcome::Unchanged);
        }
    }

    #[test]
    fn editable_capability_mutates_the_rendering_instance() {
        struct EditableProbe {
            caret: usize,
        }

        impl ViewPlugin for EditableProbe {
            fn name(&self) -> &str {
                "editable-probe"
            }

            fn mode(&self) -> PluginMode {
                PluginMode::RichText
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

        impl EditPolicy for EditableProbe {
            fn plan_edit(&self, _: &EditRequest) -> EditPlan {
                EditPlan::UseDefault
            }
        }

        impl EditableView for EditableProbe {
            fn set_cursor_byte(&mut self, caret: usize) {
                self.caret = caret;
            }

            fn cursor_screen_rect(&self, caret: usize) -> Option<(f32, f32, f32, f32)> {
                (caret == self.caret).then_some((0.0, 0.0, 1.0, 1.0))
            }
        }

        impl ViewPluginCapabilities for EditableProbe {
            fn editable(&self) -> Option<&dyn EditableView> {
                Some(self)
            }

            fn editable_mut(&mut self) -> Option<&mut dyn EditableView> {
                Some(self)
            }
        }

        let mut plugin: Box<dyn ViewPluginCapabilities> = Box::new(EditableProbe { caret: 0 });
        let caret = 4;

        assert!(plugin.editable().and_then(|view| view.cursor_screen_rect(caret)).is_none());
        plugin.editable_mut().expect("probe exposes editable state").set_cursor_byte(caret);
        let view = plugin.editable().expect("probe exposes editable state");
        assert!(view.cursor_screen_rect(caret).is_some());
        assert_eq!(view.paste_preference(), PastePreference::PlainText);
        assert_eq!(
            view.plan_semantic_edit(SemanticEditCommand::ToggleBold, 7, caret, None),
            SemanticEditPlan::Unsupported,
        );
    }

    struct CanvasProtocolPlugin {
        rendered_bounds: Option<Rect>,
    }

    impl ViewPlugin for CanvasProtocolPlugin {
        fn name(&self) -> &str {
            "canvas-protocol"
        }

        fn mode(&self) -> super::PluginMode {
            super::PluginMode::Source
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

    impl super::ViewPluginCapabilities for CanvasProtocolPlugin {}

    impl CanvasView for CanvasProtocolPlugin {}

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
        fn create(&self) -> Box<dyn ViewPluginCapabilities> {
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
        assert!(!plugin.mode().renders_content());
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
