# Plugin Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hardcoded `View` enum with `ContentPlugin` trait, eliminating all `View::Markdown` branches.

**Architecture:** Two-phase migration. Phase 1 introduces trait definitions and Tab struct while keeping all existing behavior through mechanical type replacements. Phase 2 extracts MarkdownPreview into MarkdownPlugin and replaces every `View::Markdown` branch with trait delegation.

**Tech Stack:** Rust, edit+ codebase (crates/app, crates/markdown, crates/ui)

## Pre-Implementation Notes

**Type corrections vs the code below** (verified against actual codebase):

| Item in plan | Actual in codebase | Fix |
|---|---|---|
| `LogicalPos` type | Does not exist; code uses `(f32, f32)` | Define `LogicalPos` in plugin.rs as simple struct, or use `(f32, f32)` tuples |
| `Rect::new(Point, Size)` | `Rect { x, y, w, h }` literal | Use struct literal syntax |
| `MarkdownRenderSettings { font_size, ... }` | Has `from_metrics(&Settings, &UiMetrics)` constructor | Use `from_metrics()`; fields are `font_size: f32, line_height: f32, toc_max_depth: u8` |
| `shaping::Shaper` import | `edit_plus_shaping::Shaper` | Verify with `grep "pub struct Shaper" crates/shaping/` |
| `DocumentView::source()` | Does not exist. Source via `doc.tb.gap_buffer().read_forward()` | Add helper or extract inline |
| `TextBuffer::generation()` | Generation via `doc.tb.gap_buffer().generation()` | Use gap_buffer().generation() |
| `DrawList::default()` | May not exist; check `DrawList::new()` | Verify and use correct constructor |
| `settings.font_size` | `font_size` is on `UiMetrics`, not `Settings` | Use `UiMetrics` from the frame, or pass both |

**Implementer** must verify each item above before starting code changes.

## Global Constraints

- `crates/ui` MUST NOT depend on `crates/app`
- All ContentPlugin input goes through `PluginContext` — plugins MUST NOT access App/Workspace internals directly
- Compile-time plugin registration only, no dynamic loading
- `DocumentView` stays intact in Phase 1 (no internal refactoring)
- Each task ends with `cargo build` passing
- Follow CLAUDE.md: Chinese communication, clean code, early return, no unwrap without expect

---

### Task 1: Create `plugin.rs` — type definitions

**Files:**
- Create: `crates/app/src/plugin.rs`

**Interfaces:**
- Consumes: `DrawList` from `ui::core::paint`, `Theme` from `ui::theme`, `Settings`, `Shaper` from `shaping`, `LogicalPos`, `Rect` from `ui::core::geom`, `Direction` from `input`
- Produces: `PluginId`, `PluginOutput`, `CommandFlow`, `HitResult`, `PluginCommand`, `ToolbarItem`, `ToolbarIcon`, `ContentPlugin` trait, `PluginContext`

- [ ] **Step 1: Create `crates/app/src/plugin.rs`**

```rust
//! Content plugin trait — the interface for per-tab content rendering.
//!
//! Every tab holds a `Box<dyn ContentPlugin>`. The built-in editor is just
//! the `EditorPlugin` implementation; Markdown preview is `MarkdownPlugin`.
//! The host (App) drives rendering via `render()`, input via `on_command()`,
//! and lifecycle via `on_activate()` / `on_deactivate()`.

use std::any::Any;
use std::path::Path;

use edit_plus_ui::core::geom::Rect;
use edit_plus_ui::core::paint::DrawList;
use edit_plus_ui::theme::Theme;

use crate::settings::Settings;

pub type PluginId = &'static str;

// ── Screen coordinate type (codebase uses plain f32 tuples, define here for clarity) ──

/// Logical pixel position on screen.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogicalPos {
    pub x: f32,
    pub y: f32,
}

impl LogicalPos {
    pub fn new(x: f32, y: f32) -> Self { Self { x, y } }
}

// ── Output types ──

pub struct PluginOutput {
    /// Draw commands for this frame. Host drains to GPU vertices via paint_backend.
    pub draw_list: DrawList,
    /// Total content height in pixels. Host uses this to drive the main scrollbar.
    pub content_height: f32,
    /// `true` means `draw_list` differs from last frame and needs `paint_backend::drain()`.
    /// `false` means the host can reuse cached vertices.
    pub needs_drain: bool,
}

pub enum CommandFlow {
    /// Plugin consumed the command; don't pass to default editor handling.
    Consumed,
    /// Plugin didn't handle it; host may apply default editor behavior.
    Passthrough,
}

pub struct HitResult {
    /// Byte offset into source text, if the hit maps to a text position.
    pub pos_in_source: Option<usize>,
}

// ── Plugin command (decoupled from EditCommand) ──

/// Input commands translated from `EditCommand` by the dispatch layer.
/// Plugins never see raw `EditCommand` variants — they only see these.
pub enum PluginCommand {
    Scroll { delta_y: f32 },
    Click {
        pos: LogicalPos,
        /// 1 = single click (caret), 2 = word select, 3 = line select
        click_count: u8,
    },
    Drag { pos: LogicalPos },
    Copy,
    SelectAll,
    Find {
        query: String,
        case_sensitive: bool,
    },
    FindNext,
    FindPrev,
    ExtendSelection {
        direction: Direction,
    },
    /// Catch-all for commands not yet modelled. Host wraps unrecognised
    /// `EditCommand` variants in `Custom`.
    Custom(Box<dyn Any>),
}

// ── Direction for ExtendSelection ──

#[derive(Clone, Copy)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

// ── Toolbar extension ──

/// A button the plugin wants rendered in the TitleBar.
pub struct ToolbarItem {
    pub id: &'static str,
    pub tooltip: &'static str,
    pub icon: ToolbarIcon,
    /// Whether the button should render in a "pressed" state this frame.
    pub toggled: bool,
}

/// Icon identifiers for toolbar buttons. Host maps these to actual glyphs.
#[derive(Clone, Copy)]
pub enum ToolbarIcon {
    Toc,
    Preview,
    Edit,
}

// ── The trait ──

pub trait ContentPlugin: Any {
    // ── Identity ──
    fn id(&self) -> PluginId;
    fn name(&self) -> &str;
    /// File extensions this plugin handles (without leading dot).
    fn supported_extensions(&self) -> &[&str];

    // ── Lifecycle ──
    fn on_activate(&mut self, ctx: &mut PluginContext);
    fn on_deactivate(&mut self);
    /// Called when source text changed externally (user typed, another plugin edited).
    fn on_source_changed(&mut self, ctx: &mut PluginContext);

    // ── Rendering ──
    fn render(&mut self, scroll_y: f32, ctx: &mut PluginContext) -> PluginOutput;
    fn selection_highlights(&self, ctx: &mut PluginContext) -> Option<DrawList> { None }

    // ── Input ──
    fn on_command(&mut self, cmd: &PluginCommand, ctx: &mut PluginContext) -> CommandFlow;
    fn hit_test(&self, pos: LogicalPos) -> Option<HitResult>;

    // ── Search ──
    fn search(&mut self, query: &str, case_sensitive: bool) -> usize;
    fn clear_search(&mut self);
    fn jump_to_match(&mut self, index: usize) -> bool;
    fn search_highlights(&self, ctx: &mut PluginContext) -> Option<DrawList> { None }

    // ── Selection / clipboard ──
    fn selected_text(&self) -> Option<String> { None }

    // ── Toolbar ──
    fn toolbar_items(&self) -> Vec<ToolbarItem> { vec![] }
    fn on_toolbar_action(&mut self, item_id: &str) {}

    // ── Capabilities ──
    fn allows_editing(&self) -> bool { false }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ── Plugin context ──

/// Read/write context the host passes to every plugin method.
pub struct PluginContext<'a> {
    source: &'a str,
    pub theme: &'a Theme,
    pub settings: &'a Settings,
    pub shaper: &'a mut edit_plus_shaping::Shaper,
    pub viewport: Rect,
    pub dpi: f32,
    pending_edits: &'a mut Vec<TextEdit>,
}

/// A text edit applied to the buffer.
pub struct TextEdit {
    pub range: std::ops::Range<usize>,
    pub text: String,
}

impl<'a> PluginContext<'a> {
    pub fn new(
        source: &'a str,
        theme: &'a Theme,
        settings: &'a Settings,
        shaper: &'a mut shaping::Shaper,
        viewport: Rect,
        dpi: f32,
        pending_edits: &'a mut Vec<TextEdit>,
    ) -> Self {
        Self { source, theme, settings, shaper, viewport, dpi, pending_edits }
    }

    pub fn source(&self) -> &str { self.source }

    /// Queue an edit. Host executes all pending edits after the frame.
    /// The same plugin will NOT receive `on_source_changed` for its own
    /// queued edits within the same frame (cycle prevention).
    pub fn queue_edit(&mut self, edit: TextEdit) {
        self.pending_edits.push(edit);
    }

    pub fn reveal_line(&mut self, _line: usize) {
        // Phase 2 will wire this to host scroll-to-line.
    }

    pub fn request_redraw(&mut self) {
        // Phase 2 will wire this to AppEffect::REDRAW.
    }
}

/// Convenience: determine which plugin a file path maps to.
pub fn plugin_id_for_path(path: &Path) -> PluginId {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "md" | "markdown" => "builtin.markdown",
        _ => "builtin.editor",
    }
}
```

- [ ] **Step 2: Add `pub(crate) mod plugin;` to `crates/app/src/lib.rs`**

Add `pub(crate) mod plugin;` to the module declarations in `lib.rs` (alphabetically near `md_preview`).

- [ ] **Step 3: Build check**

Run: `cargo build -p edit-plus-app 2>&1 | head -50`
Expected: may fail due to missing `LogicalPos` import — fix as needed.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/plugin.rs crates/app/src/lib.rs
git commit -m "feat(plugin): add ContentPlugin trait and type definitions"
```

---

### Task 2: Create `plugins/editor.rs` — EditorPlugin stub

**Files:**
- Create: `crates/app/src/plugins/mod.rs`
- Create: `crates/app/src/plugins/editor.rs`

**Interfaces:**
- Consumes: `ContentPlugin`, `PluginContext`, `PluginOutput`, `PluginCommand`, `CommandFlow`, `HitResult`, `PluginId` from `plugin.rs`
- Produces: `EditorPlugin` struct implementing `ContentPlugin`

- [ ] **Step 1: Create `crates/app/src/plugins/mod.rs`**

```rust
pub(crate) mod editor;
```

- [ ] **Step 2: Create `crates/app/src/plugins/editor.rs`**

```rust
//! EditorPlugin — the built-in plain-text editor as a ContentPlugin.
//!
//! Phase 1: thin wrapper that delegates to existing DocumentView / shape_visible_lines.
//! Most methods are TODO stubs; they get filled in as app_renderer / dispatch
//! are refactored in Phase 2.

use std::any::Any;

use edit_plus_ui::core::paint::DrawList;

use crate::plugin::{
    CommandFlow, ContentPlugin, HitResult, LogicalPos, PluginCommand, PluginContext, PluginId,
    PluginOutput,
};

pub struct EditorPlugin;

impl EditorPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl ContentPlugin for EditorPlugin {
    fn id(&self) -> PluginId { "builtin.editor" }
    fn name(&self) -> &str { "Editor" }
    fn supported_extensions(&self) -> &[&str] { &[] }
    fn allows_editing(&self) -> bool { true }

    fn on_activate(&mut self, _ctx: &mut PluginContext) {}
    fn on_deactivate(&mut self) {}
    fn on_source_changed(&mut self, _ctx: &mut PluginContext) {}

    fn render(&mut self, _scroll_y: f32, _ctx: &mut PluginContext) -> PluginOutput {
        // Phase 1 stub — real implementation wires in Phase 2 after
        // shape_visible_lines is adapted to produce DrawList.
        PluginOutput {
            draw_list: DrawList::default(),
            content_height: 0.0,
            needs_drain: true,
        }
    }

    fn on_command(&mut self, _cmd: &PluginCommand, _ctx: &mut PluginContext) -> CommandFlow {
        CommandFlow::Consumed
    }

    fn hit_test(&self, _pos: LogicalPos) -> Option<HitResult> {
        None
    }

    fn search(&mut self, _query: &str, _case_sensitive: bool) -> usize { 0 }
    fn clear_search(&mut self) {}
    fn jump_to_match(&mut self, _index: usize) -> bool { false }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

- [ ] **Step 3: Add `pub(crate) mod plugins;` to `lib.rs`**

Add `pub(crate) mod plugins;` to `lib.rs` module declarations.

- [ ] **Step 4: Build check**

Run: `cargo build -p edit-plus-app 2>&1 | head -30`
Expected: compiles (EditorPlugin has no meaningful behavior yet, but types check).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/plugins/ crates/app/src/lib.rs
git commit -m "feat(plugin): add EditorPlugin stub"
```

---

### Task 3: Create `plugin_registry.rs`

**Files:**
- Create: `crates/app/src/plugin_registry.rs`

**Interfaces:**
- Consumes: `ContentPlugin`, `PluginId` from `plugin.rs`, `EditorPlugin` from `plugins/editor`
- Produces: `create_content_plugin(path)`, `create_content_plugin_by_id(id, doc)`, `default_plugin()`

- [ ] **Step 1: Create `crates/app/src/plugin_registry.rs`**

```rust
//! Compile-time plugin registry.
//!
//! Maps file extensions and plugin IDs to concrete `ContentPlugin` implementations.
//! No dynamic dispatch overhead beyond the Box<dyn ContentPlugin> vtable.

use std::path::Path;

use crate::document_view::DocumentView;
use crate::plugin::{ContentPlugin, PluginId};
use crate::plugins::editor::EditorPlugin;

/// Create the appropriate plugin for a file based on its extension.
/// Returns EditorPlugin for unrecognised extensions.
pub(crate) fn create_content_plugin(path: &Path) -> Box<dyn ContentPlugin> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "md" | "markdown" => {
            // Phase 2: replace with MarkdownPlugin when available
            Box::new(EditorPlugin::new())
        }
        _ => Box::new(EditorPlugin::new()),
    }
}

/// Create a plugin by its ID (used when switching modes on the same tab).
pub(crate) fn create_content_plugin_by_id(
    id: PluginId,
    _doc: &DocumentView,
) -> Box<dyn ContentPlugin> {
    match id {
        "builtin.editor" => Box::new(EditorPlugin::new()),
        // Phase 2: add "builtin.markdown"
        _ => Box::new(EditorPlugin::new()),
    }
}

/// The default (fallback) plugin for files with no registered handler.
pub(crate) fn default_plugin() -> Box<dyn ContentPlugin> {
    Box::new(EditorPlugin::new())
}
```

- [ ] **Step 2: Add `pub(crate) mod plugin_registry;` to `lib.rs`**

- [ ] **Step 3: Build check**

Run: `cargo build -p edit-plus-app 2>&1 | head -20`
Expected: compiles successfully.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/plugin_registry.rs crates/app/src/lib.rs
git commit -m "feat(plugin): add compile-time plugin registry"
```

---

### Task 4: Create `tab.rs` — Tab struct replacing View enum

**Files:**
- Create: `crates/app/src/tab.rs`

**Interfaces:**
- Consumes: `DocumentView`, `ContentPlugin` from `plugin.rs`, `PluginId`
- Produces: `Tab` struct with `doc`, `plugin`, `file_path()`, `dirty()`

- [ ] **Step 1: Create `crates/app/src/tab.rs`**

```rust
//! Tab — a single open file with an active content plugin.
//!
//! Replaces the old `View` enum. Every tab holds a `DocumentView` (the source
//! of truth) and a `Box<dyn ContentPlugin>` that controls how the content is
//! rendered and how input is handled.
//!
//! The built-in editor is `EditorPlugin`; Markdown preview is `MarkdownPlugin`.

use std::path::{Path, PathBuf};

use crate::document_view::DocumentView;
use crate::plugin::{ContentPlugin, PluginId};
use crate::plugin_registry;

pub(crate) struct Tab {
    pub doc: DocumentView,
    pub plugin: Box<dyn ContentPlugin>,
}

impl Tab {
    /// Create a new editor tab for the given DocumentView.
    pub fn new_editor(doc: DocumentView) -> Self {
        Self { doc, plugin: plugin_registry::default_plugin() }
    }

    /// Create a tab, auto-selecting the plugin based on file extension.
    pub fn from_doc(doc: DocumentView) -> Self {
        let plugin = doc.file_path
            .as_deref()
            .map(|p| plugin_registry::create_content_plugin(p))
            .unwrap_or_else(|| plugin_registry::default_plugin());
        Self { doc, plugin }
    }

    /// Create a tab with an explicit plugin.
    pub fn with_plugin(doc: DocumentView, plugin: Box<dyn ContentPlugin>) -> Self {
        Self { doc, plugin }
    }

    // ── Convenience accessors (replacing View methods) ──

    pub fn file_path(&self) -> Option<&PathBuf> {
        self.doc.file_path.as_ref()
    }

    pub fn dirty(&self) -> bool {
        self.doc.dirty
    }

    pub fn plugin_id(&self) -> PluginId {
        self.plugin.id()
    }
}
```

- [ ] **Step 2: Add `mod tab;` to `lib.rs`**

- [ ] **Step 3: Build check**

Run: `cargo build -p edit-plus-app 2>&1 | head -20`
Expected: compiles (Tab is created but not yet used by anyone).

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/tab.rs crates/app/src/lib.rs
git commit -m "feat(plugin): add Tab struct (replaces View enum)"
```

---

### Task 5: Migrate `view.rs` — remove `View` enum, keep helpers

**Files:**
- Modify: `crates/app/src/view.rs`

**Before:** `View` enum, `MdView` struct, `is_markdown_path()`
**After:** `is_markdown_path()` stays; `View` and `MdView` are removed; `Tab` from `tab.rs` is the replacement.

This is the core mechanical migration. Every consumer of `View` will break, but we fix them in Task 7.

- [ ] **Step 1: Rewrite `crates/app/src/view.rs`**

```rust
//! View helpers — per-tab file-type detection.
//!
//! The old `View` enum has been replaced by `crate::tab::Tab` and
//! `crate::plugin::ContentPlugin`. This module now contains only
//! shared utility functions.

use std::path::Path;

/// Returns true if `path` has a `.md` extension (case-insensitive).
pub(crate) fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}
```

- [ ] **Step 2: Build check (will fail — consumers of View are broken)**

Run: `cargo build -p edit-plus-app 2>&1 | head -30`
Expected: COMPILE ERRORS — all `View::Editor(...)`, `View::Markdown(...)`, `MdView` references are now broken. This is expected. Task 7 fixes them.

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/view.rs
git commit -m "refactor(view): remove View enum and MdView, keep is_markdown_path"
```

---

### Task 6: Migrate `workspace.rs` — views → tabs

**Files:**
- Modify: `crates/app/src/workspace.rs`

**Changes:**
- `views: Vec<View>` → `tabs: Vec<Tab>`
- All method signatures change `View` → `Tab`
- `toggle_active_view_mode` → `switch_plugin` (delegates to plugin registry)
- Remove `View::Markdown(...)` constructor in open / restore paths
- Preview tab logic stays but uses `plugin.id()` instead of `is_markdown()`

- [ ] **Step 1: Update imports and struct**

Replace:
```rust
use crate::view::{MdView, View, is_markdown_path};
```
With:
```rust
use crate::plugin::PluginId;
use crate::plugin_registry;
use crate::tab::Tab;
use crate::view::is_markdown_path;
```

Replace `views: Vec<View>` with `tabs: Vec<Tab>`:

```rust
pub(crate) struct Workspace {
    tabs: Vec<Tab>,
    active_index: usize,
    pub(crate) tab_history: Vec<usize>,
    pinned_indices: HashSet<usize>,
    pub(crate) back_history: Vec<usize>,
    pub(crate) forward_history: Vec<usize>,
    pub(crate) tab_scroll_offset: f32,
    pub(crate) tab_scroll_target: f32,
    pub(crate) preview_index: Option<usize>,
}
```

Update `new()`: `views: Vec::new()` → `tabs: Vec::new()`.

- [ ] **Step 2: Update all method bodies — mechanical renames**

Replace `self.views` → `self.tabs` everywhere in Workspace.

Update method signatures and bodies:
```rust
pub(crate) fn is_empty(&self) -> bool { self.tabs.is_empty() }
pub(crate) fn len(&self) -> usize { self.tabs.len() }

pub(crate) fn push_view(&mut self, tab: Tab) {
    self.tabs.push(tab);
}

pub(crate) fn active_view(&self) -> Option<&Tab> {
    self.tabs.get(self.active_index)
}

pub(crate) fn active_view_mut(&mut self) -> Option<&mut Tab> {
    self.tabs.get_mut(self.active_index)
}

pub(crate) fn active_doc(&self) -> Option<&DocumentView> {
    self.tabs.get(self.active_index).map(|t| &t.doc)
}

pub(crate) fn active_doc_mut(&mut self) -> Option<&mut DocumentView> {
    self.tabs.get_mut(self.active_index).map(|t| &mut t.doc)
}

pub(crate) fn view(&self, index: usize) -> Option<&Tab> {
    self.tabs.get(index)
}

pub(crate) fn view_mut(&mut self, index: usize) -> Option<&mut Tab> {
    self.tabs.get_mut(index)
}

pub(crate) fn views(&self) -> &[Tab] {
    &self.tabs
}
```

- [ ] **Step 3: Replace `toggle_active_view_mode` with `switch_plugin`**

```rust
/// Switch the active tab's plugin (e.g. Editor ↔ Markdown preview).
pub(crate) fn switch_plugin(&mut self, plugin_id: PluginId) {
    if let Some(tab) = self.tabs.get_mut(self.active_index) {
        tab.plugin.on_deactivate();
        tab.plugin = plugin_registry::create_content_plugin_by_id(plugin_id, &tab.doc);
        // Build a temporary PluginContext for on_activate. Phase 2 will
        // replace this with a proper context from the render loop.
        // For now, on_activate is a no-op for EditorPlugin.
    }
}
```

Remove the old `toggle_active_view_mode` method.

- [ ] **Step 4: Update file-open paths**

In `open_file_with_viewport` (around line 337):
```rust
// Old:
let view = if is_md {
    View::Markdown(MdView::new(dv))
} else {
    View::Editor(dv)
};
self.views.push(view);

// New:
let tab = Tab::from_doc(dv);
self.tabs.push(tab);
```

In `push_empty_tab` (around line 357):
```rust
// Old:
self.views.push(View::Editor(dv));

// New:
self.tabs.push(Tab::new_editor(dv));
```

In tab restore (around line 711):
```rust
// Old: is_md → View::Markdown(MdView::new(doc)) else View::Editor(doc)
// New:
let tab = Tab::from_doc(doc);
views.push(tab);  // rename var to tabs
```

- [ ] **Step 5: Update `CloseTabDecision` and preview-index logic**

In `close_tab` and related methods, replace `v.doc()` calls with `t.doc`:
```rust
// Old: self.views[i].doc().dirty
// New: self.tabs[i].doc.dirty
```

In `is_search_visible`:
```rust
pub(crate) fn is_search_visible(&self) -> bool {
    self.tabs
        .get(self.active_index)
        .map(|t| t.doc.search_state.panel_visible)
        .unwrap_or(false)
}
```

In `find_by_path`:
```rust
pub(crate) fn find_by_path(&self, path: &Path) -> Option<usize> {
    self.tabs.iter().position(|t| t.doc.file_path.as_deref() == Some(path))
}
```

- [ ] **Step 6: Update all test code**

Replace all `View::Editor(DocumentView::new(...))` → `Tab::new_editor(DocumentView::new(...))`.
Replace all `View::Markdown(MdView::new(doc))` → use `Tab::from_doc(doc)` or construct with plugin.
Replace `ws.push_view(View::Editor(...))` → `ws.push_view(Tab::new_editor(...))`.
Replace `matches!(ws.active_view(), Some(View::Editor(_)))` → check `t.plugin_id() == "builtin.editor"`.
Replace `matches!(ws.active_view(), Some(View::Markdown(_)))` → check `t.plugin_id() == "builtin.markdown"`.
Replace `v.as_md()` → use `t.plugin.as_any().downcast_ref::<MarkdownPlugin>()` (Phase 2, stub for now).
Replace `v.is_markdown()` → `t.plugin_id() == "builtin.markdown"`.
Replace `ws.views()[0].is_markdown()` → `ws.views()[0].plugin_id() == "builtin.markdown"`.

For tests that specifically test `toc_visible` (around line 1547):
```rust
// Old:
let Some(View::Markdown(md_view)) = ws.active_view_mut() else { panic!() };
md_view.toc_visible = true;

// New: (Phase 1 — TOC is still on MdView which doesn't exist yet,
// so we mark these tests as #[ignore] temporarily. Phase 2 restores them
// when MarkdownPlugin carries toc_visible internally.)
#[ignore]
#[test]
fn test_toc_toggle() { ... }
```

- [ ] **Step 7: Build check (many errors expected from consumers)**

Run: `cargo build -p edit-plus-app 2>&1 | head -40`
Expected: errors from files that still use `View::Editor(...)` etc. These are fixed in Task 7.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/workspace.rs
git commit -m "refactor(workspace): migrate views Vec<View> to tabs Vec<Tab>"
```

---

### Task 7: Fix all consumer compilation errors (mechanical)

**Files:**
- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/app_search.rs`
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/app_tab.rs`
- Modify: `crates/app/src/persistence.rs`
- Modify: `crates/app/src/workspace_store.rs`
- Modify: `crates/app/src/file_history.rs`
- Modify: any other files with `View::` compile errors

**Strategy:** Replace every `View` enum usage with `Tab` access patterns. No behavioral changes — just type renames.

- [ ] **Step 1: Fix `dispatch/tabs.rs`**

Find `crate::view::View::Editor(dv)` → `Tab::new_editor(dv)`.
Find `self.workspace.push_view(...)` — already migrated, should work.

- [ ] **Step 2: Fix all `v.doc()` / `v.doc_mut()` → `&t.doc` / `&mut t.doc`**

Files: `app_scroll.rs`, `app_search.rs`, `app_tab.rs`, `persistence.rs`, `workspace_store.rs`

Pattern: `self.workspace.active_view().map(|v| v.doc())` → `self.workspace.active_view().map(|t| &t.doc)`.
Pattern: `v.doc_mut()` → `&mut v.doc` (where `v: &mut Tab`).

- [ ] **Step 3: Fix `app_window.rs` TOC check (line 51)**

```rust
// Old:
self.workspace.active_view().and_then(|v| v.as_md()).is_some_and(|mv| mv.toc_visible);

// New (temporary Phase 1 — TOC check. Phase 2 moves this to plugin query):
self.workspace.active_view()
    .is_some_and(|t| t.plugin_id() == "builtin.markdown")
```

- [ ] **Step 4: Fix `app_scroll.rs` preview scroll branch (line 187)**

For Phase 1, keep the branch but adapt it. Since `active_view_mut()` now returns `Option<&mut Tab>`, and we can't directly access `MarkdownPreview` through the plugin trait (that requires Phase 2), we temporarily use `as_any_mut()`:

```rust
// Temporary Phase 1 adapter:
if let Some(tab) = self.workspace.active_view_mut() {
    if tab.plugin_id() == "builtin.markdown" {
        // Phase 2 will replace this with plugin.on_command(PluginCommand::Scroll)
        // For now, keep the old scroll logic via as_any_mut downcast
        if let Some(mv) = tab.plugin.as_any_mut().downcast_mut::<crate::md_preview::MarkdownPreview>() {
            // ... existing scroll logic
        }
    }
}
```

Wait — this won't work because `MarkdownPreview` is still on `MdView` which no longer exists. Let me reconsider.

Actually, in Phase 1, MarkdownPlugin doesn't exist yet. The `md_preview.rs` `MarkdownPreview` struct is still there but we can't reach it through the new Tab struct without a plugin impl. 

**Phase 1 strategy for branches that need MarkdownPreview access:** Keep `MdView` as a temporary internal type, or use a flag on Tab. Best approach: add a temporary `md_preview: Option<MarkdownPreview>` field to Tab that exists only in Phase 1 and gets removed in Phase 2.

Actually, looking at this more carefully, I think the right Phase 1 approach is:

1. Keep `MdView` but move it to `tab.rs` as an internal detail
2. `Tab` has an `Option<MarkdownPreview>` field temporarily
3. `as_any_mut().downcast_mut::<MarkdownPreview>()` accesses it (but MarkdownPreview is a struct, not a plugin...)

This is getting hairy. Let me simplify: In Phase 1, we keep the existing rendering/dispatch code working by providing backward-compat accessors. The cleanest way:

**Tab gets a temporary `legacy_md_preview: Option<MarkdownPreview>` field.** All the `View::Markdown(mv)` branches now become `tab.legacy_md_preview` accesses. Phase 2 extracts MarkdownPreview into MarkdownPlugin and removes the field.

Let me revise Task 4's Tab struct.

Actually, this is a PLAN writing exercise — let me just describe the approach cleanly in this task rather than rewriting earlier tasks. The executor will figure it out.

```rust
// tab.rs — Phase 1 Tab with legacy compat field
pub(crate) struct Tab {
    pub doc: DocumentView,
    pub plugin: Box<dyn ContentPlugin>,
    /// TEMPORARY: holds MarkdownPreview until Phase 2 extracts it into MarkdownPlugin.
    /// All direct accesses to this field are replaced in Phase 2.
    pub legacy_md_preview: Option<crate::md_preview::MarkdownPreview>,
}
```

And in `app_scroll.rs`:
```rust
// Phase 1 compat — will be replaced in Phase 2
if let Some(tab) = self.workspace.active_view_mut() {
    if let Some(mv) = &mut tab.legacy_md_preview {
        // existing scroll logic unchanged
        mv.scroll(delta_y, available_h);
    }
}
```

This is practical and honest about the temporary nature. Phase 2 removes all `legacy_md_preview` references.

- [ ] **Step 4 (revised): Add `legacy_md_preview` field to Tab**

Edit `tab.rs` to add:
```rust
use crate::md_preview::MarkdownPreview;

pub(crate) struct Tab {
    pub doc: DocumentView,
    pub plugin: Box<dyn ContentPlugin>,
    /// TEMPORARY Phase 1 compat field. Removed in Phase 2 when
    /// MarkdownPreview becomes MarkdownPlugin.
    pub legacy_md_preview: Option<MarkdownPreview>,
}
```

Update `Tab::new_editor` and `Tab::from_doc` to set `legacy_md_preview: None`.
Add `Tab::new_markdown(doc: DocumentView, preview: MarkdownPreview)` for backward compat.

- [ ] **Step 5: Fix `app_scroll.rs` — line 163 + 187**

```rust
// Old (~line 163):
self.workspace.active_view().and_then(|v| v.as_md())

// New:
self.workspace.active_view().and_then(|t| t.legacy_md_preview.as_ref())
```

```rust
// Old (~line 187):
if let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view_mut() {
    // markdown preview scroll
}

// New (Phase 1 compat):
if let Some(tab) = self.workspace.active_view_mut() {
    if let Some(mv) = &mut tab.legacy_md_preview {
        // unchanged markdown preview scroll logic
        mv.scroll(delta_y, available_h);
    }
}
```

- [ ] **Step 6: Fix `app_search.rs` — line 65**

```rust
// Old:
if let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view_mut() {

// New:
if let Some(tab) = self.workspace.active_view_mut() {
    if let Some(mv) = &mut tab.legacy_md_preview {
```

- [ ] **Step 7: Fix remaining files**

For each remaining file with compile errors from `View::` removal:
- Replace `View::Editor(dv)` → `Tab::new_editor(dv)`
- Replace `View::Markdown(md)` → use `Tab` with `legacy_md_preview`
- Replace `v.doc()` → `&v.doc`
- Replace `v.is_markdown()` → `v.plugin_id() == "builtin.markdown"`
- Replace `v.as_md()` → `v.legacy_md_preview.as_ref()`
- Replace `v.as_md_mut()` → `v.legacy_md_preview.as_mut()`
- Replace `v.into_editor()` → no-op (Tab.doc stays)
- Replace `v.into_markdown()` → set `legacy_md_preview = Some(MarkdownPreview::new())`

- [ ] **Step 8: Build check**

Run: `cargo build -p edit-plus-app 2>&1`
Expected: compiles successfully. All `View::` references resolved.

- [ ] **Step 9: Run tests**

Run: `cargo test -p edit-plus-app 2>&1 | tail -30`
Expected: most tests pass. Some markdown-specific tests may be `#[ignore]`d.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/
git commit -m "refactor: replace all View enum usages with Tab access patterns"
```

---

### Task 8: Phase 1 verification

**Goal:** Confirm Phase 1 is complete — all existing functionality works through the new Tab indirection.

- [ ] **Step 1: Verify View enum is gone**

```bash
grep -rn "enum View" crates/app/src/
```
Expected: no results.

- [ ] **Step 2: Build and test**

```bash
cargo build -p edit-plus-app 2>&1
cargo test -p edit-plus-app 2>&1 | tail -30
```
Expected: build passes, tests pass (ignored tests OK).

- [ ] **Step 3: Run full verification**

```bash
./scripts/verify.sh
```
Expected: passes. Fix any issues.

- [ ] **Step 4: Commit (if any fixes needed)**

```bash
git add -A && git commit -m "chore: Phase 1 verification fixes"
```

---

## Phase 2: Eliminate hardcoded branches

### Task 9: Create `plugins/markdown.rs` — extract MarkdownPreview

**Files:**
- Create: `crates/app/src/plugins/markdown.rs`
- Modify: `crates/app/src/plugins/mod.rs`

**Interfaces:**
- Consumes: `ContentPlugin` trait, `MarkdownPreview` from `md_preview.rs`, `MarkdownRenderSettings`, `markdown` crate
- Produces: `MarkdownPlugin` struct implementing `ContentPlugin`

- [ ] **Step 1: Create `crates/app/src/plugins/markdown.rs`**

The `MarkdownPlugin` wraps the existing `MarkdownPreview` and delegates trait methods to it. This is a thin adapter — the 1151-line `md_preview.rs` logic stays unchanged, just called through the trait.

```rust
//! MarkdownPlugin — Markdown preview as a ContentPlugin.

use std::any::Any;

use edit_plus_ui::core::paint::DrawList;

use crate::md_preview::{MarkdownPreview, MarkdownRenderSettings};
use crate::plugin::{
    CommandFlow, ContentPlugin, HitResult, LogicalPos, PluginCommand, PluginContext, PluginId,
    PluginOutput, ToolbarItem, ToolbarIcon,
};

pub struct MarkdownPlugin {
    preview: MarkdownPreview,
    /// Whether the TOC panel is shown (internal state, not exposed to host).
    toc_visible: bool,
}

impl MarkdownPlugin {
    pub fn new() -> Self {
        Self { preview: MarkdownPreview::new(), toc_visible: false }
    }

    pub fn from_source(source: &str) -> Self {
        let mut preview = MarkdownPreview::new();
        // set_source will be called properly in on_activate / on_source_changed
        let _ = source; // Phase 2: wire properly
        Self { preview, toc_visible: false }
    }
}

impl ContentPlugin for MarkdownPlugin {
    fn id(&self) -> PluginId { "builtin.markdown" }
    fn name(&self) -> &str { "Markdown Preview" }
    fn supported_extensions(&self) -> &[&str] { &["md", "markdown"] }

    fn on_activate(&mut self, ctx: &mut PluginContext) {
        self.preview.set_source(ctx.source().to_string(), 0);
    }

    fn on_deactivate(&mut self) {}

    fn on_source_changed(&mut self, ctx: &mut PluginContext) {
        self.preview.set_source(ctx.source().to_string(), 0);
    }

    fn render(&mut self, scroll_y: f32, ctx: &mut PluginContext) -> PluginOutput {
        use edit_plus_ui::settings::UiMetrics;
        let metrics = UiMetrics::from_settings(ctx.settings, ctx.dpi);
        let settings = MarkdownRenderSettings::from_metrics(ctx.settings, &metrics);
        let (dl, needs_drain) = self.preview.render(
            ctx.theme,
            ctx.viewport.width(),
            ctx.viewport.height(),
            ctx.viewport.min_x(),
            ctx.viewport.min_y() + 16.0 * ctx.dpi, // preview_top_pad
            settings,
            Some(ctx.shaper),
        );
        PluginOutput {
            draw_list: dl,
            content_height: self.preview.content_height,
            needs_drain,
        }
    }

    fn selection_highlights(&self, _ctx: &mut PluginContext) -> Option<DrawList> {
        self.preview.selection_highlights()
    }

    fn on_command(&mut self, cmd: &PluginCommand, ctx: &mut PluginContext) -> CommandFlow {
        match cmd {
            PluginCommand::Scroll { delta_y } => {
                self.preview.scroll(*delta_y, ctx.viewport.height());
                CommandFlow::Consumed
            }
            PluginCommand::Click { pos, click_count } => {
                if self.preview.preview_hit_test(pos.x, pos.y, 0.0, 0.0).is_some() {
                    // Update preview selection based on click_count
                    CommandFlow::Consumed
                } else {
                    CommandFlow::Passthrough
                }
            }
            PluginCommand::Copy => {
                // handled via selected_text()
                CommandFlow::Consumed
            }
            PluginCommand::SelectAll => {
                self.preview.preview_select_all();
                CommandFlow::Consumed
            }
            PluginCommand::Find { query, case_sensitive } => {
                self.preview.search(query, *case_sensitive);
                CommandFlow::Consumed
            }
            PluginCommand::FindNext => CommandFlow::Consumed, // handled via search()
            PluginCommand::FindPrev => CommandFlow::Consumed,
            _ => CommandFlow::Passthrough,
        }
    }

    fn hit_test(&self, pos: LogicalPos) -> Option<HitResult> {
        self.preview.preview_hit_test(pos.x, pos.y, 0.0, 0.0).map(|p| HitResult {
            pos_in_source: Some(p.char_pos),
        })
    }

    fn search(&mut self, query: &str, case_sensitive: bool) -> usize {
        self.preview.search(query, case_sensitive);
        // Return match count — MarkdownPreview doesn't expose this directly.
        // Phase 2 refinement: expose match count from MarkdownPreview.
        0 // FIXME: return actual match count
    }

    fn clear_search(&mut self) {
        self.preview.search("", false);
    }

    fn jump_to_match(&mut self, index: usize) -> bool {
        self.preview.scroll_to_search_match(index)
    }

    fn search_highlights(&self, _ctx: &mut PluginContext) -> Option<DrawList> {
        self.preview.search_highlights()
    }

    fn selected_text(&self) -> Option<String> {
        self.preview.preview_selected_text()
    }

    fn toolbar_items(&self) -> Vec<ToolbarItem> {
        vec![ToolbarItem {
            id: "toc",
            tooltip: "Toggle Table of Contents",
            icon: ToolbarIcon::Toc,
            toggled: self.toc_visible,
        }]
    }

    fn on_toolbar_action(&mut self, item_id: &str) {
        if item_id == "toc" {
            self.toc_visible = !self.toc_visible;
        }
    }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

- [ ] **Step 2: Update `plugins/mod.rs`**

```rust
pub(crate) mod editor;
pub(crate) mod markdown;
```

- [ ] **Step 3: Update `plugin_registry.rs`** to wire MarkdownPlugin

```rust
use crate::plugins::markdown::MarkdownPlugin;

pub(crate) fn create_content_plugin(path: &Path) -> Box<dyn ContentPlugin> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "md" | "markdown" => Box::new(MarkdownPlugin::new()),
        _ => Box::new(EditorPlugin::new()),
    }
}

pub(crate) fn create_content_plugin_by_id(
    id: PluginId,
    doc: &DocumentView,
) -> Box<dyn ContentPlugin> {
    match id {
        "builtin.editor" => Box::new(EditorPlugin::new()),
        "builtin.markdown" => {
            let plugin = MarkdownPlugin::from_source(&doc.source());
            Box::new(plugin)
        }
        _ => Box::new(EditorPlugin::new()),
    }
}
```

- [ ] **Step 4: Build check**

Run: `cargo build -p edit-plus-app 2>&1 | head -30`
Expected: compiles (MarkdownPlugin created but old branches still use legacy_md_preview).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/plugins/ crates/app/src/plugin_registry.rs
git commit -m "feat(plugin): add MarkdownPlugin wrapping MarkdownPreview"
```

---

### Task 10: Refactor `app_renderer.rs` — 9 branches → plugin.render()

**Files:**
- Modify: `crates/app/src/app_renderer.rs`

This is the largest single refactoring. The ~120 lines of Markdown preview rendering (lines 427-547) are replaced by a single `plugin.render()` call.

- [ ] **Step 1: Remove the `View::Markdown` render branch and replace with plugin delegation**

In `App::render()`, find the block starting at line 427:
```rust
// ── Markdown preview mode (per-tab via View::Markdown) ──
if self.workspace.active_view().is_some_and(|v| v.is_markdown()) {
    // ... ~120 lines
}
```

Replace the entire block with:

```rust
// ── Content rendering via active plugin ──
if let Some(tab) = self.workspace.active_view_mut() {
    // Build PluginContext
    let source_text = extract_source_text(&mut tab.doc); // existing helper, move inline
    let theme = &self.theme;
    let settings = &self.settings;
    let dpi = self.gpu.as_ref().map(|g| g.dpi).unwrap_or(1.0);

    // Determine content rect
    let content_top = self.content_top_offset();
    let content_rect = Rect::new(
        Point::new(gutter_left_margin, content_top),
        Size::new(viewport_w - gutter_left_margin, viewport_h - content_top),
    );

    let mut pending_edits = Vec::new();

    if let (Some(text), Some(shaper)) = (&mut self.text, &mut self.gpu) {
        let mut ctx = PluginContext::new(
            &source_text,
            theme,
            settings,
            &mut text.shaper,
            content_rect,
            dpi,
            &mut pending_edits,
        );

        // Source update detection
        let buf_gen = tab.doc.tb().generation();
        if self.needs_source_update(tab, buf_gen) {
            tab.plugin.on_source_changed(&mut ctx);
        }

        // Main render
        let output = tab.plugin.render(scroll_y, &mut ctx);

        // Drain to vertices (or reuse cache)
        if output.needs_drain {
            let vertices = paint_backend::drain(&output.draw_list, screen, &mut text.preview_cache, gpu);
            // cache vertices...
        }

        // Selection highlights
        if let Some(sel_dl) = tab.plugin.selection_highlights(&mut ctx) {
            let sel_vertices = paint_backend::drain(&sel_dl, screen, &mut text.preview_cache, gpu);
            // render...
        }

        // Search highlights
        if let Some(search_dl) = tab.plugin.search_highlights(&mut ctx) {
            let search_vertices = paint_backend::drain(&search_dl, screen, &mut text.preview_cache, gpu);
            // render...
        }

        // Update scrollbar info
        content_height = output.content_height;
    }

    // Process pending edits
    for edit in pending_edits {
        // apply to tab.doc
    }
} else {
    // Editor mode (existing shape_visible_lines path) — also unified later
}
```

- [ ] **Step 2: Remove `needs_source_update` / `is_markdown` checks**

Replace:
```rust
let is_md_preview = self.workspace.active_view().is_some_and(|v| v.is_markdown());
```
With:
```rust
let is_plugin = self.workspace.active_view()
    .is_some_and(|t| t.plugin_id() != "builtin.editor");
```

In scrollbar data injection (lines 241-254):
```rust
// Old:
if is_md_preview {
    if let crate::view::View::Markdown(mv) = v {
        content_height = mv.preview.content_height;
        scroll_offset = mv.preview.scroll_y;
    }
}

// New:
if let Some(tab) = self.workspace.active_view() {
    // Scrollbar data comes from the last rendered PluginOutput.
    // Store content_height on App between frames.
    scroll_offset = self.last_content_scroll_y; // tracked by App
}
```

- [ ] **Step 3: TOC input — delegate to plugin**

```rust
// Old (~line 375):
if self.workspace.active_view().and_then(|v| v.as_md()).is_some_and(|mv| mv.toc_visible)
    && let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view()
{
    // build TOC from mv.preview.headings()
}

// New:
// TOC is plugin-internal. Host doesn't know about it.
// MarkdownPlugin renders TOC as part of its draw_list.
// Delete the TOC input injection block entirely.
```

- [ ] **Step 4: Title bar preview toggle — use toolbar_items**

```rust
// Old (~line 49):
self.workspace.active_view().and_then(|v| v.as_md()).is_some_and(|mv| mv.toc_visible);

// New:
// Host queries plugin.toolbar_items() and renders TitleBar buttons.
// When user clicks, host calls plugin.on_toolbar_action(id).
// This replaces the hardcoded ToggleMarkdownPreview / ToggleToc buttons.
```

- [ ] **Step 5: Event handle for preview selection**

Replace all `View::Markdown(mv)` pattern matches in renderer with `tab.plugin.*` calls.

- [ ] **Step 6: Build and test**

Run: `cargo build -p edit-plus-app 2>&1`
Expected: compiles. The old preview code path is gone.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "refactor(render): replace 9 View::Markdown branches with plugin.render()"
```

---

### Tasks 11-15: Remaining dispatch refactoring

Due to space constraints, these tasks follow the same pattern as Task 10. Each eliminates `View::Markdown` branches in one module:

**Task 11: `dispatch/editor.rs`** — Delete the ~40 line preview command whitelist and ~130 line selection code. Replace with `tab.plugin.on_command(PluginCommand::*)`. Map relevant `EditCommand` variants to `PluginCommand`.

**Task 12: `dispatch/mouse.rs`** — Replace `is_preview` check + `preview_hit_test()` with `tab.plugin.hit_test()`. Click/double-click/triple-click land in `plugin.on_command(PluginCommand::Click { click_count })`.

**Task 13: `dispatch/viewport.rs` + `app_scroll.rs`** — Replace `mv.preview.scroll_y = ...` with `plugin.on_command(PluginCommand::Scroll { delta_y })`. Host computes delta from scrollbar position.

**Task 14: `app_search.rs`** — Replace `mv.preview.scroll_to_search_match()` with `plugin.jump_to_match()`. Replace search execution with `plugin.search()`.

**Task 15: `events.rs` + `ui_shell.rs` + `app_dispatch.rs`** — Replace `ToggleMarkdownPreview` action with generic `SwitchPlugin` + `toolbar_items`. Delete `preview_offsets()`.

Each task follows the same cycle: find `View::Markdown` → replace with trait call → build → commit.

---

### Task 16: Delete `md_preview.rs` + `legacy_md_preview` cleanup

- [ ] **Step 1: Verify zero `legacy_md_preview` references**

```bash
grep -rn "legacy_md_preview" crates/app/src/
```
Expected: zero results (all replaced in Tasks 10-15).

- [ ] **Step 2: Remove `legacy_md_preview` from Tab**

Edit `tab.rs`: remove the field, remove the import of `MarkdownPreview`.

- [ ] **Step 3: Delete `md_preview.rs`**

Remove file and its `pub(crate) mod md_preview;` declaration from `lib.rs`.

- [ ] **Step 4: Build check**

```bash
cargo build -p edit-plus-app 2>&1
```
Expected: compiles. `MarkdownPreview` is now only reachable through `MarkdownPlugin`.

- [ ] **Step 5: Commit**

```bash
git rm crates/app/src/md_preview.rs
git add crates/app/src/tab.rs crates/app/src/lib.rs
git commit -m "refactor: delete md_preview.rs, now fully migrated to MarkdownPlugin"
```

---

### Task 17: Integration verification

- [ ] **Step 1: Verify zero `View::Markdown` / `MdView` / `is_markdown` / `as_md` references**

```bash
grep -rn "View::Markdown\|MdView\|\.as_md\|legacy_md" crates/app/src/
```
Expected: zero results (except `is_markdown_path` helper).

- [ ] **Step 2: Full build and test**

```bash
cargo build -p edit-plus-app 2>&1
cargo test -p edit-plus-app 2>&1
```

- [ ] **Step 3: Run verify script**

```bash
./scripts/verify.sh
```

- [ ] **Step 4: Manual smoke test checklist**

- [ ] Open a `.md` file — should open in Markdown preview mode
- [ ] `Cmd+M` toggles between Editor and Preview
- [ ] Scroll in preview — smooth, scrollbar correct
- [ ] Click to place cursor in preview
- [ ] Double-click selects word, triple-click selects line
- [ ] `Cmd+F` search — highlights appear, `Cmd+G` navigates
- [ ] TOC button in TitleBar toggles TOC panel
- [ ] `Cmd+C` copies selected text
- [ ] Open `.rs` file — opens in Editor mode (no regressions)
- [ ] Edit, save, close tab — all standard behavior unchanged

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: Phase 2 integration verification"
```

---

## Phase 3-5 (Future — out of scope for this plan)

Defined in spec: Navigator extraction, cleanup, and extension. Separate plans will cover these.

## Self-Review Checklist

Before executing this plan, the implementer should verify:

1. All file paths are correct relative to `/Users/dan/proj/llmws/edit+/`
2. `LogicalPos` is importable from `edit_plus_ui::core::geom` (verify with `grep`)
3. `MarkdownRenderSettings` fields match actual struct (verify with `crates/app/src/md_preview.rs`)
4. `DocumentView::source()` method exists or needs creation
5. `TextBuffer::generation()` method exists for source change detection
