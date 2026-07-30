# Navigator Trait v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split Navigator trait into pure-data navigation interface; move scroll/animation into TabBarWidget + App::SmoothScroll; rename Tab→DocItem; delete TabBarNavigator wrapper.

**Architecture:** Navigator trait exposes only items/active_index/switch_to/close/toggle_pin. TabBarWidget owns scroll_target and autoscroll calculation. App provides SmoothScroll interpolation. Workspace is a concrete Navigator implementation.

**Tech Stack:** Rust 1.93, winit 0.30, wgpu

## Global Constraints

- `crates/ui` must not depend on `crates/app` (architectural red line)
- `ui::tab_bar::TabBarWidget` public API stays stable
- `ContentPlugin` trait unaffected
- Persistence format (PersistedWorkspace, PersistedTab) unaffected
- All changes must compile and pass 821 existing tests

## Dependency Graph

```
Task 1 (rename) ─────────────────────────────────────┐
                                                      ├──→ Task 4 (delete old + rewrite trait)
Task 2 (TabBarWidget scroll_target, ui crate) ──┐     │
                                                 ├──→ Task 3 (UiShell bridge + App SmoothScroll + replace call sites)
Task 3 (no deps on 2 if done in right order) ───┘              │
                                                               └──→ Task 5 (Workspace impl Navigator + NavEffect)
                                                                             │
                                                                             └──→ Task 6 (verify)
```

Key ordering constraint: TabBarNavigator references old Navigator trait. We must replace all navigator call sites (Task 3) BEFORE deleting TabBarNavigator (Task 4). Task 4 combines trait rewrite + deletion to keep each commit compilable.

---

### Task 1: Rename Tab → DocItem

**Deps:** none

**Files:**
- Modify: `src/tab.rs`
- Modify: `src/workspace.rs`
- Modify: `src/dispatch/tabs.rs`
- Modify: `src/app_renderer.rs`
- Modify: `src/app_scroll.rs`
- Modify: `src/dispatch/editor.rs`
- Modify: `src/dispatch/mouse.rs`
- Modify: `src/dispatch/chrome.rs`
- Modify: `src/app_window.rs`

**Produces:** `DocItem` struct, `Workspace.entries`, renamed accessors

- [ ] **Step 1: Rewrite src/tab.rs — Tab → DocItem + add doc_title()**

Replace entire file:
```rust
//! DocItem —— 文档条目，包装 DocumentView + ContentPlugin。

use std::path::PathBuf;

use crate::document_view::DocumentView;
use crate::plugin::ContentPlugin;
use crate::plugin_registry::PluginRegistry;
use crate::preview_plugin::PreviewPlugin;

/// 单个文档条目的完整状态。
pub(crate) struct DocItem {
    pub doc: DocumentView,
    pub plugin: Box<dyn ContentPlugin>,
}

impl DocItem {
    pub(crate) fn new_editor(doc: DocumentView) -> Self {
        Self { doc, plugin: PluginRegistry::create_editor() }
    }

    pub(crate) fn new_markdown(doc: DocumentView) -> Self {
        Self { doc, plugin: PluginRegistry::create_markdown() }
    }

    pub(crate) fn file_path(&self) -> Option<&PathBuf> {
        self.doc.file_path.as_ref()
    }

    pub(crate) fn dirty(&self) -> bool {
        self.doc.dirty
    }

    pub(crate) fn doc_title(&self) -> String {
        self.doc.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string()
    }

    pub(crate) fn preview_ref(&self) -> Option<&dyn PreviewPlugin> {
        self.plugin.preview_ref()
    }

    pub(crate) fn preview(&mut self) -> Option<&mut dyn PreviewPlugin> {
        self.plugin.preview()
    }

    pub(crate) fn toc_visible(&self) -> bool {
        self.plugin.toc_visible()
    }

    pub(crate) fn set_toc_visible(&mut self, visible: bool) {
        self.plugin.set_toc_visible(visible);
    }
}
```

Run: `cargo check 2>&1 | head -5`
Expected: compilation errors — callers still use `Tab`.

- [ ] **Step 2: Bulk rename in workspace.rs**

```bash
# Tab → DocItem (preserve PersistedTab)
sed -i '' 's/\([^D]\)Tab\([^B]\)/\1DocItem\2/g' src/workspace.rs
sed -i '' 's/PersistedDocItem/PersistedTab/g' src/workspace.rs

# fields/methods
sed -i '' 's/\.tabs\b/.entries/g' src/workspace.rs
sed -i '' 's/self\.tab_history/self.entry_history/g' src/workspace.rs
sed -i '' 's/pub(crate) tab_history/pub(crate) entry_history/g' src/workspace.rs
sed -i '' 's/ fn tab(/ fn entry(/g' src/workspace.rs
sed -i '' 's/ fn tab_mut(/ fn entry_mut(/g' src/workspace.rs
sed -i '' 's/ fn tabs(/ fn entries(/g' src/workspace.rs
sed -i '' 's/ fn active_tab(/ fn active_entry(/g' src/workspace.rs
sed -i '' 's/ fn active_tab_mut(/ fn active_entry_mut(/g' src/workspace.rs
sed -i '' 's/ fn push_tab(/ fn push_entry(/g' src/workspace.rs
sed -i '' 's/ fn push_tab_for_test(/ fn push_entry_for_test(/g' src/workspace.rs
sed -i '' 's/lazy_load_tab/lazy_load_entry/g' src/workspace.rs
sed -i '' 's/close_tab_inner/close_entry_inner/g' src/workspace.rs
sed -i '' 's/try_close_tab/try_close_entry/g' src/workspace.rs
sed -i '' 's/close_tab/close_entry/g' src/workspace.rs
sed -i '' 's/local_tabs/local_entries/g' src/workspace.rs
sed -i '' 's/new_empty_tab_with_viewport/new_untitled/g' src/workspace.rs
```

Run: `cargo check 2>&1 | head -10`
Expected: fewer errors, mostly in non-workspace files.

- [ ] **Step 3: Fix remaining callers**

In each file below, replace old names with new:
- `Tab::new_editor` → `DocItem::new_editor`
- `Tab::new_markdown` → `DocItem::new_markdown`
- `.tabs()` → `.entries()`
- `.active_tab()` → `.active_entry()`
- `.active_tab_mut()` → `.active_entry_mut()`
- `.tab(` → `.entry(`
- `.tab_mut(` → `.entry_mut(`
- `.push_tab(` → `.push_entry(`
- `new_empty_tab_with_viewport` → `new_untitled`

Files to edit:
- `src/dispatch/tabs.rs`
- `src/app_renderer.rs`
- `src/app_scroll.rs`
- `src/dispatch/editor.rs`
- `src/dispatch/mouse.rs`
- `src/dispatch/chrome.rs`
- `src/app_window.rs`

Run: `cargo check 2>&1`
Expected: compilation passes.

- [ ] **Step 4: Commit**

```bash
git add src/tab.rs src/workspace.rs src/dispatch/tabs.rs src/app_renderer.rs src/app_scroll.rs src/dispatch/editor.rs src/dispatch/mouse.rs src/dispatch/chrome.rs src/app_window.rs
git commit -m "refactor: rename Tab→DocItem, tabs→entries, new_empty_tab_with_viewport→new_untitled"
```

---

### Task 2: TabBarWidget adds scroll_target + scroll_by

**Deps:** none (ui crate, no app deps)

**Files:**
- Modify: `../ui/src/widgets/tab_bar/state.rs`
- Modify: `../ui/src/widgets/tab_bar/widget.rs`

**Produces:** TabBarWidget manages its own scroll target internally

- [ ] **Step 1: Add scroll_target to TabBarState**

Read current `TabBarState` struct:
```bash
grep -n "pub struct TabBarState" ../ui/src/widgets/tab_bar/state.rs
```

Add `scroll_target` field after `scroll_offset`:
```rust
pub struct TabBarState {
    layout: Option<TabBarLayout>,
    scroll_offset: f32,
    scroll_target: f32,
    hovered_index: Option<usize>,
    preview_index: Option<usize>,
    open_menu: Option<crate::widgets::popup_menu::PopupMenu>,
}
```

Update `Default` impl — add `scroll_target: 0.0`.

Add methods to `impl TabBarState`:
```rust
/// 用户滚动输入。clamp 到 [0, max_scroll]。
pub fn scroll_by(&mut self, delta: f32) {
    let max = self.layout.as_ref().map(|l| l.max_scroll).unwrap_or(0.0);
    self.scroll_target = (self.scroll_target - delta).clamp(0.0, max);
}

/// 当前滚动目标（供外部动画驱动读取）。
pub fn scroll_target(&self) -> f32 {
    self.scroll_target
}

/// 直接设置滚动目标（autoscroll 用）。
pub fn set_scroll_target(&mut self, target: f32) {
    self.scroll_target = target;
}
```

Note: `set_scroll_offset` already exists for `scroll_offset` — do NOT remove it, it's still used by `set_input` to set the CURRENT rendered position. The new field `scroll_target` is the DESIRED position.

- [ ] **Step 2: Expose from TabBarWidget + autoscroll in set_input**

Read current `set_input` code:
```bash
grep -n "pub fn set_input" ../ui/src/widgets/tab_bar/widget.rs
```

Add public methods to `impl TabBarWidget`:
```rust
pub fn scroll_by(&mut self, delta: f32) {
    self.state.scroll_by(delta);
}

pub fn scroll_target(&self) -> f32 {
    self.state.scroll_target()
}
```

In `set_input`, add autoscroll AFTER the layout call (at end of method, before `self.input = Some(input)`):
```rust
// Autoscroll: keep active tab visible
if let Some(active_idx) = input.active_index {
    let current = self.state.scroll_offset;
    if let Some((target, _)) = self.autoscroll_target(active_idx, current) {
        self.state.set_scroll_target(target);
    }
}
```

Run: `cargo check 2>&1`
Expected: passes (ui crate independent of app).

- [ ] **Step 3: Commit**

```bash
git add ../ui/src/widgets/tab_bar/state.rs ../ui/src/widgets/tab_bar/widget.rs
git commit -m "feat: add scroll_target + scroll_by to TabBarWidget with per-frame autoscroll"
```

---

### Task 3: UiShell bridge + App SmoothScroll + replace navigator call sites

**Deps:** Task 2 (TabBarWidget has scroll_by/scroll_target)

**Files:**
- Create: `src/smooth_scroll.rs`
- Modify: `src/lib.rs` — add `mod smooth_scroll;`
- Modify: `src/app.rs` — add fields
- Modify: `src/app_init.rs` — init SmoothScroll
- Modify: `src/ui_shell.rs` — add `tab_bar_scroll_by`, `tab_bar_scroll_target`
- Modify: `src/app_renderer.rs` — replace navigator.scroll_offset/tick
- Modify: `src/app_window.rs` — replace navigator.is_animating
- Modify: `src/app_scroll.rs` — replace navigator.scroll
- Modify: `src/dispatch/chrome.rs` — replace navigator.scroll
- Modify: `src/dispatch/tabs.rs` — replace navigator.is_animating

**Produces:** All navigator scroll/animation calls replaced; old Navigator trait unreferenced for UI

- [ ] **Step 1: Create SmoothScroll**

Create `src/smooth_scroll.rs`:
```rust
//! 通用平滑滚动插值器。App 层工具，TabBar/Sidebar 共用。

pub(crate) struct SmoothScroll {
    offset: f32,
    target: f32,
}

impl SmoothScroll {
    pub fn new() -> Self { Self { offset: 0.0, target: 0.0 } }
    pub fn current(&self) -> f32 { self.offset }
    pub fn target(&self) -> f32 { self.target }
    pub fn set_target(&mut self, t: f32) { self.target = t; }

    /// 每帧调用。返回 true 表示还在动画中。
    pub fn tick(&mut self) -> bool {
        let diff = self.target - self.offset;
        if diff.abs() < 0.5 {
            self.offset = self.target;
            return false;
        }
        self.offset += diff * 0.35;
        true
    }

    pub fn is_animating(&self) -> bool {
        (self.target - self.offset).abs() >= 0.5
    }
}
```

Add to `src/lib.rs`:
```rust
mod smooth_scroll;
```

- [ ] **Step 2: Add fields to App + init**

Edit `src/app.rs`, add fields (after `sidebar_animating`):
```rust
    /// Tab bar smooth-scroll animation.
    pub(crate) tab_scroll: crate::smooth_scroll::SmoothScroll,
    /// Preview entry index (managed by plugin, not pure navigation).
    pub(crate) preview_index: Option<usize>,
```

Edit `src/app_init.rs`, add initialization (near other `Instant::now()` calls):
```rust
            tab_scroll: crate::smooth_scroll::SmoothScroll::new(),
            preview_index: None,
```

- [ ] **Step 3: Add UiShell bridge methods**

Edit `src/ui_shell.rs`. Find the existing `compute_autoscroll_target` method. After it, add:
```rust
/// 用户滚动标签栏（鼠标滚轮 / 快捷键）。
pub(crate) fn tab_bar_scroll_by(&mut self, delta: f32) {
    for child in &mut self.dock.children {
        if let Some(tbw) = child.widget.as_any_mut().downcast_mut::<ui::tab_bar::TabBarWidget>() {
            tbw.scroll_by(delta);
            return;
        }
    }
}

/// 当前标签栏滚动目标（供 App 读去做动画）。
pub(crate) fn tab_bar_scroll_target(&self) -> f32 {
    for child in &self.dock.children {
        if let Some(tbw) = child.widget.as_any().downcast_ref::<ui::tab_bar::TabBarWidget>() {
            return tbw.scroll_target();
        }
    }
    0.0
}
```

- [ ] **Step 4: Replace call site — app_renderer.rs**

Find `self.workspace.navigator.scroll_offset()` (currently around line 320). Replace with:
```rust
self.tab_scroll.current(),
```

Find the `tick` block (currently around line 835):
```rust
if self.workspace.navigator.tick() {
    self.needs_redraw = true;
}
```

Replace with (place after tab bar render section):
```rust
// Sync animation target from TabBarWidget after render
self.tab_scroll.set_target(self.ui_shell.tab_bar_scroll_target());

if self.tab_scroll.tick() {
    self.needs_redraw = true;
}
```

- [ ] **Step 5: Replace call site — app_window.rs**

Find `self.workspace.navigator.is_animating()`. Replace with:
```rust
self.tab_scroll.is_animating() || self.sidebar_animating
```

- [ ] **Step 6: Replace call site — app_scroll.rs**

Find `self.workspace.navigator.scroll(dx);`. Replace with:
```rust
self.ui_shell.tab_bar_scroll_by(dx);
```

- [ ] **Step 7: Replace call site — dispatch/chrome.rs**

Find `self.workspace.navigator.scroll(delta);`. Replace with:
```rust
self.ui_shell.tab_bar_scroll_by(delta);
```

- [ ] **Step 8: Replace call site — dispatch/tabs.rs**

Find `self.workspace.navigator.is_animating()`. Replace with:
```rust
self.tab_scroll.is_animating()
```

- [ ] **Step 9: Verify compilation**

Run: `cargo check 2>&1`
Expected: passes — zero references to old Navigator trait methods for scroll/animation. (TabBarNavigator still exists but none of its methods are called.)

- [ ] **Step 10: Commit**

```bash
git add src/smooth_scroll.rs src/lib.rs src/app.rs src/app_init.rs src/ui_shell.rs src/app_renderer.rs src/app_window.rs src/app_scroll.rs src/dispatch/chrome.rs src/dispatch/tabs.rs
git commit -m "feat: add SmoothScroll + UiShell bridge, replace all navigator scroll/animation calls"
```

---

### Task 4: Delete TabBarNavigator + rewrite Navigator trait

**Deps:** Task 3 (no more references to old Navigator methods)

**Files:**
- Delete: `src/navigators/tab_bar.rs`
- Delete: `src/navigators/mod.rs`
- Delete: `src/navigators/` directory
- Modify: `src/navigator.rs` — rewrite to pure data interface
- Modify: `src/lib.rs` — remove `mod navigators;`

**Produces:** Clean Navigator trait; TabBarNavigator gone

- [ ] **Step 1: Delete navigators/ directory**

```bash
rm src/navigators/tab_bar.rs
rm src/navigators/mod.rs
rmdir src/navigators/
```

Edit `src/lib.rs`: remove `pub(crate) mod navigators;`.

- [ ] **Step 2: Rewrite src/navigator.rs**

```rust
//! Navigator trait — 纯数据导航接口。
//!
//! 条目集合 + 激活项切换。不涉及渲染、滚动、命中测试。
//! Workspace 是默认实现。

use std::any::Any;
use std::collections::HashSet;
use std::path::PathBuf;

/// 条目的 UI 投影，不引用 DocumentView。
#[derive(Debug, Clone)]
pub struct NavEntry {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub pinned: bool,
}

/// 导航操作效果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavEffect {
    None,
    ActiveChanged,
    ItemsChanged,
}

impl NavEffect {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (NavEffect::ActiveChanged, _) | (_, NavEffect::ActiveChanged) => NavEffect::ActiveChanged,
            (NavEffect::ItemsChanged, _) | (_, NavEffect::ItemsChanged) => NavEffect::ItemsChanged,
            _ => NavEffect::None,
        }
    }
}

pub trait Navigator: Any {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    fn items(&self) -> Vec<NavEntry>;
    fn len(&self) -> usize { self.items().len() }
    fn active_index(&self) -> usize;

    fn switch_to(&mut self, index: usize) -> NavEffect;
    fn close(&mut self, index: usize) -> NavEffect;

    fn toggle_pin(&mut self, index: usize) -> NavEffect;
    fn is_pinned(&self, index: usize) -> bool;
    fn pinned_indices(&self) -> &HashSet<usize>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

- [ ] **Step 3: Verify clean compile**

Run: `cargo check 2>&1`
Expected: errors — `Workspace.navigator` field still exists but `Navigator` trait no longer has the methods it used. Workspace code referencing `navigator` needs fixing in Task 5.

- [ ] **Step 4: Commit**

```bash
git add -u src/navigators/ src/navigator.rs src/lib.rs
git commit -m "refactor: delete TabBarNavigator, rewrite Navigator trait to pure data interface"
```

---

### Task 5: Workspace implements Navigator + WorkspaceEffect → NavEffect

**Deps:** Task 4 (new Navigator trait exists, TabBarNavigator gone)

**Files:**
- Modify: `src/workspace.rs` — remove `navigator` field, add `impl Navigator`, replace WorkspaceEffect with NavEffect
- Modify: `src/dispatch/tabs.rs` — WorkspaceEffect → NavEffect
- Modify: `src/app_init.rs` — remove navigator init

**Produces:** Workspace implementing Navigator; WorkspaceEffect deleted; NavEffect used everywhere

- [ ] **Step 1: Remove navigator field from Workspace**

Edit `src/workspace.rs`. Remove from struct:
```rust
    pub(crate) navigator: Box<dyn crate::navigator::Navigator>,
```

Remove from `new()`:
```rust
    navigator: Box::new(crate::navigators::tab_bar::TabBarNavigator::new()),
```

Remove any `use crate::navigators::...` import.

- [ ] **Step 2: Delete WorkspaceEffect, replace with NavEffect**

Delete `WorkspaceEffect` enum and `impl WorkspaceEffect` block.

```bash
sed -i '' 's/WorkspaceEffect/NavEffect/g' src/workspace.rs
```

Manually fix variant names:
- `NavEffect::ActiveTabChanged` → `NavEffect::ActiveChanged`
- `NavEffect::LayoutChanged` → `NavEffect::ItemsChanged`

Add import at top of workspace.rs:
```rust
use crate::navigator::{NavEffect, Navigator};
```

- [ ] **Step 3: Convert close_entry_inner return type**

Read current signature — it returns `Result<WorkspaceEffect, String>`. Change to `Result<NavEffect, String>` and update variant names in its body.

- [ ] **Step 4: Merge duplicate switch_to methods**

Workspace currently has a `switch_to` returning `NavEffect`. The Navigator trait also defines `switch_to`. Two options:

**Option A (simpler):** Keep the existing `switch_to` method on `impl Workspace`, and have the Navigator impl delegate to it. Rename the internal one if there's a naming conflict.

**Option B:** Move the entire switch_to logic into the Navigator impl.

Choose A — no code duplication:
```rust
// Existing method renamed to avoid conflict:
impl Workspace {
    pub(crate) fn switch_to(&mut self, index: usize) -> NavEffect { ... }
}

impl Navigator for Workspace {
    fn switch_to(&mut self, index: usize) -> NavEffect {
        Workspace::switch_to(self, index)
    }
    // ...
}
```

Actually, Rust allows methods with same name on inherent impl and trait impl. Just call `self.switch_to(index)` — it disambiguates automatically. But clearer to have only one implementation. Move the logic into the trait impl and have the inherent method forward to it, or vice versa.

Simplest: the existing Workspace methods keep their signatures but return `NavEffect`. The Navigator impl calls them directly:

```rust
impl Navigator for Workspace {
    fn switch_to(&mut self, idx: usize) -> NavEffect { self.switch_to(idx) }
    fn close(&mut self, idx: usize) -> NavEffect { self.close_entry(idx).unwrap_or(NavEffect::None) }
    fn toggle_pin(&mut self, idx: usize) -> NavEffect { self.toggle_pin_at(idx) }
    fn is_pinned(&self, idx: usize) -> bool { self.is_pinned(idx) }
    fn pinned_indices(&self) -> &HashSet<usize> { self.pinned_indices() }
}
```

This works because the Navigator trait methods have distinct enough semantics.

- [ ] **Step 5: Add Navigator impl block**

After `impl Workspace { ... }`, before `#[cfg(test)]`:

```rust
impl crate::navigator::Navigator for Workspace {
    fn id(&self) -> &str { "builtin.files" }
    fn name(&self) -> &str { "Open Files" }

    fn items(&self) -> Vec<crate::navigator::NavEntry> {
        self.entries.iter().enumerate().map(|(i, e)| {
            crate::navigator::NavEntry {
                title: e.doc_title(),
                file_path: e.doc.file_path.clone(),
                is_dirty: e.doc.dirty,
                pinned: self.pinned_indices.contains(&i),
            }
        }).collect()
    }

    fn active_index(&self) -> usize { self.active_index }

    fn switch_to(&mut self, index: usize) -> NavEffect { self.switch_to(index) }
    fn close(&mut self, index: usize) -> NavEffect {
        self.close_entry(index).unwrap_or(NavEffect::None)
    }
    fn toggle_pin(&mut self, index: usize) -> NavEffect { self.toggle_pin_at(index) }
    fn is_pinned(&self, index: usize) -> bool { self.is_pinned(index) }
    fn pinned_indices(&self) -> &HashSet<usize> { self.pinned_indices() }

    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
```

- [ ] **Step 6: Fix dispatch/tabs.rs**

```bash
sed -i '' 's/WorkspaceEffect/NavEffect/g' src/dispatch/tabs.rs
```

Fix variant names: `ActiveTabChanged` → `ActiveChanged`, `LayoutChanged` → `ItemsChanged`.
Add `use crate::navigator::NavEffect;` import.

- [ ] **Step 7: Verify everything compiles**

Run: `cargo check 2>&1`
Expected: passes.

- [ ] **Step 8: Remove tab_bar_height import from workspace.rs**

The `tab_bar_height` function was imported for the old `thickness()` method. Remove:
```bash
grep -n "tab_bar_height\|tab_bar" src/workspace.rs
```
If still present as an unused import, remove it.

- [ ] **Step 9: Commit**

```bash
git add src/workspace.rs src/dispatch/tabs.rs src/app_init.rs
git commit -m "refactor: Workspace implements Navigator, WorkspaceEffect→NavEffect"
```

---

### Task 6: Verify — cargo check + cargo test

**Deps:** Task 5 (all changes complete)

**Files:** none (verification only)

- [ ] **Step 1: Compilation check**

```bash
cargo check 2>&1
```
Expected: `Finished` with 0 errors, 0 warnings (or only pre-existing warnings).

- [ ] **Step 2: App crate tests**

```bash
cargo test --lib 2>&1 | tail -5
```
Expected: `821 passed; 0 failed; 2 ignored`

- [ ] **Step 3: UI crate tests**

```bash
cargo test -p edit-plus-ui --lib 2>&1 | tail -5
```
Expected: all pass.

- [ ] **Step 4: Full workspace test**

```bash
cargo test 2>&1 | tail -10
```
Expected: all tests pass across all crates.

- [ ] **Step 5: Check for remaining old names**

```bash
grep -rn "TabBarNavigator\|WorkspaceEffect\|\.navigator\." --include="*.rs" src/
```
Expected: no results (or only in comments).

- [ ] **Step 6: Review git diff**

```bash
git diff --stat HEAD~5..HEAD
```
Verify the scope matches expectations.

- [ ] **Step 7: Commit final verification**

```bash
git add -A
git commit -m "chore: verify all tests pass after Navigator v2 refactor

821 tests pass. Navigator trait now pure data interface.
Scroll/animation in TabBarWidget + App::SmoothScroll.
TabBarNavigator deleted. Tab→DocItem. WorkspaceEffect→NavEffect."
```
