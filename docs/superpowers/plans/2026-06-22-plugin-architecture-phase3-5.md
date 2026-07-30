# Phase 3–5: Navigator 提取、清理与扩展 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract TabBar rendering/interaction into `TabBarNavigator` (implementing `Navigator` trait), audit and clean up transitional code, and prepare the architecture for future plugins.

**Architecture:** Phase 3 wraps existing `ui::widgets::tab_bar` inside a `TabBarNavigator` that implements the `Navigator` trait, moving tab scroll state from Workspace into the navigator. Phase 4 audits plugin ID checks and as_any casts, verifies performance. Phase 5 outlines extension paths.

**Tech Stack:** Rust, edit+ codebase (crates/app, crates/ui)

**Precondition:** Phase 1–2 complete — `View` enum removed, `ContentPlugin` trait active, all rendering through `plugin.render()`.

## Current State (baseline)

TabBar integration is spread across 8+ locations:

| Location | What it does |
|----------|-------------|
| `app_renderer.rs:306-337` | Builds `Vec<TabInfo>` from workspace tabs, injects into UiShell |
| `app_window.rs:19` `build_shell_inputs()` | Computes `tabs_visible` / `tabs_thickness` for Dock layout |
| `ui_shell.rs:650-780` `rebuild_dock_children()` | Adds TabBar as Dock child at top |
| `events.rs:translate_tab_action()` | Maps `TabBarAction` → `AppAction` |
| `app_scroll.rs:115-139` | Horizontal tab scroll on wheel |
| `dispatch/chrome.rs` | `ScrollTabLeft` / `ScrollTabRight` animation trigger |
| `workspace.rs:820-840` | `tab_scroll_offset` / `tab_scroll_target` / animation tick |
| `dispatch/tabs.rs` | `update_tab_layout()`, auto-scroll to active tab |

Sidebar integration follows the same pattern — SidebarWidget is already implemented as a full widget.

---

## Phase 3: Navigator 提取

### Task 18: Create `navigator.rs` — Navigator trait + shared types

**Files:**
- Create: `crates/app/src/navigator.rs`

**Interfaces:**
- Produces: `Navigator` trait, `NavContext`, `NavEntry`, `NavOutput`, `NavAction`

- [ ] **Step 1: Create `crates/app/src/navigator.rs`**

```rust
//! Navigator trait — abstracts tab/file navigation UI.
//!
//! TabBar and Sidebar are both navigators. The host (Workspace/App) owns the
//! open-file list; the navigator renders it and emits NavActions.

use std::any::Any;
use std::path::PathBuf;

use edit_plus_ui::core::geom::Rect;
use edit_plus_ui::core::paint::DrawList;
use edit_plus_ui::theme::Theme;

use crate::settings::Settings;

// ── Data types ──

/// Pure-data entry for one open file. Extracted from Tab — no DocumentView refs.
pub struct NavEntry {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub pinned: bool,
    pub language: String,
}

/// Read-only context the host provides each frame.
pub struct NavContext<'a> {
    pub open_tabs: &'a [NavEntry],
    pub active_index: usize,
    pub theme: &'a Theme,
    pub settings: &'a Settings,
    pub dpi: f32,
    pub screen_size_px: (f32, f32),
}

// ── Output ──

pub struct NavOutput {
    pub draw_list: DrawList,
    /// Actions accumulated during this frame's render + event handling.
    pub actions: Vec<NavAction>,
    /// Total scrollable width (for horizontal tab scrolling). 0.0 if not scrollable.
    pub max_scroll: f32,
}

// ── Actions ──

/// Actions the navigator emits. Host executes them.
pub enum NavAction {
    SwitchTo(usize),
    Close(usize),
    Open(PathBuf),
    NewEmpty,
    ContextMenu { tab_index: usize, anchor_px: (f32, f32) },
    HoverTab(Option<usize>),
    ScrollLeft,
    ScrollRight,
}

// ── Trait ──

pub trait Navigator: Any {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    /// Render the navigator UI. Called every frame.
    fn render(&mut self, rect: Rect, ctx: &NavContext) -> NavOutput;

    /// Hit-test a click within the navigator's rect.
    fn hit_test(&self, pos_x: f32, pos_y: f32) -> Option<NavAction>;

    /// Handle scroll within the navigator's rect. delta > 0 = scroll right.
    fn scroll(&mut self, delta: f32);

    /// Set the current hovered position (for tooltips, hover effects).
    /// pos is relative to navigator rect origin.
    fn hover(&mut self, pos_x: f32, pos_y: f32);

    /// Current scroll offset (for animation).
    fn scroll_offset(&self) -> f32;

    /// The navigator's natural thickness in pixels (height for top bar, width for sidebar).
    /// May depend on dpi.
    fn thickness(&self, dpi: f32) -> f32;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

- [ ] **Step 2: Add `pub(crate) mod navigator;` to `lib.rs`**

- [ ] **Step 3: Build check**

```bash
cargo build -p edit-plus-app 2>&1 | head -20
```
Expected: compiles (new module, not yet used).

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/navigator.rs crates/app/src/lib.rs
git commit -m "feat(navigator): add Navigator trait and shared types"
```

---

### Task 19: Create `navigators/tab_bar.rs` — TabBarNavigator

**Files:**
- Create: `crates/app/src/navigators/mod.rs`
- Create: `crates/app/src/navigators/tab_bar.rs`

**Interfaces:**
- Consumes: `Navigator` trait, `NavContext`, `NavEntry`, `NavOutput`, `NavAction` from `navigator.rs`; `TabBarWidget`, `TabBarWidgetInput`, `TabInfo` from `ui::widgets::tab_bar`
- Produces: `TabBarNavigator` struct implementing `Navigator`

- [ ] **Step 1: Create `crates/app/src/navigators/mod.rs`**

```rust
pub(crate) mod tab_bar;
```

- [ ] **Step 2: Create `crates/app/src/navigators/tab_bar.rs`**

```rust
//! TabBarNavigator — wraps ui::widgets::tab_bar as a Navigator implementation.

use std::any::Any;

use edit_plus_ui::core::geom::Rect;
use edit_plus_ui::widgets::tab_bar::{
    TabBarWidget, TabBarWidgetInput, TabInfo, tab_bar_height,
};

use crate::navigator::{NavAction, NavContext, NavEntry, NavOutput, Navigator};
use crate::settings::{Settings, UiMetrics};

pub struct TabBarNavigator {
    widget: TabBarWidget,
    scroll_offset: f32,
    pending_actions: Vec<NavAction>,
}

impl TabBarNavigator {
    pub fn new() -> Self {
        Self {
            widget: TabBarWidget::new(),
            scroll_offset: 0.0,
            pending_actions: Vec::new(),
        }
    }
}

impl Navigator for TabBarNavigator {
    fn id(&self) -> &str { "builtin.tab_bar" }
    fn name(&self) -> &str { "Tab Bar" }

    fn thickness(&self, dpi: f32) -> f32 {
        tab_bar_height(dpi)
    }

    fn scroll_offset(&self) -> f32 {
        self.scroll_offset
    }

    fn scroll(&mut self, delta: f32) {
        let max_scroll = self.widget.state().current_layout()
            .map(|l| l.max_scroll).unwrap_or(0.0);
        self.scroll_offset = (self.scroll_offset - delta).clamp(0.0, max_scroll);
    }

    fn hover(&mut self, pos_x: f32, pos_y: f32) {
        // TabBarWidget handles hover internally via on_event
    }

    fn render(&mut self, rect: Rect, ctx: &NavContext) -> NavOutput {
        self.widget.set_rect(rect);

        // NavEntry → TabInfo mapping
        let tabs: Vec<TabInfo> = ctx.open_tabs.iter().map(|e| TabInfo {
            title: e.title.clone(),
            file_path: e.file_path.clone(),
            is_dirty: e.is_dirty,
            pinned: e.pinned,
            language: e.language.clone(),
        }).collect();

        let metrics = UiMetrics::from_settings(ctx.settings, ctx.dpi);

        let input = TabBarWidgetInput {
            tabs,
            active_index: Some(ctx.active_index),
            back_enabled: false,
            forward_enabled: false,
            screen_size_px: ctx.screen_size_px,
            hovered_index: None,
            scroll_offset_px: self.scroll_offset,
            metrics,
        };
        self.widget.set_input(input);

        let draw_list = self.widget.paint();
        let max_scroll = self.widget.state().current_layout()
            .map(|l| l.max_scroll).unwrap_or(0.0);

        // Drain actions accumulated from widget events this frame
        let mut actions = std::mem::take(&mut self.pending_actions);

        // Auto-scroll to keep active tab visible
        if let Some(target) = self.widget.autoscroll_target() {
            self.scroll_offset = target;
        }

        NavOutput { draw_list, actions, max_scroll }
    }

    fn hit_test(&self, _pos_x: f32, _pos_y: f32) -> Option<NavAction> {
        // TabBar hit testing is done via Widget::on_event path in UiShell.
        // This method is for navigators that don't use the Widget system.
        None
    }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

- [ ] **Step 3: Add `pub(crate) mod navigators;` to `lib.rs`**

- [ ] **Step 4: Build check**

```bash
cargo build -p edit-plus-app 2>&1 | head -20
```
Expected: compiles (TabBarNavigator created, not yet wired in).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/navigators/ crates/app/src/lib.rs
git commit -m "feat(navigator): add TabBarNavigator wrapping TabBarWidget"
```

---

### Task 20: Wire TabBarNavigator into Workspace + App

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

**Strategy:** Replace Workspace's `tab_scroll_offset`/`tab_scroll_target` with `navigator: Box<dyn Navigator>`. Tab data injection now goes through `NavContext`. Scroll state lives in the navigator.

- [ ] **Step 1: Add navigator field to Workspace**

In `workspace.rs`, replace scroll fields:

```rust
// Remove:
pub(crate) tab_scroll_offset: f32,
pub(crate) tab_scroll_target: f32,

// Add:
pub(crate) navigator: Box<dyn Navigator>,
```

In `Workspace::new()`:
```rust
navigator: Box::new(crate::navigators::tab_bar::TabBarNavigator::new()),
```

Remove `start_scroll_animation()` and `tick_scroll_animation()` methods — animation becomes the navigator's responsibility.

- [ ] **Step 2: Update `app_renderer.rs` — Tab data injection**

Replace the Phase 6 tab bar injection block (lines 306-337) with NavEntry construction + navigator render:

```rust
// Phase 6: Navigator (TabBar / Sidebar)
let nav_entries: Vec<NavEntry> = self.workspace
    .tabs().iter().enumerate().map(|(i, t)| {
        let title = t.doc.file_path.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("untitled").to_string();
        NavEntry {
            title,
            file_path: t.doc.file_path.clone(),
            is_dirty: t.doc.dirty,
            pinned: self.workspace.pinned_indices().contains(&i),
            language: String::new(),
        }
    }).collect();

// Render navigator and process actions
let nav_rect = Rect { x: 0.0, y: 0.0, w: screen_w, h: self.workspace.navigator.thickness(dpi) };
let nav_output = self.workspace.navigator.render(nav_rect, &NavContext {
    open_tabs: &nav_entries,
    active_index: self.workspace.active_index(),
    theme: &self.theme,
    settings: &self.settings,
    dpi,
    screen_size_px: (screen_w, screen_h),
});
// Drain nav_output.draw_list to GPU
// Process nav_output.actions:
for action in nav_output.actions {
    self.dispatch_nav_action(action);
}
```

Remove the old `set_tabs_input()` call to UiShell.

- [ ] **Step 3: Remove TabBar from UiShell Dock**

In `ui_shell.rs`:
- Remove `tab_input_*` fields
- Remove `set_tabs_input()` method
- In `rebuild_dock_children()`, conditionally remove the TabBar Dock child (or keep for backward compat during transition — Phase 3 can keep the Dock child but have it be driven by navigator output)

Simplest approach: Keep the TabBar Dock child, but have it receive its input from the navigator's `nav_output.draw_list` rather than raw TabInfo. Actually, this gets complex because the Widget system and DrawList are different paths.

**Better approach for Phase 3**: The navigator renders independently in `app_renderer.rs` (its draw_list is drained alongside other overlays). The Dock no longer has a TabBar child. The navigator's rect is reserved at the top.

In `app_window.rs` `build_shell_inputs()`:
- `tabs_visible` and `tabs_thickness` remain but are computed from navigator state
- TabBar Dock child in `rebuild_dock_children()` becomes conditional: only added if using legacy path (which we're removing)

In `ui_shell.rs`:
- Keep the TabBar Dock child code for now (it's harmless — just won't be added since `tabs_visible = false` during transition, or we remove it)

- [ ] **Step 4: Update scroll handling**

In `app_scroll.rs`: Replace the tab bar scroll block (lines 115-139) with:

```rust
// Tab bar horizontal scroll via navigator
let tbh = self.current_tab_bar_height();
if tbh > 0.0 && (self.mouse.pos.1 as f32) < tbh {
    let dx = match delta {
        ScrollDelta::LineDelta(_, x) => x * 20.0 * dpi,
        ScrollDelta::PixelDelta(pos) => pos.x as f32,
    };
    self.workspace.navigator.scroll(dx);
    return AppEffect::REDRAW;
}
```

- [ ] **Step 5: Update action dispatch**

In `events.rs`: The `translate_tab_action()` function still works because TabBarWidget still produces `TabBarAction` through the Widget event system. But now the navigator also produces `NavAction` from its render. We need to handle both:

```rust
// In dispatch, add handler for NavAction:
fn dispatch_nav_action(&mut self, action: NavAction) -> AppEffect {
    match action {
        NavAction::SwitchTo(idx) => { self.workspace.switch_to(idx) }
        NavAction::Close(idx) => { self.workspace.close_tab(idx) }
        NavAction::NewEmpty => { /* open new tab */ }
        NavAction::ScrollLeft => { self.workspace.navigator.scroll(-50.0); }
        NavAction::ScrollRight => { self.workspace.navigator.scroll(50.0); }
        NavAction::ContextMenu { tab_index, anchor_px } => { /* show menu */ }
        NavAction::HoverTab(idx_opt) => { /* set cursor */ }
        NavAction::Open(path) => { /* open file */ }
    }
    AppEffect::REDRAW
}
```

- [ ] **Step 6: Update `dispatch/tabs.rs`**

Replace `workspace.tab_scroll_offset` / `workspace.tab_scroll_target` references with `workspace.navigator.scroll_offset()`.

Remove `update_tab_layout()` if it only served the old scroll system.

In `dispatch/chrome.rs`: Replace `ScrollTabLeft`/`ScrollTabRight` handling:
```rust
ChromeDispatchAction::ScrollTabLeft => {
    self.workspace.navigator.scroll(50.0);
}
ChromeDispatchAction::ScrollTabRight => {
    self.workspace.navigator.scroll(-50.0);
}
```

- [ ] **Step 7: Build and test**

```bash
cargo build -p edit-plus-app 2>&1
cargo test -p edit-plus-app 2>&1 | tail -20
```
Expected: compiles, tests pass. Fix any type mismatches.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/
git commit -m "refactor(navigator): wire TabBarNavigator into Workspace and App"
```

---

### Task 21: Event routing — unify Widget events through Navigator

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/ui_shell.rs`

**Goal:** TabBar widget events (clicks, hover) are processed by the navigator's internal widget, and resulting `NavAction`s are collected in `pending_actions` to be drained during render.

- [ ] **Step 1: Route TabBar Widget events to navigator**

In `ui_shell.rs` `dispatch()`: When a `WidgetAction::TabBar(ta)` is received, instead of returning it to the caller, inject it into the navigator:

```rust
// In UiShell::dispatch(), after dock.dispatch():
if let Some(action) = &result.action {
    if let WidgetAction::TabBar(ta) = action {
        // Translate TabBarAction → NavAction and push to navigator
        let nav_action = match ta {
            TabBarAction::SwitchTab(i) => NavAction::SwitchTo(*i),
            TabBarAction::CloseTab(i) => NavAction::Close(*i),
            TabBarAction::NewEmptyTab => NavAction::NewEmpty,
            TabBarAction::ScrollLeft => NavAction::ScrollLeft,
            TabBarAction::ScrollRight => NavAction::ScrollRight,
            TabBarAction::HoverTab(i) => NavAction::HoverTab(*i),
            TabBarAction::OpenContextMenuPx { tab_index, anchor_px } =>
                NavAction::ContextMenu { tab_index: *tab_index, anchor_px: *anchor_px },
            _ => return, // unhandled
        };
        // Push to navigator's pending actions
        // (navigator needs to be accessible from UiShell, or actions bubble up)
    }
}
```

Actually, this creates a coupling where UiShell needs to know about the navigator. Cleaner approach: keep the current flow where `translate_tab_action()` in `events.rs` maps `TabBarAction → AppAction`. The navigator's render already returns its own actions from the frame. The two paths coexist:

1. **Widget event path** (existing): `TabBarAction → AppAction` via `translate_tab_action()`. No change needed.
2. **Navigator render path** (new): `NavAction` returned from `navigator.render()`.

The key insight: the navigator renders through `app_renderer.rs` but receives input events through the existing Widget dispatch. The `pending_actions` vector bridges them.

- [ ] **Step 2: Keep existing translate_tab_action, add NavAction dispatch**

No changes to `events.rs` `translate_tab_action()`. The existing flow still works because `TabBarWidget` still produces `TabBarAction` through the Widget system.

In `app_renderer.rs`, after navigator.render(), process `nav_output.actions` with `dispatch_nav_action()`.

- [ ] **Step 3: Verify TabBar interactions still work**

```bash
cargo build -p edit-plus-app 2>&1
```
Build check only — manual testing needed for clicks, scroll, hover.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/
git commit -m "refactor(navigator): unify TabBar events through Navigator"
```

---

### Task 22: Phase 3 verification

- [ ] **Step 1: Verify navigator field exists on Workspace**

```bash
grep -n "navigator:" crates/app/src/workspace.rs
```
Expected: shows `navigator: Box<dyn Navigator>` field.

- [ ] **Step 2: Verify tab_scroll_offset removed from Workspace**

```bash
grep -n "tab_scroll" crates/app/src/workspace.rs
```
Expected: zero results (moved into navigator).

- [ ] **Step 3: Full build + test + verify**

```bash
cargo build -p edit-plus-app 2>&1
cargo test -p edit-plus-app 2>&1 | tail -20
./scripts/verify.sh
```
Expected: all pass.

- [ ] **Step 4: Manual smoke test**

- [ ] TabBar renders correctly (titles, dirty dots, active highlight)
- [ ] Click tab → switches
- [ ] Close button → closes tab
- [ ] Horizontal scroll on tab bar overflow
- [ ] Right-click context menu
- [ ] "+" button → new tab
- [ ] Sidebar still works (if not yet converted to SidebarNavigator)

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: Phase 3 verification"
```

---

## Phase 4: 清理与稳定

### Task 23: Audit `as_any` / `as_any_mut` casts

**Goal:** Ensure no code bypasses the `ContentPlugin` or `Navigator` traits via downcasting.

- [ ] **Step 1: Find all as_any/as_any_mut calls**

```bash
grep -rn "as_any\(\)\|as_any_mut\(\)\|downcast_ref\|downcast_mut" crates/app/src/ --include="*.rs" | grep -v test | grep -v "fn as_any\|fn as_any_mut"
```

- [ ] **Step 2: For each result, classify**

| Pattern | Verdict |
|---------|---------|
| Plugin-internal (e.g., `MarkdownPlugin` casts itself) | OK — plugin owns its type |
| Host downcasts to `MarkdownPlugin` to access `toc_visible` | BAD — replace with trait method or `toolbar_items().toggled` |
| Host downcasts for testing | OK in `#[cfg(test)]` |

- [ ] **Step 3: Fix violations**

For each BAD cast, add the needed accessor to the trait or route through existing trait methods.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/
git commit -m "refactor: audit and clean as_any downcasts"
```

---

### Task 24: Audit `plugin.id()` runtime checks

**Goal:** Eliminate runtime plugin ID checks that are equivalent to the old `View::Markdown` branching.

- [ ] **Step 1: Find all plugin.id() comparisons**

```bash
grep -rn 'plugin\.id()\|plugin_id()' crates/app/src/ --include="*.rs" | grep -v test | grep -v "fn id\|fn plugin_id"
```

- [ ] **Step 2: Classify each usage**

| Pattern | OK? |
|---------|-----|
| `plugin.id() == "builtin.markdown"` in dispatch/render | BAD — same as old `View::Markdown` |
| `plugin.id() == "builtin.editor"` to check if editing allowed | BAD — use `plugin.allows_editing()` |
| `plugin.id()` in registry `create_content_plugin` | OK — that's the registry's job |
| `plugin.id()` in tests | OK |

- [ ] **Step 3: Replace violations**

Replace `tab.plugin_id() == "builtin.markdown"` with trait method calls. If a needed capability isn't on the trait, add it (e.g., `fn has_preview_mode(&self) -> bool`).

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/
git commit -m "refactor: eliminate plugin.id() runtime checks"
```

---

### Task 25: Performance baseline

**Goal:** Confirm no performance regression from the trait-based architecture.

- [ ] **Step 1: Benchmark frame times**

Use the app's built-in FPS counter or instrument `App::render()` with `std::time::Instant`:

```rust
let frame_start = std::time::Instant::now();
// ... render ...
let frame_us = frame_start.elapsed().as_micros();
// Log every 60 frames
```

- [ ] **Step 2: Compare scenarios**

| Scenario | Expected |
|----------|---------|
| Open 1 `.rs` file, idle | < 2ms |
| Open 1 `.md` file, idle preview | < 5ms |
| Scroll `.md` preview rapidly | < 8ms |
| Switch tabs rapidly | < 3ms |

- [ ] **Step 3: Profile if regression > 5%**

Use `cargo flamegraph` or `instruments` on macOS to identify hotspots.

- [ ] **Step 4: Document results**

Add a section to the spec doc: "Performance Verification" with frame times.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-06-22-plugin-architecture-design.md
git commit -m "docs: add performance verification results"
```

---

### Task 26: Spec doc finalization

**Goal:** Mark the design spec as "implemented".

- [ ] **Step 1: Update spec status**

In `docs/superpowers/specs/2026-06-22-plugin-architecture-design.md`, add at top:

```markdown
> **Status: Implemented** (Phase 1–4 complete, Phase 5 ongoing)
> 
> Implementation plan: `docs/superpowers/plans/2026-06-22-plugin-architecture-implementation.md`
> Phase 3–5 plan: `docs/superpowers/plans/YYYY-MM-DD-plugin-architecture-phase3-5.md`
```

- [ ] **Step 2: Final grep — zero View::Markdown**

```bash
grep -rn "View::Markdown\|MdView\|enum View" crates/app/src/ --include="*.rs"
```
Expected: zero results.

- [ ] **Step 3: Run verify.sh one final time**

```bash
./scripts/verify.sh
```

- [ ] **Step 4: Commit**

```bash
git add docs/ && git commit -m "docs: mark plugin architecture spec as implemented"
```

---

## Phase 5: 扩展路径（后续独立计划，此处仅概述）

### SidebarNavigator

当前 `SidebarWidget` 已完整实现。提取为 `SidebarNavigator` 的步骤：

1. 创建 `crates/app/src/navigators/sidebar.rs`
2. `SidebarNavigator` 包装 `SidebarWidget`，实现 `Navigator` trait
3. `thickness()` 返回 `sidebar_config.width`
4. `render()` 中 NavEntry → SidebarWidgetInput 映射（当前 `app_renderer.rs` Phase 7 的逻辑）
5. Dock 中的 Sidebar child 替换为 navigator 渲染

### 新 ContentPlugin 示例

- **MindMapPlugin**：`.xmind` 文件 → 脑图渲染（第三方库 + 自定义 Canvas 渲染）
- **NovelReaderPlugin**：`.txt` 文件 → 小说分页阅读模式（大字体、翻页动画）
- **DiffPlugin**：`.diff` / `.patch` → 并排 diff 视图

### 用户配置

在 Settings 中添加：

```rust
pub struct PluginPreferences {
    /// Default plugin per file extension. Falls back to registry default.
    pub extension_overrides: HashMap<String, PluginId>,
    /// Auto-activate plugin on file open (vs. manual toggle).
    pub auto_activate: bool,
}
```

---

## File Change Summary (Phase 3–4)

### New
| File | Purpose |
|------|---------|
| `crates/app/src/navigator.rs` | `Navigator` trait + `NavContext` + `NavEntry` + `NavOutput` + `NavAction` |
| `crates/app/src/navigators/mod.rs` | Navigator module entry |
| `crates/app/src/navigators/tab_bar.rs` | `TabBarNavigator` |

### Modified
| File | Change |
|------|--------|
| `workspace.rs` | Remove `tab_scroll_offset`/`tab_scroll_target`; add `navigator: Box<dyn Navigator>`; remove scroll animation methods |
| `app_renderer.rs` | Replace Phase 6 tab injection with `NavEntry` → `navigator.render()`; add `dispatch_nav_action()` |
| `ui_shell.rs` | Remove `tab_input_*` fields and `set_tabs_input()` |
| `app_scroll.rs` | Replace direct `tab_scroll_offset` manipulation with `navigator.scroll()` |
| `events.rs` | Keep `translate_tab_action()` (Widget path still works); add NavAction dispatch |
| `dispatch/chrome.rs` | `ScrollTabLeft`/`ScrollTabRight` → `navigator.scroll()` |
| `dispatch/tabs.rs` | Replace `tab_scroll_offset` refs with `navigator.scroll_offset()` |
| `lib.rs` | Add `mod navigator;` + `mod navigators;` |

### No Changes
| Layer | Reason |
|-------|--------|
| `crates/ui/widgets/tab_bar/` | Wrapped, not modified — pure UI component stays pure |
| `crates/ui/widgets/sidebar/` | Not touched in Phase 3–4 |
| `crates/ui/core/dock.rs` | Dock still used for other chrome; TabBar just exits the Dock |
